//! Relay server: axum WebSocket at `/relay`, routes opaque frames between authenticated
//! nodes of the same `network_id`. It sees node ids, network ids, sizes and timing only.
//!
//! Link protocol (all WebSocket binary messages):
//! 1. C->R `[RELAY_VERSION=1] || noise_xx_msg1`
//! 2. R->C `noise_xx_msg2`
//! 3. C->R `noise_xx_msg3` with payload = `network_id` (32 bytes)
//!    (`Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue `"octo-relay-v1"`). The relay's fresh
//!    ephemeral is the challenge; msg3 proves possession of the client static key, which
//!    fixes the node id. After that every message is one Noise transport message:
//! - C->R `0x10 || to(16) || payload`  (SEND)
//! - R->C `0x11 || from(16) || payload` (DELIVER; `from` is the authenticated sender)
//! - R->C `0x12 || code` (ERROR), R->C `0x13` (READY, sent once after auth)
use crate::error::Result;
use crate::identity::NodeId;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::StreamExt;
use snow::{Builder, TransportState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc};

pub const RELAY_VERSION: u8 = 1;
pub const RELAY_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
pub const RELAY_PROLOGUE: &[u8] = b"octo-relay-v1";
/// 65535 (Noise max) - 16 (tag) - 1 (type) - 16 (node id).
pub const MAX_PAYLOAD: usize = 65_502;
pub(crate) const NOISE_MAX: usize = 65_535;

pub(crate) const T_SEND: u8 = 0x10;
pub(crate) const T_DELIVER: u8 = 0x11;
pub(crate) const T_ERROR: u8 = 0x12;
pub(crate) const T_READY: u8 = 0x13;

pub const ERR_PEER_UNAVAILABLE: u8 = 1;
pub const ERR_RATE_LIMITED: u8 = 2;
pub const ERR_BUSY: u8 = 3;

/// Noise transport state of the node<->relay link.
pub(crate) struct Link(pub TransportState);

