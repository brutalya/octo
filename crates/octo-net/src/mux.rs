//! Stream multiplexer carried over an ordered, reliable frame channel (a Noise session).
//!
//! Frame: `stream_id u32 BE || kind u8 || body`. Kinds: 0 open, 1 data, 2 end, 3 reset,
//! 4 window (body u32 BE = bytes of credit). The initiator opens odd ids, the responder even.
//! Flow control: each direction of each stream starts with `INITIAL_WINDOW` bytes of credit;
//! the receiver returns credit only when the application *reads* data, so a slow reader
//! stalls only its own stream's writer. Data beyond the window, or an empty data frame, resets
//! the stream; a data/end/window frame for an unknown stream is answered with RESET (once per id).
//! RESET and window frames never block the read loop: if the outgoing queue is full they wait in
//! an internal queue drained by a pump task.
use crate::error::{NetError, Result};
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, mpsc, watch};
use tokio::task::JoinHandle;

/// Largest data body in one frame (whole frame stays below the 64 KB session limit).
pub const MAX_DATA: usize = 64_000;
pub const INITIAL_WINDOW: u32 = 262_144;
const WRITE_CHUNK: usize = 16_384;
/// A stream with no frame in either direction for this long is reset.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_STREAMS: usize = 256;
const MAX_WINDOW_UPDATE: u32 = 1 << 24;
/// Remembered ids we already answered with RESET (bounded; cleared when full).
const MAX_RESET_MEMORY: usize = 4096;

const K_OPEN: u8 = 0;
const K_DATA: u8 = 1;
const K_END: u8 = 2;
const K_RESET: u8 = 3;
const K_WINDOW: u8 = 4;

enum Msg {
    Data(Vec<u8>),
    End,
    Reset,
}

struct StreamInner {
    id: u32,
    send_win: Semaphore,
    inbox: mpsc::UnboundedSender<Msg>,
    /// Bytes the peer may still send; goes down on arrival, up when the app reads.
    credit: AtomicI64,
    local_end: AtomicBool,
    remote_end: AtomicBool,
    dead: AtomicBool,
    /// Flips to true with `dead`; wakes writers blocked on the outgoing queue.
    dead_tx: watch::Sender<bool>,
    /// Last frame in either direction, in ms since `Shared::base`.
    last: AtomicU64,
}

struct Shared {
    out: mpsc::Sender<Vec<u8>>,
    /// Overflow queue for RESET/WINDOW frames when `out` is full.
    ctl: mpsc::UnboundedSender<Vec<u8>>,
    streams: Mutex<HashMap<u32, Arc<StreamInner>>>,
    /// Next id to hand out (u64 so exhaustion is detectable).
    next_id: AtomicU64,
    base: Instant,
}

impl StreamInner {
    /// Marks the stream dead and wakes its writers; true if this call did it.
    fn mark_dead(&self) -> bool {
        if self.dead.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.send_win.close();
        self.dead_tx.send_replace(true);
        true
    }
}

fn frame(id: u32, kind: u8, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(5 + body.len());
    f.extend_from_slice(&id.to_be_bytes());
    f.push(kind);
    f.extend_from_slice(body);
    f
}

impl Shared {
    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<u32, Arc<StreamInner>>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn now_ms(&self) -> u64 {
        self.base.elapsed().as_millis() as u64
    }

