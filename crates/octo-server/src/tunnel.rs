//! Wire format of a tunneled HTTP exchange on one mux stream.
//!
//! Request:  `u32 BE len || JSON ReqHead || body bytes... || end`.
//! Response: `u32 BE len || JSON RespHead || body bytes... || end`.
use octo_net::mux::Stream;
use octo_net::{NetError, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const MAX_HEAD: usize = 16 * 1024;

#[derive(Serialize, Deserialize)]
pub struct ReqHead {
    /// HTTP method, or `PING` for a liveness probe.
    pub method: String,
    /// Path and query, e.g. `/api/threads?x=1`.
    pub uri: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize)]
pub struct RespHead {
    pub status: u16,
    pub headers: Vec<(String, String)>,
}

pub struct Framed {
    s: Stream,
    buf: Vec<u8>,
}

impl Framed {
    pub fn new(s: Stream) -> Self {
        Self { s, buf: Vec::new() }
    }

    pub async fn send_head<T: Serialize>(&self, h: &T) -> Result<()> {
        let j = serde_json::to_vec(h)?;
        if j.len() > MAX_HEAD {
            return Err(NetError::TooLarge);
        }
        let mut b = (j.len() as u32).to_be_bytes().to_vec();
        b.extend_from_slice(&j);
        self.s.write(&b).await
    }

    pub async fn read_head<T: DeserializeOwned>(&mut self) -> Result<T> {
        loop {
            if let Some((len, _)) = self.buf.split_first_chunk::<4>() {
                let len = u32::from_be_bytes(*len) as usize;
                if len > MAX_HEAD {
                    return Err(NetError::Malformed);
                }
                if self.buf.len() >= 4 + len {
                    let h = serde_json::from_slice(&self.buf[4..4 + len])?;
                    self.buf.drain(..4 + len);
                    return Ok(h);
                }
            }
            match self.s.read().await? {
                Some(c) => self.buf.extend_from_slice(&c),
                None => return Err(NetError::Closed),
            }
        }
    }

    /// Next body chunk; `None` at the end of the stream.
    pub async fn next_body(&mut self) -> Result<Option<Vec<u8>>> {
        if !self.buf.is_empty() {
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        self.s.read().await
    }

    pub async fn write(&self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.s.write(data).await
    }

    pub async fn finish(&self) -> Result<()> {
        self.s.finish().await
    }

    pub async fn reset(&self) {
        self.s.reset().await
    }
}
