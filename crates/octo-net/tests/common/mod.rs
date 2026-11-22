#![allow(dead_code)]
use octo_net::identity::NodeKey;
use octo_net::membership::{Member, Membership, NetworkKey};
use octo_net::relay::{Relay, RelayLimits, router};
use octo_net::relay_client::{RelayClientConfig, RelayHandle};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::task::JoinHandle;

pub async fn spawn_relay(limits: RelayLimits) -> (SocketAddr, JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    serve_on(l, limits)
}

pub fn serve_on(l: tokio::net::TcpListener, limits: RelayLimits) -> (SocketAddr, JoinHandle<()>) {
    let addr = l.local_addr().unwrap();
    let app = router(Relay::new(limits).unwrap());
    let h = tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    (addr, h)
}

/// Relay that admits only clients presenting `secret`.
pub async fn spawn_private_relay(secret: &str) -> (SocketAddr, JoinHandle<()>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let relay = Relay::with_secret(RelayLimits::default(), Some(secret.to_owned())).unwrap();
    let app = router(relay);
    let h = tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    (addr, h)
}

pub fn url(addr: SocketAddr) -> String {
    format!("ws://{addr}/relay")
}

/// Two memberships of one network, each listing the other node.
pub fn two_nodes() -> ((NodeKey, Membership), (NodeKey, Membership)) {
    let (ka, kb) = (NodeKey::generate(), NodeKey::generate());
    let nk = NetworkKey::generate();
    let mut ma = Membership::ephemeral(nk.clone());
    let mut mb = Membership::ephemeral(nk);
    ma.add(Member::new(kb.public(), "b")).unwrap();
    mb.add(Member::new(ka.public(), "a")).unwrap();
    ((ka, ma), (kb, mb))
}

pub fn fast_cfg(addr: SocketAddr, key: &NodeKey, network_id: [u8; 32]) -> RelayClientConfig {
    RelayClientConfig {
        url: url(addr),
        key: key.clone(),
        network_id,
        backoff_base: Duration::from_millis(20),
        backoff_max: Duration::from_millis(200),
    }
}

pub async fn online(addr: SocketAddr, key: &NodeKey, network_id: [u8; 32]) -> RelayHandle {
    let h = RelayHandle::connect(fast_cfg(addr, key, network_id));
    h.wait_online(Duration::from_secs(5)).await.unwrap();
    h
}

/// Relay on its own runtime: dropping/killing it closes every connection (unlike aborting
/// `axum::serve`, which leaves per-connection tasks running).
pub struct RelayProc(Option<tokio::runtime::Runtime>);

impl RelayProc {
    pub fn start(addr: &str) -> std::io::Result<(SocketAddr, Self)> {
        let std_l = std::net::TcpListener::bind(addr)?;
        std_l.set_nonblocking(true)?;
        let addr = std_l.local_addr()?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?;
        let app = router(Relay::new(RelayLimits::default()).unwrap());
        rt.spawn(async move {
            let l = tokio::net::TcpListener::from_std(std_l).unwrap();
            let _ = axum::serve(l, app).await;
        });
        Ok((addr, Self(Some(rt))))
    }

    pub fn kill(&mut self) {
        if let Some(rt) = self.0.take() {
            rt.shutdown_background();
        }
    }
}

impl Drop for RelayProc {
    fn drop(&mut self) {
        self.kill();
    }
}