    fn touch(&self, s: &StreamInner) {
        s.last.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Registers a stream: the peer's `id`, or the next local id when `None`. The capacity
    /// check and the insert happen under one lock.
    fn new_stream(
        &self,
        id: Option<u32>,
    ) -> Result<(Arc<StreamInner>, mpsc::UnboundedReceiver<Msg>)> {
        let mut map = self.map();
        if map.len() >= MAX_STREAMS {
            return Err(NetError::TooLarge);
        }
        let id = match id {
            Some(id) if map.contains_key(&id) => return Err(NetError::Malformed),
            Some(id) => id,
            None => {
                let next = self.next_id.load(Ordering::SeqCst);
                if next > u64::from(u32::MAX) {
                    return Err(NetError::Closed);
                }
                self.next_id.store(next + 2, Ordering::SeqCst);
                next as u32
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(StreamInner {
            id,
            send_win: Semaphore::new(INITIAL_WINDOW as usize),
            inbox: tx,
            credit: AtomicI64::new(INITIAL_WINDOW as i64),
            local_end: AtomicBool::new(false),
            remote_end: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            dead_tx: watch::channel(false).0,
            last: AtomicU64::new(self.now_ms()),
        });
        map.insert(id, inner.clone());
        Ok((inner, rx))
    }

    /// Queues a RESET/WINDOW frame without ever waiting: `out` if it has room, else the
    /// overflow queue.
    fn send_ctl(&self, f: Vec<u8>) {
        if let Err(mpsc::error::TrySendError::Full(f)) = self.out.try_send(f) {
            let _ = self.ctl.send(f);
        }
    }

    /// Tears the stream down locally; optionally tells the peer.
    fn kill(&self, s: &StreamInner, tell_peer: bool) {
        if !s.mark_dead() {
            return;
        }
        let _ = s.inbox.send(Msg::Reset);
        self.map().remove(&s.id);
        if tell_peer {
            self.send_ctl(frame(s.id, K_RESET, &[]));
        }
    }

    fn forget_if_done(&self, s: &StreamInner) {
        if s.local_end.load(Ordering::SeqCst) && s.remote_end.load(Ordering::SeqCst) {
            self.map().remove(&s.id);
        }
    }
}

pub struct Mux {
    shared: Arc<Shared>,
    accept_rx: mpsc::Receiver<Stream>,
    task: JoinHandle<()>,
    watchdog: JoinHandle<()>,
    pump: JoinHandle<()>,
}

impl Drop for Mux {
    fn drop(&mut self) {
        self.task.abort();
        self.watchdog.abort();
        self.pump.abort();
        // The link is gone (e.g. the session was replaced): fail pending reads and wake
        // blocked writers at once.
        let streams: Vec<_> = self.shared.map().drain().map(|(_, s)| s).collect();
        for s in streams {
            s.mark_dead();
            let _ = s.inbox.send(Msg::Reset);
        }
    }
}

impl Mux {
    /// `out` receives frames to send; `inbound` yields frames received from the peer.
    /// Exactly one side must pass `initiator = true`. Needs a tokio runtime.
    pub fn new(
        out: mpsc::Sender<Vec<u8>>,
        inbound: mpsc::Receiver<Vec<u8>>,
        initiator: bool,
    ) -> Self {
        Self::with_idle(out, inbound, initiator, IDLE_TIMEOUT)
    }

    /// Like `new`, with an explicit per-stream idle timeout.
    pub fn with_idle(
        out: mpsc::Sender<Vec<u8>>,
        inbound: mpsc::Receiver<Vec<u8>>,
        initiator: bool,
        idle: Duration,
    ) -> Self {
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            out,
            ctl,
            streams: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(if initiator { 1 } else { 2 }),
            base: Instant::now(),
        });
        let (accept_tx, accept_rx) = mpsc::channel(64);
        let task = tokio::spawn(read_loop(shared.clone(), inbound, accept_tx, initiator));
        let watchdog = tokio::spawn(watchdog(shared.clone(), idle));
        let pump = tokio::spawn(pump(shared.out.clone(), ctl_rx));
        Self {
            shared,
            accept_rx,
            task,
            watchdog,
            pump,
        }
    }

    pub async fn open(&self) -> Result<Stream> {
        let (inner, inbox) = self.shared.new_stream(None)?;
        let id = inner.id;
        let s = Stream {
            inner,
            shared: self.shared.clone(),
            inbox,
            read_done: false,
        };
        self.shared
            .out
            .send(frame(id, K_OPEN, &[]))
            .await
            .map_err(|_| NetError::Closed)?;
        Ok(s)
    }

    /// Next stream opened by the peer; `None` when the link is closed.
    pub async fn accept(&mut self) -> Option<Stream> {
        self.accept_rx.recv().await
    }
}

/// Forwards overflowed control frames to `out`, off the read loop.
async fn pump(out: mpsc::Sender<Vec<u8>>, mut ctl: mpsc::UnboundedReceiver<Vec<u8>>) {
    while let Some(f) = ctl.recv().await {
        if out.send(f).await.is_err() {
            return;
        }
    }
}

/// Resets streams that saw no frame in either direction for `idle` (stalled or dead peer).
async fn watchdog(shared: Arc<Shared>, idle: Duration) {
    let tick = (idle / 4).max(Duration::from_millis(10));
    loop {
        tokio::time::sleep(tick).await;
        let now = shared.now_ms();
        let stale: Vec<_> = shared
            .map()
            .values()
            .filter(|s| {
                now.saturating_sub(s.last.load(Ordering::Relaxed)) >= idle.as_millis() as u64
            })
            .cloned()
            .collect();
        for s in stale {
            shared.kill(&s, true);
        }
    }
}

async fn read_loop(
    shared: Arc<Shared>,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    accept: mpsc::Sender<Stream>,
    initiator: bool,
) {
    let mut answered: HashSet<u32> = HashSet::new();
    while let Some(f) = inbound.recv().await {
        let Some((id, rest)) = f.split_first_chunk::<4>() else {
            continue;
        };
        let [kind, body @ ..] = rest else { continue };
        let id = u32::from_be_bytes(*id);
        let existing = shared.map().get(&id).cloned();
        if let Some(s) = &existing {
            shared.touch(s);
        }
        // Answers a frame the peer should not have sent with RESET, at most once per id.
        let mut reset_unknown = |shared: &Shared| {
            if answered.len() >= MAX_RESET_MEMORY {
                answered.clear();
            }
            if answered.insert(id) {
                shared.send_ctl(frame(id, K_RESET, &[]));
            }
        };
        match *kind {
            K_OPEN => {
                // Peer ids have the opposite parity of ours (initiator = odd).
                let peer_parity_ok = id != 0 && (id % 2 == 1) != initiator;
                let created = if existing.is_some() || !peer_parity_ok {
                    None
                } else {
                    shared.new_stream(Some(id)).ok()
                };
                let Some((inner, inbox)) = created else {
                    reset_unknown(&shared);
                    continue;
                };
                let s = Stream {
                    inner: inner.clone(),
                    shared: shared.clone(),
                    inbox,
                    read_done: false,
                };
                if accept.try_send(s).is_err() {
                    shared.kill(&inner, true);
                }
            }
            K_DATA => {
                let Some(s) = existing else {
                    reset_unknown(&shared);
                    continue;
                };
                let over = body.is_empty()
                    || body.len() > MAX_DATA
                    || s.credit.fetch_sub(body.len() as i64, Ordering::SeqCst)
                        - (body.len() as i64)
                        < 0
                    || s.remote_end.load(Ordering::SeqCst);
                if over {
                    shared.kill(&s, true);
                } else {
                    let _ = s.inbox.send(Msg::Data(body.to_vec()));
                }
            }
            K_END => {
                let Some(s) = existing else {
                    reset_unknown(&shared);
                    continue;
                };
                s.remote_end.store(true, Ordering::SeqCst);
                let _ = s.inbox.send(Msg::End);
                shared.forget_if_done(&s);
            }
            K_RESET => {
                if let Some(s) = existing {
                    shared.kill(&s, false);
                }
            }
            K_WINDOW => {
                let Some(s) = existing else {
                    reset_unknown(&shared);
                    continue;
                };
                match <[u8; 4]>::try_from(body).map(u32::from_be_bytes) {
                    Ok(n) if n <= MAX_WINDOW_UPDATE => s.send_win.add_permits(n as usize),
                    _ => shared.kill(&s, true),
                }
            }
            _ => {}
        }
    }
    // Link closed: fail every stream.
    let all: Vec<_> = shared.map().values().cloned().collect();
    for s in all {
        shared.kill(&s, false);
    }
}

pub struct Stream {
    inner: Arc<StreamInner>,
    shared: Arc<Shared>,
    inbox: mpsc::UnboundedReceiver<Msg>,
    read_done: bool,
}

impl Stream {
    pub fn id(&self) -> u32 {
        self.inner.id
    }