impl Link {
    pub fn seal(&mut self, pt: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; pt.len() + 16];
        let n = self.0.write_message(pt, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
    pub fn open(&mut self, ct: &[u8]) -> Result<Vec<u8>> {
        let mut out = vec![0u8; ct.len()];
        let n = self.0.read_message(ct, &mut out)?;
        out.truncate(n);
        Ok(out)
    }
}

/// Reads the next binary message; `None` on close, error, or any non-binary data frame.
pub(crate) async fn recv_bin_axum(ws: &mut WebSocket) -> Option<Vec<u8>> {
    loop {
        match ws.next().await? {
            Ok(Message::Binary(b)) => return Some(b.to_vec()),
            Ok(Message::Ping(_) | Message::Pong(_)) => continue,
            _ => return None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RelayLimits {
    pub msgs_per_sec: f64,
    pub msg_burst: f64,
    pub bytes_per_sec: f64,
    pub byte_burst: f64,
    pub handshake_timeout: Duration,
    pub max_connections: usize,
    /// Per-connection outbound queue (messages).
    pub queue: usize,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            msgs_per_sec: 200.0,
            msg_burst: 400.0,
            bytes_per_sec: 8.0 * 1024.0 * 1024.0,
            byte_burst: 16.0 * 1024.0 * 1024.0,
            handshake_timeout: Duration::from_secs(10),
            max_connections: 1024,
            queue: 256,
        }
    }
}

pub struct TokenBucket {
    capacity: f64,
    rate: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, rate_per_sec: f64, now: Instant) -> Self {
        Self {
            capacity,
            rate: rate_per_sec,
            tokens: capacity,
            last: now,
        }
    }
    pub fn allow(&mut self, cost: f64, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.capacity);
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

struct Peer {
    conn: u64,
    tx: mpsc::Sender<Vec<u8>>,
}

struct Inner {
    limits: RelayLimits,
    static_key: Vec<u8>,
    nets: Mutex<HashMap<[u8; 32], HashMap<NodeId, Peer>>>,
    next_conn: AtomicU64,
    conns: Semaphore,
}

#[derive(Clone)]
pub struct Relay(Arc<Inner>);

impl Relay {
    pub fn new(limits: RelayLimits) -> Result<Self> {
        // The relay key is per process; clients do not pin it (the relay is untrusted).
        let kp = Builder::new(RELAY_PARAMS.parse()?).generate_keypair()?;
        Ok(Self(Arc::new(Inner {
            conns: Semaphore::new(limits.max_connections),
            limits,
            static_key: kp.private,
            nets: Mutex::new(HashMap::new()),
            next_conn: AtomicU64::new(1),
        })))
    }

    fn nets(&self) -> std::sync::MutexGuard<'_, HashMap<[u8; 32], HashMap<NodeId, Peer>>> {
        self.0.nets.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register(&self, net: [u8; 32], node: NodeId, peer: Peer) {
        // A newer connection of the same (network, node) replaces the older one.
        self.nets().entry(net).or_default().insert(node, peer);
    }

    fn unregister(&self, net: [u8; 32], node: NodeId, conn: u64) {
        let mut nets = self.nets();
        if let Some(n) = nets.get_mut(&net) {
            if n.get(&node).is_some_and(|p| p.conn == conn) {
                n.remove(&node);
            }
            if n.is_empty() {
                nets.remove(&net);
            }
        }
    }

    /// Only peers registered under the *sender's* network are visible.
    fn lookup(&self, net: &[u8; 32], node: &NodeId) -> Option<mpsc::Sender<Vec<u8>>> {
        self.nets().get(net)?.get(node).map(|p| p.tx.clone())
    }
}

pub fn router(relay: Relay) -> Router {
    Router::new()
        .route("/relay", get(ws_handler))
        .with_state(relay)
}

async fn ws_handler(State(relay): State<Relay>, ws: WebSocketUpgrade) -> Response {
    match relay.0.conns.try_acquire() {
        Ok(p) => p.forget(),
        Err(_) => return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
    ws.max_message_size(NOISE_MAX + 64)
        .max_frame_size(NOISE_MAX + 64)
        .on_upgrade(move |s| async move {
            handle_socket(&relay, s).await;
            relay.0.conns.add_permits(1);
        })
}

async fn handshake(relay: &Relay, ws: &mut WebSocket) -> Option<(Link, NodeId, [u8; 32])> {
    let m1 = recv_bin_axum(ws).await?;
    let (ver, m1) = m1.split_first()?;
    if *ver != RELAY_VERSION {
        return None;
    }
    let mut hs = Builder::new(RELAY_PARAMS.parse().ok()?)
        .local_private_key(&relay.0.static_key)
        .ok()?
        .prologue(RELAY_PROLOGUE)
        .ok()?
        .build_responder()
        .ok()?;
    let mut buf = vec![0u8; NOISE_MAX];
    hs.read_message(m1, &mut buf).ok()?;
    let n = hs.write_message(&[], &mut buf).ok()?;
    ws.send(Message::Binary(buf[..n].to_vec().into()))
        .await
        .ok()?;
    let m3 = recv_bin_axum(ws).await?;
    let n = hs.read_message(&m3, &mut buf).ok()?;
    let net: [u8; 32] = buf[..n].try_into().ok()?;
    let pk: [u8; 32] = hs.get_remote_static()?.try_into().ok()?;
    let link = Link(hs.into_transport_mode().ok()?);
    Some((link, NodeId::from_pubkey(&pk), net))
}

async fn handle_socket(relay: &Relay, mut ws: WebSocket) {
    let hs =
        tokio::time::timeout(relay.0.limits.handshake_timeout, handshake(relay, &mut ws)).await;
    let Ok(Some((mut link, me, net))) = hs else {
        return;
    };
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(relay.0.limits.queue);
    let conn = relay.0.next_conn.fetch_add(1, Ordering::Relaxed);
    relay.register(net, me, Peer { conn, tx });
    run_connection(relay, &mut ws, &mut link, &mut rx, me, net).await;
    relay.unregister(net, me, conn);
}

async fn send_sealed(ws: &mut WebSocket, link: &mut Link, pt: &[u8]) -> bool {
    match link.seal(pt) {
        Ok(ct) => ws.send(Message::Binary(ct.into())).await.is_ok(),
        Err(_) => false,
    }
}

async fn run_connection(
    relay: &Relay,
    ws: &mut WebSocket,
    link: &mut Link,
    rx: &mut mpsc::Receiver<Vec<u8>>,
    me: NodeId,
    net: [u8; 32],
) {
    let l = &relay.0.limits;
    let now = Instant::now();
    let mut msgs = TokenBucket::new(l.msg_burst, l.msgs_per_sec, now);
    let mut bytes = TokenBucket::new(l.byte_burst, l.bytes_per_sec, now);
    let mut violations = 0u32;
    if !send_sealed(ws, link, &[T_READY]).await {
        return;
    }
    loop {
        tokio::select! {
            incoming = recv_bin_axum(ws) => {
                let Some(ct) = incoming else { return };
                // Must decrypt every message even if we then drop it: the Noise nonce is implicit.
                let Ok(pt) = link.open(&ct) else { return };
                let Some((&T_SEND, rest)) = pt.split_first() else { return };
                if rest.len() < 16 || rest.len() - 16 > MAX_PAYLOAD { return }
                let (to, payload) = rest.split_at(16);
                let Ok(to) = <[u8; 16]>::try_from(to) else { return };
                let to = NodeId::from_bytes(to);
                let t = Instant::now();
                if !(msgs.allow(1.0, t) && bytes.allow(payload.len() as f64, t)) {
                    violations += 1;
                    if violations > 200 { return }
                    if !send_sealed(ws, link, &[T_ERROR, ERR_RATE_LIMITED]).await { return }
                    continue;
                }
                violations = 0;
                let code = match relay.lookup(&net, &to) {
                    None => Some(ERR_PEER_UNAVAILABLE),
                    Some(tx) => {
                        let mut m = Vec::with_capacity(17 + payload.len());
                        m.push(T_DELIVER);
                        m.extend_from_slice(me.as_bytes());
                        m.extend_from_slice(payload);
                        match tx.try_send(m) {
                            Ok(()) => None,
                            Err(mpsc::error::TrySendError::Full(_)) => Some(ERR_BUSY),
                            Err(mpsc::error::TrySendError::Closed(_)) => Some(ERR_PEER_UNAVAILABLE),
                        }
                    }
                };
                if let Some(c) = code
                    && !send_sealed(ws, link, &[T_ERROR, c]).await { return }
            }
            out = rx.recv() => {
                // None: replaced by a newer connection of the same node.
                let Some(m) = out else { return };
                if !send_sealed(ws, link, &m).await { return }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_over_time() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(3.0, 1.0, t0);
        assert!(b.allow(1.0, t0) && b.allow(1.0, t0) && b.allow(1.0, t0));
        assert!(!b.allow(1.0, t0));
        assert!(b.allow(1.0, t0 + Duration::from_secs(1)));
        assert!(!b.allow(1.0, t0 + Duration::from_secs(1)));
        // Never exceeds capacity.
        let later = t0 + Duration::from_secs(100);
        assert!(b.allow(3.0, later) && !b.allow(1.0, later));
    }
}
