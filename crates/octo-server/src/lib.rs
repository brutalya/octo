use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use octo_core::provider::ProviderKind;
use octo_core::storage::Space;
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

struct ApiError(CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            CoreError::NotFound => StatusCode::NOT_FOUND,
            CoreError::Busy => StatusCode::CONFLICT,
            CoreError::Invalid(_) => StatusCode::BAD_REQUEST,
            CoreError::Policy(_) => StatusCode::FORBIDDEN,
            CoreError::Storage(_) | CoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error": self.0.to_string()}))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Deserialize)]
struct NewThread {
    provider: ProviderKind,
    space: Space,
}

#[derive(Deserialize)]
struct NewMessage {
    content: String,
}

pub fn app(hub: Shared, web_dir: Option<PathBuf>) -> Router {
    let api = Router::new()
        .route("/providers", get(providers))
        .route("/threads", get(list_threads).post(create_thread))
        .route("/threads/{id}", axum::routing::delete(delete_thread))
        .route(
            "/threads/{id}/messages",
            get(list_messages).post(send_message),
        )
        .route("/threads/{id}/cancel", post(cancel))
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
    Ok(Json(h.create_thread(b.provider, b.space)?))
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
    let rx = h.send_message(&id, &b.content).await?;
    let stream = ReceiverStream::new(rx).map(|ev| {
        let data = serde_json::to_string(&ev).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event(ev.name()).data(data))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
