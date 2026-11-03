use axum::extract::multipart::MultipartError;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use octo_core::files::{Files, FilesError};
use octo_core::providers::ProviderKind;
use octo_core::settings::SettingsPatch;
use octo_core::storage::{Approval, AttachmentRef, Space};
use octo_core::{CoreError, Hub};
use serde::Deserialize;
use serde_json::json;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::services::{ServeDir, ServeFile};

type Shared = Arc<Hub>;

enum ApiError {
    Core(CoreError),
    Upload(MultipartError),
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        Self::Core(e)
    }
}

impl From<MultipartError> for ApiError {
    fn from(e: MultipartError) -> Self {
        Self::Upload(e)
    }
}

impl From<FilesError> for ApiError {
    fn from(e: FilesError) -> Self {
        Self::Core(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let err = match self {
            ApiError::Core(e) => e,
            ApiError::Upload(e) => {
                let body = Json(json!({"error": e.body_text()}));
                return (e.status(), body).into_response();
            }
        };
        let status = match &err {
            CoreError::NotFound => StatusCode::NOT_FOUND,
            CoreError::Busy => StatusCode::CONFLICT,
            CoreError::Invalid(_) => StatusCode::BAD_REQUEST,
            CoreError::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            CoreError::Upstream(_) => StatusCode::BAD_GATEWAY,
            CoreError::Policy(_) => StatusCode::FORBIDDEN,
            CoreError::Settings(octo_core::settings::SettingsError::BadUrl)
            | CoreError::Settings(octo_core::settings::SettingsError::BadBudget) => {
                StatusCode::BAD_REQUEST
            }
            CoreError::Files(e) => match e {
                FilesError::NotFound => StatusCode::NOT_FOUND,
                FilesError::Exists | FilesError::NotEmpty => StatusCode::CONFLICT,
                FilesError::Path(_)
                | FilesError::IsDir
                | FilesError::NotDir
                | FilesError::Invalid => StatusCode::BAD_REQUEST,
                FilesError::Crypto(_) | FilesError::Corrupt | FilesError::Io(_) => {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            },
            CoreError::Settings(_) | CoreError::Storage(_) | CoreError::Io(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        (status, Json(json!({"error": err.to_string()}))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Deserialize)]
struct NewThread {
    provider: ProviderKind,
    space: Space,
    #[serde(default)]
    approval: Approval,
    model: Option<String>,
}

#[derive(Deserialize)]
struct PatchThread {
    approval: Approval,
}

#[derive(Deserialize)]
struct NewMessage {
    content: String,
    #[serde(default)]
    attachments: Vec<AttachmentRef>,
}

#[derive(Deserialize)]
struct PathQuery {
    #[serde(default)]
    path: String,
}

#[derive(Deserialize)]
struct RenameBody {
    new_path: String,
}

/// Uploads are limited to 50 MB (plus multipart framing).
const UPLOAD_LIMIT: usize = 50 * 1024 * 1024 + 64 * 1024;

pub fn app(hub: Shared, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/providers", get(providers))
        .route("/threads", get(list_threads).post(create_thread))
        .route(
            "/threads/{id}",
            axum::routing::delete(delete_thread).patch(patch_thread),
        )
        .route(
            "/threads/{id}/messages",
            get(list_messages).post(send_message),
        )
        .route("/threads/{id}/cancel", post(cancel))
        .route("/providers/{kind}/models", get(models))
        .route("/settings", get(get_settings).patch(patch_settings))
        .route(
            "/files/{space}",
            get(list_files)
                .post(upload_files)
                .patch(rename_file)
                .delete(delete_file)
                .layer(DefaultBodyLimit::max(UPLOAD_LIMIT)),
        )
        .route("/files/{space}/content", get(file_content))
        .route("/files/{space}/dir", post(make_dir))
        .with_state(hub);
    let router = Router::new().nest("/api", api);
    match web_dir {
        Some(dir) => {
            let index = ServeFile::new(dir.join("index.html"));
            router.fallback_service(ServeDir::new(dir).fallback(index))
        }
        None => router,
    }
}

async fn providers(State(h): State<Shared>) -> impl IntoResponse {
    Json(h.providers().await)
}

async fn list_threads(State(h): State<Shared>) -> ApiResult<impl IntoResponse> {
    Ok(Json(h.list_threads()?))
}

async fn create_thread(
    State(h): State<Shared>,
    Json(b): Json<NewThread>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(h.create_thread(
        b.provider,
        b.space,
        b.approval,
        b.model.as_deref(),
    )?))
}

async fn patch_thread(
    State(h): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<PatchThread>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(h.set_approval(&id, b.approval)?))
}

async fn delete_thread(State(h): State<Shared>, Path(id): Path<String>) -> ApiResult<StatusCode> {
    h.delete_thread(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_messages(
    State(h): State<Shared>,
    Path(id): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(h.list_messages(&id)?))
}

async fn cancel(State(h): State<Shared>, Path(id): Path<String>) -> StatusCode {
    h.cancel(&id);
    StatusCode::NO_CONTENT
}

async fn send_message(
    State(h): State<Shared>,
    Path(id): Path<String>,
    Json(b): Json<NewMessage>,
) -> ApiResult<impl IntoResponse> {
    let rx = h.send_message(&id, &b.content, &b.attachments).await?;
    let stream = ReceiverStream::new(rx).map(|ev| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event(ev.name()).data(data))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

async fn models(
    State(h): State<Shared>,
    Path(kind): Path<ProviderKind>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(h.models(kind).await?))
}

async fn get_settings(State(h): State<Shared>) -> impl IntoResponse {
    Json(h.settings())
}

async fn patch_settings(
    State(h): State<Shared>,
    Json(b): Json<SettingsPatch>,
) -> ApiResult<impl IntoResponse> {
    let view = tokio::task::spawn_blocking(move || h.update_settings(b))
        .await
        .map_err(|_| CoreError::Invalid("settings task failed"))??;
    Ok(Json(view))
}

/// Runs file I/O (sync fs + crypto) off the async runtime.
async fn blocking<T, F>(h: &Shared, f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&Files) -> Result<T, FilesError> + Send + 'static,
{
    let files = h.files().clone();
    tokio::task::spawn_blocking(move || f(&files))
        .await
        .map_err(|_| ApiError::Core(CoreError::Invalid("file task failed")))?
        .map_err(ApiError::from)
}

async fn list_files(
    State(h): State<Shared>,
    Path(space): Path<Space>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(blocking(&h, move |f| f.list(space)).await?))
}

async fn upload_files(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
    mut mp: Multipart,
) -> ApiResult<impl IntoResponse> {
    let mut saved = vec![];
    while let Some(field) = mp.next_field().await? {
        let Some(name) = field.file_name().map(str::to_string) else {
            continue;
        };
        let data = field.bytes().await?;
        let target = if q.path.is_empty() {
            name
        } else {
            format!("{}/{name}", q.path)
        };
        saved.push(blocking(&h, move |f| f.write(space, &target, &data)).await?);
    }
    if saved.is_empty() {
        return Err(CoreError::Invalid("no file in request").into());
    }
    Ok(Json(saved))
}

async fn file_content(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
) -> ApiResult<impl IntoResponse> {
    let data = blocking(&h, move |f| f.read(space, &q.path)).await?;
    // Never rendered by the browser: user files share the app's origin.
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_SECURITY_POLICY, "sandbox"),
        ],
        data,
    ))
}

async fn make_dir(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
) -> ApiResult<StatusCode> {
    blocking(&h, move |f| f.mkdir(space, &q.path)).await?;
    Ok(StatusCode::CREATED)
}

async fn rename_file(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
    Json(b): Json<RenameBody>,
) -> ApiResult<StatusCode> {
    blocking(&h, move |f| f.rename(space, &q.path, &b.new_path)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_file(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
) -> ApiResult<StatusCode> {
    blocking(&h, move |f| f.delete(space, &q.path)).await?;
    Ok(StatusCode::NO_CONTENT)
}
