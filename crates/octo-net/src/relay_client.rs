//! Relay client: authenticated WebSocket link with automatic reconnect and backoff.
//! See `relay` for the wire protocol.
use crate::NOISE_MAX;
use crate::error::{NetError, Result};
use crate::identity::{NodeId, NodeKey};
pub use crate::relay::{ERR_BUSY, ERR_PEER_UNAVAILABLE};
use crate::relay::{
    KEEPALIVE_EVERY, Link, MAX_PAYLOAD, RELAY_PARAMS, RELAY_PROLOGUE, RELAY_VERSION, SharedLink,
    T_DELIVER, T_ERROR, T_KEEPALIVE, T_READY, T_SEND, lock_link,
};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use snow::Builder;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;
/// A socket write that takes longer than this ends the session (reconnect).
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct RelayClientConfig {
    /// Full URL including the path, e.g. `ws://host:7878/relay`.
    pub url: String,
    pub key: NodeKey,
    pub network_id: [u8; 32],
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

/// Events are best-effort: `Connected`, `Disconnected` and `RelayError` are sent with `try_send`
/// and are lost if the event queue is full. `Frame` is never dropped (it applies backpressure).
/// The source of truth for the link state is [`RelayHandle::is_online`] /
/// [`RelayHandle::wait_online`], which is updated before the matching event is queued.
#[derive(Debug, PartialEq, Eq)]
pub enum Event {
    Connected,
    Disconnected,
    Frame {
        from: NodeId,
        payload: Vec<u8>,
    },
    /// Relay refused or could not deliver a frame (`ERR_*`).
    RelayError(u8),
}

pub struct RelayHandle {
    cmd: Option<mpsc::Sender<(NodeId, Vec<u8>)>>,
    events: mpsc::Receiver<Event>,
    online: watch::Receiver<bool>,
    task: Option<JoinHandle<()>>,
}

/// Dropping aborts the connection immediately (queued frames are lost); use `shutdown` to flush.
impl Drop for RelayHandle {
    fn drop(&mut self) {
        if let Some(t) = &self.task {
            t.abort();
        }
    }
}

/// Upper bound of the delay before reconnect attempt `attempt` (0-based): `base * 2^attempt`, capped.
pub fn backoff_ceiling(base: Duration, max: Duration, attempt: u32) -> Duration {
    base.saturating_mul(1u32.checked_shl(attempt.min(30)).unwrap_or(u32::MAX))
        .min(max)
}

/// Ceiling with jitter in [ceiling/2, ceiling].
pub fn backoff_delay(base: Duration, max: Duration, attempt: u32) -> Duration {
    let c = backoff_ceiling(base, max, attempt);
    let frac: f64 = rand::random::<u32>() as f64 / u32::MAX as f64;
    c / 2 + c.mul_f64(0.5 * frac)
}

impl RelayHandle {
    /// Spawns the connection task (needs a tokio runtime). Returns immediately.
    pub fn connect(cfg: RelayClientConfig) -> Self {
        let (cmd, cmd_rx) = mpsc::channel(256);
        let (ev_tx, events) = mpsc::channel(1024);
        let (on_tx, online) = watch::channel(false);
        let task = tokio::spawn(run(cfg, cmd_rx, ev_tx, on_tx));
        Self {
            cmd: Some(cmd),
            events,
            online,
            task: Some(task),
        }
    }

    /// True while the authenticated relay link is up. Authoritative; see [`Event`].
    pub fn is_online(&self) -> bool {
        *self.online.borrow()
    }

    pub async fn wait_online(&self, timeout: Duration) -> Result<()> {
        let mut rx = self.online.clone();
        tokio::time::timeout(timeout, rx.wait_for(|v| *v))
            .await
            .map_err(|_| NetError::Timeout)?
            .map_err(|_| NetError::Closed)?;
        Ok(())
    }

    /// Queues a frame for `to`. Fails fast while offline (no hidden buffering across reconnects).
    pub async fn send(&self, to: NodeId, payload: Vec<u8>) -> Result<()> {
        if payload.len() > MAX_PAYLOAD {
            return Err(NetError::TooLarge);
        }
        if !self.is_online() {
            return Err(NetError::Offline);
        }
        let cmd = self.cmd.as_ref().ok_or(NetError::Closed)?;
        cmd.send((to, payload)).await.map_err(|_| NetError::Closed)
    }

    /// Flushes frames already queued by `send`, then closes the connection.
    pub async fn shutdown(mut self) {
        self.cmd = None;
        if let Some(t) = self.task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), t).await;
        }
    }

    pub async fn recv(&mut self) -> Option<Event> {
        self.events.recv().await
    }
}

async fn run(
    cfg: RelayClientConfig,
    mut cmd: mpsc::Receiver<(NodeId, Vec<u8>)>,
    events: mpsc::Sender<Event>,
    online: watch::Sender<bool>,
) {
    let mut attempt = 0u32;
    loop {
        let mut ready = false;
        let _ = session(&cfg, &mut cmd, &events, &online, &mut ready).await;
        if *online.borrow() {
            let _ = online.send(false);
            let _ = events.try_send(Event::Disconnected);
        }
        if events.is_closed() || cmd.is_closed() {
            return;
        }
        attempt = if ready { 0 } else { attempt.saturating_add(1) };
        tokio::time::sleep(backoff_delay(
            cfg.backoff_base,
            cfg.backoff_max,
            attempt.saturating_sub(1),
        ))
        .await;
        // Frames queued while the link was down are stale for stateful Noise sessions: drop them.
        while cmd.try_recv().is_ok() {}
    }
}

async fn recv_bin<S>(ws: &mut S) -> Result<Vec<u8>>
where
    S: futures_util::Stream<
            Item = std::result::Result<Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(b))) => return Ok(b.to_vec()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            _ => return Err(NetError::Closed),
        }
    }
}

