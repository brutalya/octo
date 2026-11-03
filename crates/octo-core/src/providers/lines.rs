/// Splits a byte stream into lines. Bytes are buffered so UTF-8 sequences and lines
/// split across network chunks stay intact.
#[derive(Default)]
pub struct LineBuffer {
    buf: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(chunk);
        let mut out = vec![];
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            out.push(decode(&line[..line.len() - 1]));
        }
        out
    }

    /// The trailing partial line, if any.
    pub fn finish(&mut self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let line = std::mem::take(&mut self.buf);
        Some(decode(&line))
    }
}

fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

use super::AgentEvent;
use futures_util::StreamExt;
use tokio::sync::mpsc;

/// Reads a streaming HTTP body line by line, forwarding parsed events.
/// Returns true once a terminal event (`Done`/`Error`) was forwarded.
pub async fn pump(
    resp: reqwest::Response,
    parse: fn(&str) -> Vec<AgentEvent>,
    tx: &mpsc::Sender<AgentEvent>,
) -> bool {
    let mut buf = LineBuffer::default();
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let lines = match chunk {
            Ok(c) => buf.push(&c),
            Err(e) => {
                let message = format!("stream interrupted: {}", e.without_url());
                let _ = tx.send(AgentEvent::Error { message }).await;
                return true;
            }
        };
        if forward(&lines, parse, tx).await {
            return true;
        }
    }
    let rest: Vec<String> = buf.finish().into_iter().collect();
    forward(&rest, parse, tx).await
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
