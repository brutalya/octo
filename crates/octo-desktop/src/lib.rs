//! Octo desktop: embeds the axum server on a random loopback port and loads it in a webview.

mod app;
pub use app::run;

use anyhow::Result;
use octo_core::{Config, Hub};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// Per-launch secret: 32 random bytes, hex-encoded.
pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes[..]);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A running embedded server. Call [`Server::shutdown`] to stop it and kill child processes.
pub struct Server {
    pub addr: SocketAddr,
    pub token: String,
    hub: Arc<Hub>,
    /// Keeps the node service (relay link, pairing, sync, tunnel) alive.
    _node: Arc<octo_server::node::Node>,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    /// Cancels running agents, then waits for axum's graceful shutdown.
    pub async fn shutdown(mut self) {
        self.hub.shutdown();
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        match self.task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => eprintln!("octo: server stopped with an error: {e}"),
            Err(e) => eprintln!("octo: server task failed: {e}"),
        }
    }
}

/// True for pages of the embedded server (`http://127.0.0.1:<port>`); the webview may show
/// nothing else, so a link or redirect can never turn it into a browser with the app's token.
pub fn is_app_url(url: &tauri::Url, addr: SocketAddr) -> bool {
    url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port() == Some(addr.port())
        && matches!(url.host(), Some(url::Host::Ipv4(ip)) if ip == addr.ip())
}

/// Binds `127.0.0.1:0` and serves the API (token required, node service included) plus the
/// built web UI.
pub async fn start_server(
    mut config: Config,
    web: Option<octo_server::WebAssets>,
) -> Result<Server> {
    let token = new_token();
    // Bind first: Claude's approval prompts call back to the address actually bound.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    config.local_api = Some(addr);
    let data_dir = config.data_dir.clone();
    let hub = Arc::new(Hub::new(config, &octo_server::node::node_id(&data_dir)?)?);
    let node = octo_server::node::Node::start(hub.clone(), &data_dir, Default::default())?;
    let app = octo_server::app(
        hub.clone(),
        octo_server::AppConfig {
            web,
            token: Some(token.clone()),
            node: Some(node.clone()),
            allowed_host: None,
        },
    );
    let (stop, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(octo_server::serve(listener, app, async {
        let _ = rx.await;
    }));
    Ok(Server {
        addr,
        token,
        hub,
        _node: node,
        stop: Some(stop),
        task,
    })
}

/// Script injected into every page before it runs. The token is only set on the app's own
/// origin (e.g. `http://127.0.0.1:4321`), never on a page that got in by other means.
pub fn init_script(token: &str, origin: &str) -> String {
    format!("if (location.origin === \"{origin}\") {{ window.__OCTO_TOKEN__ = \"{token}\"; }}")
}
