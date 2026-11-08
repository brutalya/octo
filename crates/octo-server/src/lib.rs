pub mod node;
pub mod policy;
mod tunnel;

use axum::Json;
pub use axum::Router;
use axum::extract::multipart::MultipartError;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use node::{Node, NodeError};
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

/// Serves `app` on `listener` until `shutdown` resolves, then drains connections.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

/// Router for the separate relay listener: only `/relay` (WebSocket) and `/healthz`.
pub fn relay_app(relay: octo_net::relay::Relay) -> Router {
    octo_net::relay::router(relay).route("/healthz", get(|| async { "ok" }))
}

/// Refuses a non-loopback API address without a bearer token.
pub fn check_exposure(addr: std::net::SocketAddr, token: Option<&str>) -> Result<(), String> {
    if addr.ip().is_loopback() || token.is_some() {
        return Ok(());
    }
    Err(format!(
        "refusing to serve the API on non-loopback address {addr} without --token: \
         the API would be open to the network. Pass --token <secret> or bind to 127.0.0.1"
    ))
}

/// Reads the API token from `path`, creating a random 0600 one when missing.
pub fn load_or_create_token(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Write;
    match std::fs::read_to_string(path) {
        Ok(s) => {
            let t = s.trim().to_string();
            if t.is_empty() {
                return Err(std::io::Error::other("token file is empty"));
            }
            return Ok(t);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut raw = [0u8; 48];
    rand::fill(&mut raw[..]);
    let token: String = raw
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)?.write_all(token.as_bytes())?;
    Ok(token)
}

/// Minimal HTTP GET /healthz over plain TCP (no curl in the slim image).
pub fn healthcheck(addr: &str) -> std::io::Result<()> {
    use std::io::{Read, Write};
    use std::net::ToSocketAddrs;
    use std::time::Duration;
    let sa = addr
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::other("bad address"))?;
    let t = Duration::from_secs(3);
    let mut s = std::net::TcpStream::connect_timeout(&sa, t)?;
    s.set_read_timeout(Some(t))?;
    s.write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    let mut buf = String::new();
    s.read_to_string(&mut buf)?;
    if buf.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(std::io::Error::other("healthz: not 200"))
    }
}

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
            | CoreError::Settings(octo_core::settings::SettingsError::BadBudget)
            | CoreError::Settings(octo_core::settings::SettingsError::BadRelayUrl) => {
                StatusCode::BAD_REQUEST
            }
            CoreError::Files(e) => match e {
                FilesError::NotFound => StatusCode::NOT_FOUND,
                FilesError::Exists | FilesError::NotEmpty => StatusCode::CONFLICT,
                FilesError::Path(_)
                | FilesError::IsDir
                | FilesError::NotDir
                | FilesError::HashMismatch
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
    cwd: Option<String>,
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
struct SessionsQuery {
    #[serde(default)]
    q: String,
}

#[derive(Deserialize)]
struct ImportBody {
    external_id: String,
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
    app_with_token(hub, web_dir, None)
}

/// Same as [`app`]; when `token` is set, `/api/*` requires `Authorization: Bearer <token>`.
pub fn app_with_token(hub: Shared, web_dir: Option<PathBuf>, token: Option<String>) -> Router {
    build(hub, None, web_dir, token)
}

/// Same as [`app_with_token`], plus the node endpoints (`/api/nodes`, `/api/pairing/*`).
/// Requests tunneled from member nodes are dispatched into the local API only (no token, no
/// node/pairing endpoints, so they cannot hop further).
pub fn app_with_node(
    hub: Shared,
    node: Arc<Node>,
    web_dir: Option<PathBuf>,
    token: Option<String>,
) -> Router {
    build(hub, Some(node), web_dir, token)
}

fn build(
    hub: Shared,
    node: Option<Arc<Node>>,
    web_dir: Option<PathBuf>,
    token: Option<String>,
) -> Router {
    let local = local_api(hub);
    let api = match node {
        Some(n) => {
            // Weak: the router is stored inside the node, a strong reference would be a cycle.
            let notify = Arc::downgrade(&n);
            let local = local.layer(middleware::from_fn(move |req: Request, next: Next| {
                let notify = notify.clone();
                async move {
                    let changes = req.method() != axum::http::Method::GET
                        && req
                            .uri()
                            .path()
                            .trim_start_matches("/api")
                            .starts_with("/files/private");
                    let resp = next.run(req).await;
                    if changes
                        && resp.status().is_success()
                        && let Some(n) = notify.upgrade()
                    {
                        n.private_changed();
                    }
                    resp
                }
            }));
            n.set_router(Router::new().nest("/api", local.clone()));
            local.merge(node_routes(n))
        }
        None => local,
    };
    let api = match token {
        Some(t) => api.layer(middleware::from_fn_with_state(
            Arc::<str>::from(t),
            require_token,
        )),
        None => api,
    };
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest("/api", api);
    match web_dir {
        Some(dir) => {
            let index = ServeFile::new(dir.join("index.html"));
            router.fallback_service(ServeDir::new(dir).fallback(index))
        }
        None => router,
    }
}

fn local_api(hub: Shared) -> Router {
    Router::new()
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
        .route("/import/{kind}/sessions", get(import_sessions))
        .route("/import/{kind}", post(import_session))
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
        .route("/sync/private/manifest", get(sync_manifest))
        .route("/sync/private/blob/{hash}", get(sync_blob))
        .with_state(hub)
}

async fn require_token(State(token): State<Arc<str>>, req: Request, next: Next) -> Response {
    let ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|given| ct_eq(given.as_bytes(), token.as_bytes()));
    if ok {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response()
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
    Ok(Json(h.create_thread_in(
        b.provider,
        b.space,
        b.approval,
        b.model.as_deref(),
        b.cwd.as_deref(),
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
    // may re-read the CLI's session file
    Ok(Json(blocking_core(move || h.list_messages(&id)).await?))
}

async fn import_sessions(
    State(h): State<Shared>,
    Path(kind): Path<ProviderKind>,
    Query(q): Query<SessionsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        blocking_core(move || h.import_sessions(kind, &q.q)).await?,
    ))
}

async fn import_session(
    State(h): State<Shared>,
    Path(kind): Path<ProviderKind>,
    Json(b): Json<ImportBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        blocking_core(move || h.import_session(kind, &b.external_id)).await?,
    ))
}

