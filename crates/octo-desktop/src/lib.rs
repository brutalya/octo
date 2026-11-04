//! Octo desktop: embeds the axum server on a random loopback port and loads it in a webview.

use anyhow::Result;
use octo_core::{Config, Hub};
use std::net::SocketAddr;
use std::path::PathBuf;
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
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
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
        let _ = self.task.await;
    }
}

/// Binds `127.0.0.1:0` and serves the API (token required) plus the built web UI.
pub async fn start_server(config: Config, web_dir: Option<PathBuf>) -> Result<Server> {
    let token = new_token();
    let hub = Arc::new(Hub::new(config)?);
    let app = octo_server::app_with_token(hub.clone(), web_dir, Some(token.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let _ = axum_serve(listener, app, rx).await;
    });
    Ok(Server {
        addr,
        token,
        hub,
        stop: Some(stop),
        task,
    })
}

async fn axum_serve(
    listener: tokio::net::TcpListener,
    app: octo_server::Router,
    stop: oneshot::Receiver<()>,
) -> std::io::Result<()> {
    octo_server::serve(listener, app, async {
        let _ = stop.await;
    })
    .await
}

/// Script injected into every page before it runs.
pub fn init_script(token: &str) -> String {
    format!("window.__OCTO_TOKEN__ = \"{token}\";")
}
