mod mcp;
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
use tower::ServiceExt;
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

/// A usable bearer token: not empty or blank.
pub fn check_token(token: &str) -> Result<(), String> {
    if token.trim().is_empty() {
        return Err("the API token is empty; pass a non-empty --token or use --token-file".into());
    }
    Ok(())
}

/// Reads the API token from `path`, creating a random 0600 one when missing. Creation is atomic:
/// the token is written to a private temp file and linked into place, so a concurrent start
/// either sees the complete file or loses the race and reads the winner's.
pub fn load_or_create_token(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Write;
    let read = |p: &std::path::Path| -> std::io::Result<Option<String>> {
        match std::fs::read_to_string(p) {
            Ok(s) => {
                let t = s.trim().to_string();
                check_token(&t).map_err(|_| std::io::Error::other("token file is empty"))?;
                Ok(Some(t))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    };
    if let Some(t) = read(path)? {
        return Ok(t);
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
    let mut tmp_name = path.as_os_str().to_owned();
    let mut salt = [0u8; 6];
    rand::fill(&mut salt[..]);
    let salt: String = salt.iter().map(|b| format!("{b:02x}")).collect();
    tmp_name.push(format!(".{salt}.tmp"));
    let tmp = std::path::PathBuf::from(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(token.as_bytes())?;
    f.sync_all()?;
    drop(f);
    let linked = octo_core::fsutil::place_new(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(token),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            read(path)?.ok_or_else(|| std::io::Error::other("token file vanished"))
        }
        Err(e) => Err(e),
    }
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
            CoreError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
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
                FilesError::Crypto(_)
                | FilesError::Corrupt
                | FilesError::Serialize
                | FilesError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
            },
            CoreError::Settings(_)
            | CoreError::Storage(_)
            | CoreError::Internal(_)
            | CoreError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // Server-side failures can carry SQL, paths or OS errors: log them, tell the client little.
        let msg = if status.is_server_error() && status != StatusCode::BAD_GATEWAY {
            eprintln!("octo: {status}: {err}");
            "internal error".to_string()
        } else {
            err.to_string()
        };
        (status, Json(json!({"error": msg}))).into_response()
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

/// Largest accepted file upload / blob (50 MB). Defined once for the API, the tunnel and sync.
pub(crate) const MAX_UPLOAD: usize = 50 * 1024 * 1024;
/// Upload request limit: the file plus multipart framing. Also the largest tunneled body.
pub(crate) const UPLOAD_LIMIT: usize = MAX_UPLOAD + 64 * 1024;

/// Everything that varies between the server binary, the desktop app and the tests.
#[derive(Default)]
pub struct AppConfig {
    /// Web UI to serve besides the API.
    pub web: Option<WebAssets>,
    /// When set, `/api/*` requires `Authorization: Bearer <token>`.
    pub token: Option<String>,
    /// Adds the node endpoints (`/api/nodes`, `/api/pairing/*`). Requests tunneled from member
    /// nodes are dispatched into the local API only (no token, no node/pairing endpoints, so
    /// they cannot hop further).
    pub node: Option<Arc<Node>>,
    /// Host name (no port) of the configured listen address, accepted in the `Host` header
    /// besides the loopback names when no token is set.
    pub allowed_host: Option<String>,
}

/// The one router builder used by the binary, the desktop app and the tests.
pub fn app(hub: Shared, cfg: AppConfig) -> Router {
    let AppConfig {
        web,
        token,
        node,
        allowed_host,
    } = cfg;
    let mcp = Router::new()
        .route("/mcp/{token}", post(mcp::handle))
        .with_state(hub.clone());
    let local = local_api(hub);
    let api = match node {
        Some(n) => {
            // Weak: the router is stored inside the node, a strong reference would be a cycle.
            let notify = Arc::downgrade(&n);
            let local = local.layer(middleware::from_fn(move |req: Request, next: Next| {
                let notify = notify.clone();
                async move {
                    let changes = req.method() != axum::http::Method::GET
                        && touches_private_files(req.uri().path());
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
    let guard_host = token.is_none();
    let api = match token {
        Some(t) => api.layer(middleware::from_fn_with_state(
            Arc::<str>::from(t),
            require_token,
        )),
        None => api,
    };
    // The MCP route authenticates by its per-run token, not by the bearer token; it is not
    // reachable through the tunnel (only `/api` is dispatched there).
    let router = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(mcp)
        .nest("/api", api);
    let router = match web {
        Some(WebAssets::Dir(dir)) => router.fallback(get(serve_dir).with_state(Arc::new(dir))),
        Some(WebAssets::Embedded(lookup)) => router.fallback(get(move |uri: axum::http::Uri| {
            let lookup = lookup.clone();
            async move { serve_embedded(&lookup, uri.path()) }
        })),
        None => router,
    };
    if guard_host {
        // Without a token the loopback listener is the only protection; a browser tricked into
        // resolving an attacker's name to 127.0.0.1 (DNS rebinding) must not get through.
        let allowed: Arc<[String]> = ["127.0.0.1", "localhost", "[::1]"]
            .into_iter()
            .map(String::from)
            .chain(allowed_host.map(|h| h.to_ascii_lowercase()))
            .collect();
        router.layer(middleware::from_fn(move |req: Request, next: Next| {
            let allowed = allowed.clone();
            async move {
                match req.headers().get(header::HOST) {
                    None => next.run(req).await,
                    Some(h) if h.to_str().ok().is_some_and(|h| host_allowed(h, &allowed)) => {
                        next.run(req).await
                    }
                    Some(_) => (
                        StatusCode::FORBIDDEN,
                        Json(json!({"error": "host not allowed"})),
                    )
                        .into_response(),
                }
            }
        }))
    } else {
        router
    }
}

/// Host header value (optionally with port) against the allowed host names.
fn host_allowed(value: &str, allowed: &[String]) -> bool {
    let value = value.trim().to_ascii_lowercase();
    let host = if value.starts_with('[') {
        value.split_once(']').map(|(h, _)| format!("{h}]"))
    } else {
        Some(
            value
                .rsplit_once(':')
                .map_or(value.as_str(), |(h, _)| h)
                .to_string(),
        )
    };
    host.is_some_and(|h| allowed.contains(&h))
}

/// True if `path` (with or without the `/api` prefix) is under `/files/private`.
fn touches_private_files(path: &str) -> bool {
    let mut seg = path.trim_start_matches("/api").split('/').skip(1);
    seg.next() == Some("files") && seg.next() == Some("private")
}

/// Where the web UI comes from.
#[derive(Clone)]
pub enum WebAssets {
    /// A built `web/dist` directory on disk.
    Dir(PathBuf),
    /// Assets embedded in the binary: path (no leading slash) to bytes and mime type.
    Embedded(EmbeddedLookup),
}

pub type EmbeddedLookup = Arc<dyn Fn(&str) -> Option<(Vec<u8>, String)> + Send + Sync>;

/// Headers for everything the web UI serves: no inline script, no foreign origins.
const WEB_CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data: blob:; \
style-src 'self' 'unsafe-inline'; object-src 'none'; frame-ancestors 'none'";

fn web_headers(mut resp: Response) -> Response {
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static(WEB_CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    resp
}

/// True if the last path segment has an extension (a file request, not a client route).
fn has_extension(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|s| s.contains('.'))
}

/// Exact asset; unknown path with an extension is 404, without one it is a client route and
/// gets `index.html` (SPA fallback).
fn serve_embedded(lookup: &EmbeddedLookup, path: &str) -> Response {
    let path = path.trim_start_matches('/');
    let found = if path.is_empty() { None } else { lookup(path) };
    let found = match found {
        Some(f) => Some(f),
        None if has_extension(path) => None,
        None => lookup("index.html"),
    };
    web_headers(match found {
        Some((bytes, mime)) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}

/// `web/dist` from disk with the same fallback rules as [`serve_embedded`].
async fn serve_dir(State(dir): State<Arc<PathBuf>>, req: Request) -> Response {
    let (method, uri) = (req.method().clone(), req.uri().clone());
    drop(req);
    let path = uri.path().to_string();
    let again = |uri: axum::http::Uri| {
        Request::builder()
            .method(method.clone())
            .uri(uri)
            .body(axum::body::Body::empty())
    };
    let Ok(first) = again(uri) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let resp = match ServeDir::new(&*dir).oneshot(first).await {
        Ok(r) => r.map(axum::body::Body::new),
        Err(never) => match never {},
    };
    if resp.status() != StatusCode::NOT_FOUND || has_extension(&path) {
        return web_headers(resp);
    }
    let Ok(index) = again(axum::http::Uri::from_static("/")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    web_headers(
        match ServeFile::new(dir.join("index.html")).oneshot(index).await {
            Ok(r) => r.map(axum::body::Body::new),
            Err(never) => match never {},
        },
    )
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
        .route("/threads/{id}/approvals", get(list_approvals))
        .route("/threads/{id}/approvals/{aid}", post(decide_approval))
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

async fn list_approvals(
    State(hub): State<Shared>,
    Path(id): Path<String>,
) -> Json<Vec<octo_core::approvals::PendingApproval>> {
    Json(hub.pending_approvals(&id))
}

#[derive(Deserialize)]
struct Decision {
    allow: bool,
}

async fn decide_approval(
    State(hub): State<Shared>,
    Path((id, aid)): Path<(String, String)>,
    Json(d): Json<Decision>,
) -> ApiResult<StatusCode> {
    hub.resolve_approval(&id, &aid, d.allow)?;
    Ok(StatusCode::NO_CONTENT)
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
    Ok(Json(blocking(move || h.list_messages(&id)).await?))
}

async fn import_sessions(
    State(h): State<Shared>,
    Path(kind): Path<ProviderKind>,
    Query(q): Query<SessionsQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(blocking(move || h.import_sessions(kind, &q.q)).await?))
}

async fn import_session(
    State(h): State<Shared>,
    Path(kind): Path<ProviderKind>,
    Json(b): Json<ImportBody>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(
        blocking(move || h.import_session(kind, &b.external_id)).await?,
    ))
}

/// Runs blocking work (sync fs, SQLite, crypto) off the async runtime; a panicked or
/// cancelled task is a 500.
async fn blocking<T, E, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    E: Send + 'static,
    ApiError: From<E>,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| {
            eprintln!("octo: blocking task failed: {e}");
            ApiError::Core(CoreError::Internal("task failed"))
        })?
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
    Ok(Json(blocking(move || h.update_settings(b)).await?))
}

/// Runs file I/O (sync fs + crypto) off the async runtime.
async fn files_blocking<T, F>(h: &Shared, f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&Files) -> Result<T, FilesError> + Send + 'static,
{
    let files = h.files().clone();
    blocking(move || f(&files)).await
}

async fn list_files(
    State(h): State<Shared>,
    Path(space): Path<Space>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(files_blocking(&h, move |f| f.list(space)).await?))
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
        saved.push(files_blocking(&h, move |f| f.write(space, &target, &data)).await?);
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
    let data = files_blocking(&h, move |f| f.read(space, &q.path)).await?;
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
    Ok(Json(files_blocking(&h, |f| f.private_manifest()).await?))
}

async fn sync_blob(State(h): State<Shared>, Path(hash): Path<String>) -> ApiResult<Response> {
    let valid = hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if !valid {
        return Err(CoreError::Invalid("invalid hash").into());
    }
    let data = files_blocking(&h, move |f| f.private_blob(&hash))
        .await?
        .ok_or(FilesError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data).into_response())
}

async fn make_dir(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
) -> ApiResult<StatusCode> {
    files_blocking(&h, move |f| f.mkdir(space, &q.path)).await?;
    Ok(StatusCode::CREATED)
}

async fn rename_file(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
    Json(b): Json<RenameBody>,
) -> ApiResult<StatusCode> {
    files_blocking(&h, move |f| f.rename(space, &q.path, &b.new_path)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_file(
    State(h): State<Shared>,
    Path(space): Path<Space>,
    Query(q): Query<PathQuery>,
) -> ApiResult<StatusCode> {
    files_blocking(&h, move |f| f.delete(space, &q.path)).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl IntoResponse for NodeError {
    fn into_response(self) -> Response {
        let status = match &self {
            NodeError::NoRelay | NodeError::BadCode | NodeError::BadPath | NodeError::BadRelay => {
                StatusCode::BAD_REQUEST
            }
            NodeError::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            NodeError::Busy | NodeError::InNetwork => StatusCode::CONFLICT,
            NodeError::NoPending | NodeError::UnknownNode => StatusCode::NOT_FOUND,
            NodeError::Unreachable | NodeError::Net(_) => StatusCode::BAD_GATEWAY,
        };
        let msg = match &self {
            NodeError::Net(e) => {
                eprintln!("octo: node error: {e}");
                "network error".to_string()
            }
            _ => self.to_string(),
        };
        (status, Json(json!({"error": msg}))).into_response()
    }
}

fn node_routes(node: Arc<Node>) -> Router {
    Router::new()
        .route("/nodes", get(list_nodes))
        .route("/nodes/{id}", axum::routing::delete(remove_node))
        .route("/nodes/{id}/trust", axum::routing::patch(set_node_trust))
        .route("/nodes/{id}/agents", axum::routing::patch(set_node_agents))
        .route("/nodes/{id}/{*rest}", any(forward_to_node))
        .route("/relay", get(relay_status))
        .route("/relay/check", post(relay_check))
        .route("/pairing/invite", post(pairing_invite))
        .route("/pairing/join", post(pairing_join))
        .route("/pairing/pending", get(pairing_pending))
        .route("/pairing/confirm", post(pairing_confirm))
        .route("/pairing/reject", post(pairing_reject))
        .route("/pairing/cancel", post(pairing_cancel))
        .route("/pairing", get(pairing_state))
        .route("/network", get(network_view))
        .route("/network/create", post(network_create))
        .route("/network/leave", post(network_leave))
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

async fn relay_status(State(n): State<Arc<Node>>) -> impl IntoResponse {
    Json(n.relay_status())
}

#[derive(Deserialize, Default)]
struct RelayCheckBody {
    relay_url: Option<String>,
}

async fn relay_check(State(n): State<Arc<Node>>, body: axum::body::Bytes) -> Response {
    let b: RelayCheckBody = if body.is_empty() {
        RelayCheckBody::default()
    } else if let Ok(b) = serde_json::from_slice(&body) {
        b
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid request body"})),
        )
            .into_response();
    };
    let url = b.relay_url.map(|u| u.trim().to_string());
    if url
        .as_deref()
        .is_some_and(|u| !(u.starts_with("ws://") || u.starts_with("wss://")))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "relay_url must start with ws:// or wss://"})),
        )
            .into_response();
    }
    match n.relay_check(url).await {
        Ok(c) => Json(c).into_response(),
        Err(e) => e.into_response(),
    }
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
    #[serde(default)]
    allow_agents: bool,
}

async fn pairing_confirm(State(n): State<Arc<Node>>, body: axum::body::Bytes) -> Response {
    let b: ConfirmBody = if body.is_empty() {
        ConfirmBody::default()
    } else if let Ok(b) = serde_json::from_slice(&body) {
        b
    } else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid request body"})),
        )
            .into_response();
    };
    match n.decide(true, b.trust_private, b.allow_agents) {
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

#[derive(Deserialize)]
struct AgentsBody {
    agents: bool,
}

async fn set_node_agents(
    State(n): State<Arc<Node>>,
    Path(id): Path<String>,
    Json(b): Json<AgentsBody>,
) -> Result<StatusCode, NodeError> {
    n.set_agents(&id, b.agents)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_node(
    State(n): State<Arc<Node>>,
    Path(id): Path<String>,
) -> Result<StatusCode, NodeError> {
    n.remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pairing_state(State(n): State<Arc<Node>>) -> impl IntoResponse {
    Json(n.pairing_state())
}

async fn network_view(State(n): State<Arc<Node>>) -> impl IntoResponse {
    Json(n.network())
}

#[derive(Deserialize)]
struct CreateNetworkBody {
    relay_url: String,
    #[serde(default)]
    name: String,
}

async fn network_create(
    State(n): State<Arc<Node>>,
    Json(b): Json<CreateNetworkBody>,
) -> Result<impl IntoResponse, NodeError> {
    Ok(Json(n.create_network(&b.relay_url, &b.name).await?))
}

async fn network_leave(State(n): State<Arc<Node>>) -> Result<StatusCode, NodeError> {
    n.leave().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pairing_cancel(State(n): State<Arc<Node>>) -> StatusCode {
    n.cancel_pairing();
    StatusCode::NO_CONTENT
}

async fn pairing_reject(State(n): State<Arc<Node>>) -> Result<StatusCode, NodeError> {
    n.decide(false, false, false)?;
    Ok(StatusCode::NO_CONTENT)
}
