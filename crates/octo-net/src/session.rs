//! Per-peer Noise `IK` sessions (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) carried in relay payloads.
//!
//! Frame: `[version=1][kind][noise bytes]`, kind 1 = handshake msg1, 2 = msg2, 3 = transport.
//! The IK prologue is `"octo-ik-v1" || network_id`, so sessions never cross networks.
//! msg1 carries a big-endian u64 millisecond timestamp that must strictly increase per peer
//! (WireGuard-style) so a replayed msg1 cannot replace a live session; the last timestamp per
//! peer can be persisted (`with_store`) so this survives restarts.
//! Data frames use a stateless transport: `[1][3][nonce u64 BE][ciphertext]`, with a sender-side
//! strictly increasing counter and a 2048-bit sliding replay window on receive, so dropped or
//! reordered frames never desynchronise the session.
use crate::error::{NetError, Result};
use crate::identity::{NodeId, NodeKey};
use crate::membership::Membership;
use snow::{Builder, HandshakeState, StatelessTransportState};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

pub const IK_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
pub const WIRE_VERSION: u8 = 1;
const K_HS1: u8 = 1;
const K_HS2: u8 = 2;
const K_DATA: u8 = 3;
/// Largest plaintext per `encrypt` call; keeps the frame under the relay payload cap.
pub const MAX_PLAINTEXT: usize = 65_000;
const NOISE_MAX: usize = 65_535;

/// Consecutive AEAD failures from an established peer before we re-handshake.
pub const MAX_AEAD_FAILURES: u32 = 16;
const WINDOW_BITS: u64 = 2048;
const WINDOW_WORDS: usize = (WINDOW_BITS / 64) as usize;

/// WireGuard-style sliding replay window.
struct ReplayWindow {
    started: bool,
    top: u64,
    bits: [u64; WINDOW_WORDS],
}

impl ReplayWindow {
    fn new() -> Self {
        Self {
            started: false,
            top: 0,
            bits: [0; WINDOW_WORDS],
        }
    }

    fn slot(n: u64) -> (usize, u64) {
        let i = n % WINDOW_BITS;
        ((i / 64) as usize, 1u64 << (i % 64))
    }

    fn check(&self, n: u64) -> bool {
        if !self.started || n > self.top {
            return true;
        }
        if self.top - n >= WINDOW_BITS {
            return false;
        }
        let (w, m) = Self::slot(n);
        self.bits[w] & m == 0
    }

    /// Marks `n` as seen; call only after authentication succeeded.
    fn mark(&mut self, n: u64) {
        if !self.started {
            self.started = true;
            self.top = n;
            self.bits = [0; WINDOW_WORDS];
        } else if n > self.top {
            let gap = n - self.top;
            if gap >= WINDOW_BITS {
                self.bits = [0; WINDOW_WORDS];
            } else {
                for k in 1..=gap {
                    let (w, m) = Self::slot(self.top + k);
                    self.bits[w] &= !m;
                }
            }
            self.top = n;
        }
        let (w, m) = Self::slot(n);
        self.bits[w] |= m;
    }
}

struct Ready {
    t: StatelessTransportState,
    send: u64,
    recv: ReplayWindow,
    failures: u32,
}

enum Slot {
    Init(Box<HandshakeState>),
    Ready(Box<Ready>),
}

fn ready(hs: HandshakeState) -> Result<Slot> {
    Ok(Slot::Ready(Box::new(Ready {
        t: hs.into_stateless_transport_mode()?,
        send: 0,
        recv: ReplayWindow::new(),
        failures: 0,
    })))
}

#[derive(Debug)]
pub enum Handled {
    /// We were the responder: send this frame back; the session is established.
    Reply(Vec<u8>),
    /// We were the initiator and the session is now established.
    Established,
    Data(Vec<u8>),
    /// Simultaneous connect: we are the initiator by tie-break; the incoming msg1 was ignored.
    Ignored,
    /// Too many consecutive AEAD failures: the session was reset; send this msg1 to re-handshake.
    Rehandshake(Vec<u8>),
}

pub struct Sessions {
    key: NodeKey,
    slots: HashMap<NodeId, Slot>,
    last_ts_in: HashMap<NodeId, u64>,
    last_ts_out: u64,
    store: Option<PathBuf>,
}

fn prologue(m: &Membership) -> Vec<u8> {
    [b"octo-ik-v1".as_slice(), &m.network_id()].concat()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(body.len() + 2);
    f.push(WIRE_VERSION);
    f.push(kind);
    f.extend_from_slice(body);
    f
}

impl Sessions {
    pub fn new(key: NodeKey) -> Self {
        Self {
            key,
            slots: HashMap::new(),
            last_ts_in: HashMap::new(),
            last_ts_out: 0,
            store: None,
        }
    }

