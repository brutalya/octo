//! Stream multiplexer carried over an ordered, reliable frame channel (a Noise session).
//!
//! Frame: `stream_id u32 BE || kind u8 || body`. Kinds: 0 open, 1 data, 2 end, 3 reset,
//! 4 window (body u32 BE = bytes of credit). The initiator opens odd ids, the responder even.
//! Flow control: each direction of each stream starts with `INITIAL_WINDOW` bytes of credit;
//! the receiver returns credit only when the application *reads* data, so a slow reader
//! stalls only its own stream's writer. Data beyond the window resets the stream.
use crate::error::{NetError, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;

/// Largest data body in one frame (whole frame stays below the 64 KB session limit).
pub const MAX_DATA: usize = 64_000;
pub const INITIAL_WINDOW: u32 = 262_144;
const WRITE_CHUNK: usize = 16_384;
const MAX_STREAMS: usize = 256;
const MAX_WINDOW_UPDATE: u32 = 1 << 24;

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
}

struct Shared {
    out: mpsc::Sender<Vec<u8>>,
    streams: Mutex<HashMap<u32, Arc<StreamInner>>>,
    next_id: AtomicU32,
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

    fn new_stream(&self, id: u32) -> (Arc<StreamInner>, mpsc::UnboundedReceiver<Msg>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(StreamInner {
            id,
            send_win: Semaphore::new(INITIAL_WINDOW as usize),
            inbox: tx,
            credit: AtomicI64::new(INITIAL_WINDOW as i64),
            local_end: AtomicBool::new(false),
            remote_end: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        });
        self.map().insert(id, inner.clone());
        (inner, rx)
    }

    /// Tears the stream down locally; optionally tells the peer.
    async fn kill(&self, s: &StreamInner, tell_peer: bool) {
        if s.dead.swap(true, Ordering::SeqCst) {
            return;
        }
        s.send_win.close();
        let _ = s.inbox.send(Msg::Reset);
        self.map().remove(&s.id);
        if tell_peer {
            let _ = self.out.send(frame(s.id, K_RESET, &[])).await;
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
}

impl Drop for Mux {
    fn drop(&mut self) {
        self.task.abort();
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
        let shared = Arc::new(Shared {
            out,
            streams: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(if initiator { 1 } else { 2 }),
        });
        let (accept_tx, accept_rx) = mpsc::channel(64);
        let task = tokio::spawn(read_loop(shared.clone(), inbound, accept_tx, initiator));
        Self {
            shared,
            accept_rx,
            task,
        }
    }

    pub async fn open(&self) -> Result<Stream> {
        if self.shared.map().len() >= MAX_STREAMS {
            return Err(NetError::TooLarge);
        }
        let id = self.shared.next_id.fetch_add(2, Ordering::SeqCst);
        let (inner, inbox) = self.shared.new_stream(id);
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

async fn read_loop(
    shared: Arc<Shared>,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    accept: mpsc::Sender<Stream>,
    initiator: bool,
) {
    while let Some(f) = inbound.recv().await {
        let ([a, b, c, d, kind, body @ ..], true) = (f.as_slice(), f.len() >= 5) else {
            continue;
        };
        let id = u32::from_be_bytes([*a, *b, *c, *d]);
        let existing = shared.map().get(&id).cloned();
        match *kind {
            K_OPEN => {
                // Peer ids have the opposite parity of ours (initiator = odd).
                let peer_parity_ok = id != 0 && (id % 2 == 1) != initiator;
                if existing.is_some() || !peer_parity_ok || shared.map().len() >= MAX_STREAMS {
                    let _ = shared.out.send(frame(id, K_RESET, &[])).await;
                    continue;
                }
                let (inner, inbox) = shared.new_stream(id);
                let s = Stream {
                    inner: inner.clone(),
                    shared: shared.clone(),
                    inbox,
                    read_done: false,
                };
                if accept.try_send(s).is_err() {
                    shared.kill(&inner, true).await;
                }
            }
            K_DATA => {
                let Some(s) = existing else { continue };
                let over = body.len() > MAX_DATA
                    || s.credit.fetch_sub(body.len() as i64, Ordering::SeqCst)
                        - (body.len() as i64)
                        < 0
                    || s.remote_end.load(Ordering::SeqCst);
                if over {
                    shared.kill(&s, true).await;
                } else {
                    let _ = s.inbox.send(Msg::Data(body.to_vec()));
                }
            }
            K_END => {
                let Some(s) = existing else { continue };
                s.remote_end.store(true, Ordering::SeqCst);
                let _ = s.inbox.send(Msg::End);
                shared.forget_if_done(&s);
            }
            K_RESET => {
                if let Some(s) = existing {
                    shared.kill(&s, false).await;
                }
            }
            K_WINDOW => {
                let Some(s) = existing else { continue };
                match <[u8; 4]>::try_from(body).map(u32::from_be_bytes) {
                    Ok(n) if n <= MAX_WINDOW_UPDATE => s.send_win.add_permits(n as usize),
                    _ => shared.kill(&s, true).await,
                }
            }
            _ => {}
        }
    }
    // Link closed: fail every stream.
    let all: Vec<_> = shared.map().values().cloned().collect();
    for s in all {
        shared.kill(&s, false).await;
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
            self.shared
                .out
                .send(frame(self.inner.id, K_DATA, chunk))
                .await
                .map_err(|_| NetError::Closed)?;
        }
        Ok(())
    }

    /// Half-close: no more writes from this side.
    pub async fn finish(&self) -> Result<()> {
        if self.inner.dead.load(Ordering::SeqCst) {
            return Err(NetError::Reset);
        }
        if !self.inner.local_end.swap(true, Ordering::SeqCst) {
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
        self.shared.kill(&self.inner, true).await;
    }

    /// Next chunk; `Ok(None)` after the peer's `finish`. Reading returns credit to the peer.
    pub async fn read(&mut self) -> Result<Option<Vec<u8>>> {
        if self.read_done {
            return Ok(None);
        }
        match self.inbox.recv().await {
            Some(Msg::Data(b)) => {
                self.inner
                    .credit
                    .fetch_add(b.len() as i64, Ordering::SeqCst);
                if !self.inner.remote_end.load(Ordering::SeqCst)
                    && !self.inner.dead.load(Ordering::SeqCst)
                {
                    let w = (b.len() as u32).to_be_bytes();
                    let _ = self
                        .shared
                        .out
                        .send(frame(self.inner.id, K_WINDOW, &w))
                        .await;
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
        if !finished && !i.dead.swap(true, Ordering::SeqCst) {
            i.send_win.close();
            let _ = self.shared.out.try_send(frame(i.id, K_RESET, &[]));
        }
        self.shared.map().remove(&i.id);
    }
}