    /// Writes all of `data`, waiting for peer credit as needed (backpressure).
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        for chunk in data.chunks(WRITE_CHUNK) {
            if self.inner.dead.load(Ordering::SeqCst) {
                return Err(NetError::Reset);
            }
            if self.inner.local_end.load(Ordering::SeqCst) {
                return Err(NetError::Closed);
            }
            let permit = self
                .inner
                .send_win
                .acquire_many(chunk.len() as u32)
                .await
                .map_err(|_| NetError::Reset)?;
            permit.forget();
            self.shared.touch(&self.inner);
            let mut dead = self.inner.dead_tx.subscribe();
            tokio::select! {
                sent = self.shared.out.send(frame(self.inner.id, K_DATA, chunk)) => {
                    sent.map_err(|_| NetError::Closed)?;
                }
                _ = dead.wait_for(|d| *d) => return Err(NetError::Reset),
            }
        }
        Ok(())
    }

    /// Half-close: no more writes from this side.
    pub async fn finish(&self) -> Result<()> {
        if self.inner.dead.load(Ordering::SeqCst) {
            return Err(NetError::Reset);
        }
        if !self.inner.local_end.swap(true, Ordering::SeqCst) {
            self.shared.touch(&self.inner);
            self.shared
                .out
                .send(frame(self.inner.id, K_END, &[]))
                .await
                .map_err(|_| NetError::Closed)?;
            self.shared.forget_if_done(&self.inner);
        }
        Ok(())
    }

