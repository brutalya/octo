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
//! - C->R `0x14` (KEEPALIVE): no payload, never forwarded. Clients send one every 30 s so an
//!   authenticated connection that stays silent for `idle_timeout` (120 s) can be closed.
//!
//! Each connection runs a reader and a writer concurrently, so a slow socket write never stops
//! reads (and vice versa); a write that does not complete within `write_timeout` closes the
//! connection.
use crate::NOISE_MAX;
use crate::error::Result;
use crate::identity::NodeId;
use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use snow::{Builder, TransportState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc};

pub const RELAY_VERSION: u8 = 1;
pub const RELAY_PARAMS: &str = crate::XX_PARAMS;
pub const RELAY_PROLOGUE: &[u8] = b"octo-relay-v1";
/// 65535 (Noise max) - 16 (tag) - 1 (type) - 16 (node id).
pub const MAX_PAYLOAD: usize = 65_502;

pub(crate) const T_SEND: u8 = 0x10;
pub(crate) const T_DELIVER: u8 = 0x11;
pub(crate) const T_ERROR: u8 = 0x12;
pub(crate) const T_READY: u8 = 0x13;
pub(crate) const T_KEEPALIVE: u8 = 0x14;
/// How often clients send `T_KEEPALIVE`.
pub(crate) const KEEPALIVE_EVERY: Duration = Duration::from_secs(30);

pub const ERR_PEER_UNAVAILABLE: u8 = 1;
pub const ERR_BUSY: u8 = 3;

/// Noise transport state of the node<->relay link.
pub(crate) struct Link(pub TransportState);

/// The link is shared by the reader (open) and the writer (seal) of one connection; the lock is
/// only held for the CPU-bound cipher call, never across an await.
pub(crate) type SharedLink = Arc<Mutex<Link>>;

pub(crate) fn lock_link(l: &SharedLink) -> MutexGuard<'_, Link> {
    l.lock().unwrap_or_else(|e| e.into_inner())
}

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
pub(crate) async fn recv_bin_stream<S>(ws: &mut S) -> Option<Vec<u8>>
where
    S: futures_util::Stream<Item = std::result::Result<Message, axum::Error>> + Unpin,
{
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
    /// Time allowed for the whole pre-auth handshake.
    pub handshake_timeout: Duration,
    /// An authenticated connection that sends nothing for this long is closed.
    pub idle_timeout: Duration,
    /// A socket write that takes longer than this closes the connection.
    pub write_timeout: Duration,
    pub max_connections: usize,
    /// Per-connection outbound queue (messages).
    pub queue: usize,
    /// A connection that stays over its limit this long (continuously) is closed.
    pub abuse_after: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            msgs_per_sec: 200.0,
            msg_burst: 400.0,
            bytes_per_sec: 8.0 * 1024.0 * 1024.0,
            byte_burst: 16.0 * 1024.0 * 1024.0,
            handshake_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(120),
            write_timeout: Duration::from_secs(30),
            max_connections: 1024,
            queue: 256,
            abuse_after: Duration::from_secs(30),
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

    /// Takes `cost` tokens unconditionally (the balance may go negative) and returns how long
    /// the caller must wait before the debt is repaid. Zero when tokens were available.
    pub fn reserve(&mut self, cost: f64, now: Instant) -> Duration {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.capacity) - cost;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
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
    conns: Arc<Semaphore>,
}

#[derive(Clone)]
pub struct Relay(Arc<Inner>);