/// TLS for `wss://` relays: explicit crypto provider (several are linked into the binary, so the
/// process default is ambiguous) and the bundled Mozilla roots (same on every platform).
fn tls_connector() -> Result<tokio_tungstenite::Connector> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| NetError::Relay(e.to_string()))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(tokio_tungstenite::Connector::Rustls(Arc::new(cfg)))
}

fn werr(e: tokio_tungstenite::tungstenite::Error) -> NetError {
    NetError::Relay(e.to_string())
}

async fn session(
    cfg: &RelayClientConfig,
    cmd: &mut mpsc::Receiver<(NodeId, Vec<u8>)>,
    events: &mpsc::Sender<Event>,
    online: &watch::Sender<bool>,
    ready: &mut bool,
) -> Result<()> {
    let to = Duration::from_secs(10);
    let connect = tokio_tungstenite::connect_async_tls_with_config(
        cfg.url.as_str(),
        None,
        false,
        Some(tls_connector()?),
    );
    let (mut ws, _) = tokio::time::timeout(to, connect)
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(werr)?;
    let mut hs = Builder::new(RELAY_PARAMS.parse()?)
        .local_private_key(cfg.key.secret())?
        .prologue(RELAY_PROLOGUE)?
        .build_initiator()?;
    let mut buf = vec![0u8; NOISE_MAX];
    let n = hs.write_message(&[], &mut buf)?;
    let mut m1 = vec![RELAY_VERSION];
    m1.extend_from_slice(&buf[..n]);
    ws.send(Message::Binary(m1.into())).await.map_err(werr)?;
    let m2 = tokio::time::timeout(to, recv_bin(&mut ws))
        .await
        .map_err(|_| NetError::Timeout)??;
    hs.read_message(&m2, &mut buf)?;
    let n = hs.write_message(&cfg.network_id, &mut buf)?;
    ws.send(Message::Binary(buf[..n].to_vec().into()))
        .await
        .map_err(werr)?;
    let link: SharedLink = Arc::new(Mutex::new(Link(hs.into_transport_mode()?)));
    let first = tokio::time::timeout(to, recv_bin(&mut ws))
        .await
        .map_err(|_| NetError::Timeout)??;
    if lock_link(&link).open(&first)? != [T_READY] {
        return Err(NetError::Relay("expected READY".into()));
    }
    *ready = true;
    let _ = online.send(true);
    let _ = events.try_send(Event::Connected);
    let (sink, stream) = ws.split();
    // Reader and writer run concurrently: a blocked socket write never stops reads.
    let writer = write_half(sink, link.clone(), cmd);
    let reader = read_half(stream, link, events);
    tokio::select! {
        r = writer => r,
        r = reader => r,
    }
}

async fn write_timed(
    sink: &mut SplitSink<Ws, Message>,
    link: &SharedLink,
    pt: &[u8],
) -> Result<()> {
    let ct = lock_link(link).seal(pt)?;
    tokio::time::timeout(WRITE_TIMEOUT, sink.send(Message::Binary(ct.into())))
        .await
        .map_err(|_| NetError::Timeout)?
        .map_err(werr)
}

/// Sends queued frames and a keepalive every `KEEPALIVE_EVERY`; closes the link when the
/// command queue is closed (`shutdown`, after flushing what was queued).
async fn write_half(
    mut sink: SplitSink<Ws, Message>,
    link: SharedLink,
    cmd: &mut mpsc::Receiver<(NodeId, Vec<u8>)>,
) -> Result<()> {
    let mut tick = tokio::time::interval(KEEPALIVE_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    loop {
        tokio::select! {
            c = cmd.recv() => {
                let Some((to, payload)) = c else {
                    let _ = sink.close().await;
                    return Ok(());
                };
                let mut pt = Vec::with_capacity(17 + payload.len());
                pt.push(T_SEND);
                pt.extend_from_slice(to.as_bytes());
                pt.extend_from_slice(&payload);
                write_timed(&mut sink, &link, &pt).await?;
                tick.reset();
            }
            _ = tick.tick() => write_timed(&mut sink, &link, &[T_KEEPALIVE]).await?,
        }
    }
}

async fn read_half(
    mut stream: SplitStream<Ws>,
    link: SharedLink,
    events: &mpsc::Sender<Event>,
) -> Result<()> {
    loop {
        let ct = recv_bin(&mut stream).await?;
        let pt = lock_link(&link).open(&ct)?;
        match pt.split_first() {
            Some((&T_DELIVER, rest)) if rest.len() >= 16 => {
                let (from, payload) = rest.split_at(16);
                let from = NodeId::from_bytes(from.try_into().map_err(|_| NetError::Malformed)?);
                let ev = Event::Frame {
                    from,
                    payload: payload.to_vec(),
                };
                if events.send(ev).await.is_err() {
                    return Ok(());
                }
            }
            Some((&T_ERROR, [code])) => {
                let _ = events.try_send(Event::RelayError(*code));
            }
            _ => return Err(NetError::Malformed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps_with_bounded_jitter() {
        let (b, m) = (Duration::from_millis(100), Duration::from_secs(5));
        let c: Vec<_> = (0..8).map(|a| backoff_ceiling(b, m, a)).collect();
        assert_eq!(c[0], Duration::from_millis(100));
        assert_eq!(c[3], Duration::from_millis(800));
        assert_eq!(c[7], m);
        assert_eq!(backoff_ceiling(b, m, 1000), m);
        for a in 0..10 {
            for _ in 0..50 {
                let d = backoff_delay(b, m, a);
                let ce = backoff_ceiling(b, m, a);
                assert!(d >= ce / 2 && d <= ce);
            }
        }
    }
}
