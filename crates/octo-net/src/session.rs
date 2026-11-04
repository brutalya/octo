//! Per-peer Noise `IK` sessions (`Noise_IK_25519_ChaChaPoly_BLAKE2s`) carried in relay payloads.
//!
//! Frame: `[version=1][kind][noise bytes]`, kind 1 = handshake msg1, 2 = msg2, 3 = transport.
//! The IK prologue is `"octo-ik-v1" || network_id`, so sessions never cross networks.
//! msg1 carries a big-endian u64 millisecond timestamp that must strictly increase per peer
//! (WireGuard-style) so a replayed msg1 cannot replace a live session.
use crate::error::{NetError, Result};
use crate::identity::{NodeId, NodeKey};
use crate::membership::Membership;
use snow::{Builder, HandshakeState, TransportState};
use std::collections::HashMap;

pub const IK_PARAMS: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
pub const WIRE_VERSION: u8 = 1;
const K_HS1: u8 = 1;
const K_HS2: u8 = 2;
const K_DATA: u8 = 3;
/// Largest plaintext per `encrypt` call; keeps the frame under the relay payload cap.
pub const MAX_PLAINTEXT: usize = 65_000;
const NOISE_MAX: usize = 65_535;

enum Slot {
    Init(Box<HandshakeState>),
    Ready(Box<TransportState>),
}

#[derive(Debug)]
pub enum Handled {
    /// We were the responder: send this frame back; the session is established.
    Reply(Vec<u8>),
    /// We were the initiator and the session is now established.
    Established,
    Data(Vec<u8>),
}

pub struct Sessions {
    key: NodeKey,
    slots: HashMap<NodeId, Slot>,
    last_ts_in: HashMap<NodeId, u64>,
    last_ts_out: u64,
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
        }
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
            K_DATA => self.on_data(from, body),
            _ => Err(NetError::Malformed),
        }
    }

    fn on_hs1(&mut self, m: &Membership, from: NodeId, body: &[u8]) -> Result<Handled> {
        let member = m.get(&from).ok_or(NetError::NotMember)?;
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
        self.last_ts_in.insert(from, ts);
        self.slots
            .insert(from, Slot::Ready(Box::new(hs.into_transport_mode()?)));
        Ok(Handled::Reply(frame(K_HS2, &out[..n])))
    }

    fn on_hs2(&mut self, from: NodeId, body: &[u8]) -> Result<Handled> {
        let Some(Slot::Init(mut hs)) = self.slots.remove(&from) else {
            return Err(NetError::NoSession);
        };
        let mut pl = vec![0u8; NOISE_MAX];
        hs.read_message(body, &mut pl)?;
        self.slots
            .insert(from, Slot::Ready(Box::new(hs.into_transport_mode()?)));
        Ok(Handled::Established)
    }

    fn on_data(&mut self, from: NodeId, body: &[u8]) -> Result<Handled> {
        let Some(Slot::Ready(t)) = self.slots.get_mut(&from) else {
            return Err(NetError::NoSession);
        };
        let mut pl = vec![0u8; NOISE_MAX];
        let n = t.read_message(body, &mut pl)?;
        pl.truncate(n);
        Ok(Handled::Data(pl))
    }

    pub fn encrypt(&mut self, peer: NodeId, plaintext: &[u8]) -> Result<Vec<u8>> {
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(NetError::TooLarge);
        }
        let Some(Slot::Ready(t)) = self.slots.get_mut(&peer) else {
            return Err(NetError::NoSession);
        };
        let mut out = vec![0u8; plaintext.len() + 16];
        let n = t.write_message(plaintext, &mut out)?;
        Ok(frame(K_DATA, &out[..n]))
    }
}