impl Relay {
    pub fn new(limits: RelayLimits) -> Result<Self> {
        // The relay key is per process; clients do not pin it (the relay is untrusted).
        let kp = Builder::new(RELAY_PARAMS.parse()?).generate_keypair()?;
        Ok(Self(Arc::new(Inner {
            conns: Arc::new(Semaphore::new(limits.max_connections)),
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
    // The permit lives in the upgrade closure: it is released when the connection ends, and
    // also when the upgrade fails and the closure is dropped without running.
    let Ok(permit) = relay.0.conns.clone().try_acquire_owned() else {
        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.max_message_size(NOISE_MAX + 64)
        .max_frame_size(NOISE_MAX + 64)
        .on_upgrade(move |s| async move {
            let _permit = permit;
            handle_socket(&relay, s).await;
        })
}

async fn handshake(relay: &Relay, ws: &mut WebSocket) -> Option<(Link, NodeId, [u8; 32])> {
    let m1 = recv_bin_stream(ws).await?;
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
    let m3 = recv_bin_stream(ws).await?;
    let n = hs.read_message(&m3, &mut buf).ok()?;
    let net: [u8; 32] = buf[..n].try_into().ok()?;
    let pk: [u8; 32] = hs.get_remote_static()?.try_into().ok()?;
    let link = Link(hs.into_transport_mode().ok()?);
    Some((link, NodeId::from_pubkey(&pk), net))
}

async fn handle_socket(relay: &Relay, mut ws: WebSocket) {
    let hs =
        tokio::time::timeout(relay.0.limits.handshake_timeout, handshake(relay, &mut ws)).await;
    let Ok(Some((link, me, net))) = hs else {
        return;
    };
    let link: SharedLink = Arc::new(Mutex::new(link));
    let (tx, rx) = mpsc::channel::<Vec<u8>>(relay.0.limits.queue);
    let conn = relay.0.next_conn.fetch_add(1, Ordering::Relaxed);
    relay.register(net, me, Peer { conn, tx });
    run_connection(relay, ws, link, rx, me, net).await;
    relay.unregister(net, me, conn);
}

/// Seals `pt` and writes it, giving up after `limit`.
async fn send_sealed(
    sink: &mut SplitSink<WebSocket, Message>,
    link: &SharedLink,
    pt: &[u8],
    limit: Duration,
) -> bool {
    let Ok(ct) = lock_link(link).seal(pt) else {
        return false;
    };
    matches!(
        tokio::time::timeout(limit, sink.send(Message::Binary(ct.into()))).await,
        Ok(Ok(()))
    )
}

/// Writer half: delivers other nodes' frames (`rx`) and this connection's own control
/// messages (`ctl`: READY, ERROR). Returns when the socket fails or times out.
async fn write_loop(
    mut sink: SplitSink<WebSocket, Message>,
    link: SharedLink,
    mut rx: mpsc::Receiver<Vec<u8>>,
    mut ctl: mpsc::Receiver<Vec<u8>>,
    limit: Duration,
) {
    loop {
        let m = tokio::select! {
            // None: replaced by a newer connection of the same node.
            m = rx.recv() => m,
            m = ctl.recv() => m,
        };
        let Some(m) = m else { return };
        if !send_sealed(&mut sink, &link, &m, limit).await {
            return;
        }
    }
}

/// Reader half: decrypts, rate-limits and routes. Returns on close, error or violation.
async fn read_loop(
    relay: &Relay,
    mut stream: SplitStream<WebSocket>,
    link: SharedLink,
    ctl: mpsc::Sender<Vec<u8>>,
    me: NodeId,
    net: [u8; 32],
) {
    let l = &relay.0.limits;
    let now = Instant::now();
    let mut msgs = TokenBucket::new(l.msg_burst, l.msgs_per_sec, now);
    let mut bytes = TokenBucket::new(l.byte_burst, l.bytes_per_sec, now);
    // Start of the current uninterrupted over-limit period.
    let mut over_since: Option<Instant> = None;
    loop {
        let Ok(Some(ct)) = tokio::time::timeout(l.idle_timeout, recv_bin_stream(&mut stream)).await
        else {
            return;
        };
        // Must decrypt every message even if we then drop it: the Noise nonce is implicit.
        let Ok(pt) = lock_link(&link).open(&ct) else {
            return;
        };
        let payload = match pt.split_first() {
            Some((&T_KEEPALIVE, [])) => &[][..],
            Some((&T_SEND, rest)) if rest.len() >= 16 && rest.len() - 16 <= MAX_PAYLOAD => {
                &rest[16..]
            }
            _ => return,
        };
        let t = Instant::now();
        // Backpressure: stop reading this connection until the debt is repaid, so the sender's
        // WebSocket/TCP window fills. Frames are never dropped. The writer keeps flowing.
        let wait = msgs
            .reserve(1.0, t)
            .max(bytes.reserve(payload.len() as f64, t));
        if wait.is_zero() {
            over_since = None;
        } else {
            let since = *over_since.get_or_insert(t);
            if t + wait > since + l.abuse_after {
                return;
            }
            tokio::time::sleep(wait).await;
        }
        if pt[0] == T_KEEPALIVE {
            continue;
        }
        let Ok(to) = <[u8; 16]>::try_from(&pt[1..17]) else {
            return;
        };
        let code = match relay.lookup(&net, &NodeId::from_bytes(to)) {
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
            && ctl.send(vec![T_ERROR, c]).await.is_err()
        {
            return;
        }
    }
}

async fn run_connection(
    relay: &Relay,
    ws: WebSocket,
    link: SharedLink,
    rx: mpsc::Receiver<Vec<u8>>,
    me: NodeId,
    net: [u8; 32],
) {
    let (sink, stream) = ws.split();
    let (ctl_tx, ctl_rx) = mpsc::channel(16);
    if ctl_tx.send(vec![T_READY]).await.is_err() {
        return;
    }
    let limit = relay.0.limits.write_timeout;
    let writer = write_loop(sink, link.clone(), rx, ctl_rx, limit);
    let reader = read_loop(relay, stream, link, ctl_tx, me, net);
    // Either half ending (close, error, timeout, violation) ends the connection.
    tokio::select! {
        () = writer => {}
        () = reader => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_refills_over_time() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(3.0, 1.0, t0);
        let s1 = Duration::from_secs(1);
        assert!(b.reserve(1.0, t0).is_zero() && b.reserve(1.0, t0).is_zero());
        assert!(b.reserve(1.0, t0).is_zero());
        // Empty: the next token costs one second of waiting.
        assert_eq!(b.reserve(1.0, t0), s1);
        // One second later the debt of one token is repaid; the next reserve waits again.
        assert_eq!(b.reserve(1.0, t0 + s1), s1);
        // Never exceeds capacity.
        let later = t0 + Duration::from_secs(100);
        assert!(b.reserve(3.0, later).is_zero());
        assert!(!b.reserve(1.0, later).is_zero());
    }
}