    /// Like `new`, but persists the per-peer msg1 timestamps in `path` (JSON, 0600) so a
    /// replayed msg1 is rejected after a restart.
    pub fn with_store(key: NodeKey, path: &std::path::Path) -> Result<Self> {
        let mut s = Self::new(key);
        match std::fs::read(path) {
            Ok(b) => {
                let m: HashMap<String, u64> = serde_json::from_slice(&b)?;
                for (k, v) in m {
                    s.last_ts_in.insert(NodeId::parse(&k)?, v);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        s.store = Some(path.to_owned());
        Ok(s)
    }

    fn persist(&self) -> Result<()> {
        let Some(path) = &self.store else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let m: HashMap<String, u64> = self
            .last_ts_in
            .iter()
            .map(|(k, v)| (k.to_string(), *v))
            .collect();
        let json = serde_json::to_vec(&m)?;
        let tmp = path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = crate::identity::create_private(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn is_established(&self, peer: &NodeId) -> bool {
        matches!(self.slots.get(peer), Some(Slot::Ready(_)))
    }

    /// Forget the session (e.g. after a decrypt error or unpairing).
    pub fn reset(&mut self, peer: &NodeId) {
        self.slots.remove(peer);
    }

    /// Starts a handshake to a member; returns the frame to send.
    pub fn connect(&mut self, m: &Membership, peer: NodeId) -> Result<Vec<u8>> {
        let member = m.get(&peer).ok_or(NetError::NotMember)?;
        let pro = prologue(m);
        let mut hs = Builder::new(IK_PARAMS.parse()?)
            .local_private_key(self.key.secret())?
            .remote_public_key(&member.pubkey)?
            .prologue(&pro)?
            .build_initiator()?;
        self.last_ts_out = now_ms().max(self.last_ts_out + 1);
        let mut buf = vec![0u8; NOISE_MAX];
        let n = hs.write_message(&self.last_ts_out.to_be_bytes(), &mut buf)?;
        self.slots.insert(peer, Slot::Init(Box::new(hs)));
        Ok(frame(K_HS1, &buf[..n]))
    }

    /// Processes a frame received from `from` (the relay-reported sender; untrusted until
    /// the handshake / AEAD check passes).
    pub fn handle(&mut self, m: &Membership, from: NodeId, data: &[u8]) -> Result<Handled> {
        let [ver, kind, body @ ..] = data else {
            return Err(NetError::Malformed);
        };
        if *ver != WIRE_VERSION {
            return Err(NetError::Version(*ver));
        }
        match *kind {
            K_HS1 => self.on_hs1(m, from, body),
            K_HS2 => self.on_hs2(from, body),
            K_DATA => self.on_data(m, from, body),
            _ => Err(NetError::Malformed),
        }
    }

    fn on_hs1(&mut self, m: &Membership, from: NodeId, body: &[u8]) -> Result<Handled> {
        let member = m.get(&from).ok_or(NetError::NotMember)?;
        // Simultaneous connect: the smaller node id stays initiator, the other one yields.
        if matches!(self.slots.get(&from), Some(Slot::Init(_))) && self.key.node_id() < from {
            return Ok(Handled::Ignored);
        }
        let pro = prologue(m);
        let mut hs = Builder::new(IK_PARAMS.parse()?)
            .local_private_key(self.key.secret())?
            .prologue(&pro)?
            .build_responder()?;
        let mut pl = vec![0u8; NOISE_MAX];
        let n = hs.read_message(body, &mut pl)?;
        let ts = u64::from_be_bytes(pl[..n].try_into().map_err(|_| NetError::Malformed)?);
        // The static key that actually authenticated must be the member we think `from` is.
        if hs.get_remote_static() != Some(member.pubkey.as_slice()) {
            return Err(NetError::NotMember);
        }
        if self.last_ts_in.get(&from).is_some_and(|last| ts <= *last) {
            return Err(NetError::Replay);
        }
        let mut out = vec![0u8; NOISE_MAX];
        let n = hs.write_message(&[], &mut out)?;
        let prev = self.last_ts_in.insert(from, ts);
        if let Err(e) = self.persist() {
            match prev {
                Some(p) => self.last_ts_in.insert(from, p),
                None => self.last_ts_in.remove(&from),
            };
            return Err(e);
        }
        self.slots.insert(from, ready(hs)?);
        Ok(Handled::Reply(frame(K_HS2, &out[..n])))
    }

    fn on_hs2(&mut self, from: NodeId, body: &[u8]) -> Result<Handled> {
        let Some(Slot::Init(mut hs)) = self.slots.remove(&from) else {
            return Err(NetError::NoSession);
        };
        let mut pl = vec![0u8; NOISE_MAX];
        hs.read_message(body, &mut pl)?;
        self.slots.insert(from, ready(*hs)?);
        Ok(Handled::Established)
    }

    fn on_data(&mut self, m: &Membership, from: NodeId, body: &[u8]) -> Result<Handled> {
        let Some(Slot::Ready(r)) = self.slots.get_mut(&from) else {
            return Err(NetError::NoSession);
        };
        let (nb, ct) = body.split_first_chunk::<8>().ok_or(NetError::Malformed)?;
        let nonce = u64::from_be_bytes(*nb);
        if nonce == u64::MAX || !r.recv.check(nonce) {
            return Err(NetError::Replay);
        }
        let mut pl = vec![0u8; NOISE_MAX];
        match r.t.read_message(nonce, ct, &mut pl) {
            Ok(n) => {
                r.recv.mark(nonce);
                r.failures = 0;
                pl.truncate(n);
                Ok(Handled::Data(pl))
            }
            Err(e) => {
                r.failures += 1;
                if r.failures >= MAX_AEAD_FAILURES {
                    self.slots.remove(&from);
                    return Ok(Handled::Rehandshake(self.connect(m, from)?));
                }
                Err(e.into())
            }
        }
    }

    pub fn encrypt(&mut self, peer: NodeId, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(NetError::TooLarge);
        }
        let Some(Slot::Ready(r)) = self.slots.get_mut(&peer) else {
            return Err(NetError::NoSession);
        };
        if r.send == u64::MAX {
            return Err(NetError::Closed);
        }
        let mut out = vec![0u8; plaintext.len() + 16];
        let n = r.t.write_message(r.send, plaintext, &mut out)?;
        let mut body = Vec::with_capacity(n + 8);
        body.extend_from_slice(&r.send.to_be_bytes());
        body.extend_from_slice(&out[..n]);
        r.send += 1;
        Ok(frame(K_DATA, &body))
    }
}