async fn blocking_core<T, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ApiError::Core(CoreError::Invalid("task failed")))?
        .map_err(ApiError::from)
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

async fn sync_manifest(State(h): State<Shared>) -> ApiResult<impl IntoResponse> {
    Ok(Json(blocking(&h, |f| f.private_manifest()).await?))
}

async fn sync_blob(State(h): State<Shared>, Path(hash): Path<String>) -> ApiResult<Response> {
    let valid = hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !valid {
        return Err(CoreError::Invalid("invalid hash").into());
    }
    let data = blocking(&h, move |f| f.private_blob(&hash))
        .await?
        .ok_or(FilesError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
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

impl IntoResponse for NodeError {
    fn into_response(self) -> Response {
        let status = match &self {
            NodeError::NoRelay | NodeError::BadCode | NodeError::BadPath => StatusCode::BAD_REQUEST,
            NodeError::Busy => StatusCode::CONFLICT,
            NodeError::NoPending | NodeError::UnknownNode => StatusCode::NOT_FOUND,
            NodeError::Unreachable | NodeError::Net(_) => StatusCode::BAD_GATEWAY,
        };
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

fn node_routes(node: Arc<Node>) -> Router {
    Router::new()
        .route("/nodes", get(list_nodes))
        .route("/nodes/{id}", axum::routing::delete(remove_node))
        .route("/nodes/{id}/trust", axum::routing::patch(set_node_trust))
        .route("/nodes/{id}/{*rest}", any(forward_to_node))
        .route("/pairing/invite", post(pairing_invite))
        .route("/pairing/join", post(pairing_join))
        .route("/pairing/pending", get(pairing_pending))
        .route("/pairing/confirm", post(pairing_confirm))
        .route("/pairing/reject", post(pairing_reject))
        .with_state(node)
}

async fn list_nodes(State(n): State<Arc<Node>>) -> impl IntoResponse {
    Json(n.list().await)
}

async fn forward_to_node(
    State(n): State<Arc<Node>>,
    Path((id, rest)): Path<(String, String)>,
    req: Request,
) -> Result<Response, NodeError> {
    Ok(n.forward(&id, &rest, req).await?.into_response())
}

async fn pairing_invite(State(n): State<Arc<Node>>) -> Result<impl IntoResponse, NodeError> {
    Ok(Json(n.invite().await?))
}

#[derive(Deserialize)]
struct JoinBody {
    code: String,
    #[serde(default)]
    name: String,
}

async fn pairing_join(
    State(n): State<Arc<Node>>,
    Json(b): Json<JoinBody>,
) -> Result<StatusCode, NodeError> {
    n.join(&b.code, &b.name)?;
    Ok(StatusCode::ACCEPTED)
}

async fn pairing_pending(State(n): State<Arc<Node>>) -> Result<impl IntoResponse, NodeError> {
    n.pending().map(Json).ok_or(NodeError::NoPending)
}

#[derive(Deserialize, Default)]
struct ConfirmBody {
    #[serde(default)]
    trust_private: bool,
}

async fn pairing_confirm(State(n): State<Arc<Node>>, body: axum::body::Bytes) -> Response {
    let b: ConfirmBody = if body.is_empty() {
        ConfirmBody::default()
    } else if let Ok(b) = serde_json::from_slice(&body) {
        b
    } else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match n.decide(true, b.trust_private) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

#[derive(Deserialize)]
struct TrustBody {
    trust: octo_net::membership::Trust,
}

async fn set_node_trust(
    State(n): State<Arc<Node>>,
    Path(id): Path<String>,
    Json(b): Json<TrustBody>,
) -> Result<StatusCode, NodeError> {
    n.set_trust(&id, b.trust).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_node(
    State(n): State<Arc<Node>>,
    Path(id): Path<String>,
) -> Result<StatusCode, NodeError> {
    n.remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pairing_reject(State(n): State<Arc<Node>>) -> Result<StatusCode, NodeError> {
    n.decide(false, false)?;
    Ok(StatusCode::NO_CONTENT)
}
