use super::AgentEvent;
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;

/// Longest accepted line; a longer one is a protocol error, not a reason to grow forever.
pub const MAX_LINE: usize = 8 * 1024 * 1024;

/// A stream that sends nothing for this long is considered dead.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Splits a byte stream into lines. Bytes are buffered so UTF-8 sequences and lines
/// split across network chunks stay intact. A line longer than `MAX_LINE` sets
/// `too_long()` and the buffer is dropped.
#[derive(Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
    /// Prefix of `buf` already known to contain no newline.
    scanned: usize,
    too_long: bool,
}

impl LineBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        if self.too_long {
            return vec![];
        }
        self.buf.extend_from_slice(chunk);
        let mut out = vec![];
        let mut start = 0;
        loop {
            let from = self.scanned.max(start);
            let Some(i) = self.buf[from..].iter().position(|&b| b == b'\n') else {
                break;
            };
            let end = from + i;
            if end - start > MAX_LINE {
                return self.overflow();
            }
            out.push(decode(&self.buf[start..end]));
            start = end + 1;
        }
        self.buf.drain(..start);
        self.scanned = self.buf.len();
        if self.buf.len() > MAX_LINE {
            return self.overflow();
        }
        out
    }

    fn overflow(&mut self) -> Vec<String> {
        self.too_long = true;
        self.buf = Vec::new();
        self.scanned = 0;
        vec![]
    }

    /// True once a line exceeded `MAX_LINE`.
    pub fn too_long(&self) -> bool {
        self.too_long
    }

    /// The trailing partial line, if any.
    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        self.scanned = 0;
        let line = std::mem::take(&mut self.buf);
        Some(decode(&line))
    }
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// How a pumped body ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pumped {
    /// A terminal event (`Done`/`Error`) was forwarded, or the receiver is gone.
    Terminal,
    /// The body ended cleanly without a terminal event; `complete` tells whether
    /// `complete(line)` matched any line (e.g. a finish reason).
    Eof { complete: bool },
}

/// Reads a streaming HTTP body line by line, forwarding parsed events.
pub async fn pump(
    resp: reqwest::Response,
    parse: fn(&str) -> Vec<AgentEvent>,
    complete: fn(&str) -> bool,
    tx: &mpsc::Sender<AgentEvent>,
) -> Pumped {
    pump_with(resp, parse, complete, IDLE_TIMEOUT, tx).await
}

/// `pump` with a configurable per-chunk idle timeout.
pub async fn pump_with(
    resp: reqwest::Response,
    parse: fn(&str) -> Vec<AgentEvent>,
    complete: fn(&str) -> bool,
    idle: Duration,
    tx: &mpsc::Sender<AgentEvent>,
) -> Pumped {
    let mut buf = LineBuffer::default();
    let mut body = resp.bytes_stream();
    let mut saw_complete = false;
    loop {
        let chunk = match tokio::time::timeout(idle, body.next()).await {
            Err(_) => return fail(tx, "stream stalled: no data received".into()).await,
            Ok(None) => break,
            Ok(Some(c)) => c,
        };
        let lines = match chunk {
            Ok(c) => buf.push(&c),
            Err(e) => {
                return fail(tx, format!("stream interrupted: {}", e.without_url())).await;
            }
        };
        if buf.too_long() {
            return fail(tx, "stream line too long".into()).await;
        }
        saw_complete |= lines.iter().any(|l| complete(l));
        if forward(&lines, parse, tx).await {
            return Pumped::Terminal;
        }
    }
    let rest: Vec<String> = buf.finish().into_iter().collect();
    saw_complete |= rest.iter().any(|l| complete(l));
    if forward(&rest, parse, tx).await {
        return Pumped::Terminal;
    }
    Pumped::Eof {
        complete: saw_complete,
    }
}

async fn fail(tx: &mpsc::Sender<AgentEvent>, message: String) -> Pumped {
    let _ = tx.send(AgentEvent::Error { message }).await;
    Pumped::Terminal
}

async fn forward(
    lines: &[String],
    parse: fn(&str) -> Vec<AgentEvent>,
    tx: &mpsc::Sender<AgentEvent>,
) -> bool {
    for line in lines {
        for ev in parse(line) {
            let terminal = matches!(ev, AgentEvent::Done | AgentEvent::Error { .. });
            if tx.send(ev).await.is_err() || terminal {
                return true;
            }
        }
    }
    false
}