    pub async fn reset(&self) {
        self.shared.kill(&self.inner, true);
    }

    /// Next chunk; `Ok(None)` after the peer's `finish`. Reading returns credit to the peer.
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        if self.read_done {
            return Ok(None);
        }
        match self.inbox.recv().await {
            Some(Msg::Data(b)) => {
                self.shared.touch(&self.inner);
                self.inner
                    .credit
                    .fetch_add(b.len() as i64, Ordering::SeqCst);
                if !self.inner.remote_end.load(Ordering::SeqCst)
                    && !self.inner.dead.load(Ordering::SeqCst)
                {
                    let w = (b.len() as u32).to_be_bytes();
                    self.shared.send_ctl(frame(self.inner.id, K_WINDOW, &w));
                }
                Ok(Some(b))
            }
            Some(Msg::End) => {
                self.read_done = true;
                Ok(None)
            }
            Some(Msg::Reset) | None => {
                self.read_done = true;
                Err(NetError::Reset)
            }
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let i = &self.inner;
        let finished = i.local_end.load(Ordering::SeqCst) && i.remote_end.load(Ordering::SeqCst);
        if !finished && i.mark_dead() {
            self.shared.send_ctl(frame(i.id, K_RESET, &[]));
        }
        self.shared.map().remove(&i.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stream_id_exhaustion_is_an_error_not_a_wrap() {
        let (out, _out_rx) = mpsc::channel(8);
        let (_in_tx, inbound) = mpsc::channel(8);
        let m = Mux::new(out, inbound, true);
        m.shared
            .next_id
            .store(u64::from(u32::MAX) - 2, Ordering::SeqCst);
        assert_eq!(m.open().await.unwrap().id(), u32::MAX - 2);
        assert_eq!(m.open().await.unwrap().id(), u32::MAX);
        assert!(m.open().await.is_err());
    }
}
