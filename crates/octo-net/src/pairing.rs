//! Pairing a new node into a network.
//!
//! Host shows a code `relay_url#token`. `token` = base32(host_node_id(16) || secret(16)).
//! Both sides join the relay under the throwaway network id `BLAKE2s("octo-pair-net-v1"||secret)`.
//! Joiner sends Noise `XX` (`Noise_XX_25519_ChaChaPoly_BLAKE2s`, prologue
//! `"octo-pair-v1" || secret`) to the host; msg3 payload is the joiner's display name.
//! SAS = 6 digits from `BLAKE2s("octo-sas-v1" || handshake_hash) mod 10^6`. Both users compare it;
//! the joiner sends `confirm`, then the host user accepts and sends `welcome` (members + network key).
//! Frames: `[1][kind][bytes]`, kind 0x21 msg1, 0x22 msg2, 0x23 msg3, 0x24 transport (JSON).
use crate::error::{NetError, Result};
use crate::identity::{NodeId, NodeKey};
use crate::membership::{Member, Membership, NetworkKey};
use crate::relay_client::{ERR_PEER_UNAVAILABLE, Event, RelayClientConfig, RelayHandle};
use crate::session::WIRE_VERSION;
use blake2::{Blake2s256, Digest};
use data_encoding::{BASE32_NOPAD, HEXLOWER};
use serde::{Deserialize, Serialize};
use snow::{Builder, HandshakeState, TransportState};
use std::fmt;
use std::time::Duration;
use tokio::time::Instant;
use zeroize::Zeroizing;

pub const XX_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
/// Validity of a pairing code.
pub const PAIR_TTL: Duration = Duration::from_secs(300);
const K_HS1: u8 = 0x21;
const K_HS2: u8 = 0x22;
const K_HS3: u8 = 0x23;
const K_DATA: u8 = 0x24;
const BUF: usize = 65_535;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PairingToken {
    pub host: NodeId,
    secret: [u8; 16],
}

impl PairingToken {
    pub fn generate(host: NodeId) -> Self {
        let mut secret = [0u8; 16];
        rand::fill(&mut secret[..]);
        Self { host, secret }
    }

    fn network_id(&self) -> [u8; 32] {
        let mut h = Blake2s256::new();
        h.update(b"octo-pair-net-v1");
        h.update(self.secret);
        h.finalize().into()
    }
}

impl fmt::Display for PairingToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let raw = [self.host.as_bytes().as_slice(), &self.secret].concat();
        f.write_str(&BASE32_NOPAD.encode(&raw).to_ascii_lowercase())
    }
}

impl PairingToken {
    fn parse(s: &str) -> Result<Self> {
        let raw = BASE32_NOPAD
            .decode(s.to_ascii_uppercase().as_bytes())
            .map_err(|_| NetError::Malformed)?;
        if raw.len() != 32 {
            return Err(NetError::Malformed);
        }
        let host = NodeId::from_bytes(raw[..16].try_into().map_err(|_| NetError::Malformed)?);
        let secret = raw[16..].try_into().map_err(|_| NetError::Malformed)?;
        Ok(Self { host, secret })
    }
}

#[derive(Clone, Debug)]
pub struct PairingCode {
    pub relay_url: String,
    pub token: PairingToken,
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.relay_url, self.token)
    }
}

impl PairingCode {
    pub fn parse(s: &str) -> Result<Self> {
        let (url, tok) = s.trim().rsplit_once('#').ok_or(NetError::Malformed)?;
        Ok(Self {
            relay_url: url.to_owned(),
            token: PairingToken::parse(tok)?,
        })
    }
}

/// Noise XX state; `secret` goes into the prologue so only holders of the code can complete it.
pub fn xx_state(initiator: bool, key: &NodeKey, secret: &[u8; 16]) -> Result<HandshakeState> {
    let pro = [b"octo-pair-v1".as_slice(), secret].concat();
    let b = Builder::new(XX_PARAMS.parse()?)
        .local_private_key(key.secret())?
        .prologue(&pro)?;
    Ok(if initiator {
        b.build_initiator()?
    } else {
        b.build_responder()?
    })
}

/// 6-digit short authentication string from the Noise handshake hash.
pub fn sas_from_hash(h: &[u8]) -> String {
    let mut d = Blake2s256::new();
    d.update(b"octo-sas-v1");
    d.update(h);
    let r = d.finalize();
    let n = u32::from_be_bytes([r[0], r[1], r[2], r[3]]) % 1_000_000;
    format!("{n:06}")
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum PairMsg {
    Confirm,
    Abort,
    Welcome {
        host: Member,
        members: Vec<Member>,
        network_key: String,
    },
}

fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut f = vec![WIRE_VERSION, kind];
    f.extend_from_slice(body);
    f
}

fn seal(t: &mut TransportState, m: &PairMsg) -> Result<Vec<u8>> {
    let json = Zeroizing::new(serde_json::to_vec(m)?);
    if json.len() > BUF - 16 {
        return Err(NetError::TooLarge);
    }
    let mut out = vec![0u8; json.len() + 16];
    let n = t.write_message(&json, &mut out)?;
    Ok(frame(K_DATA, &out[..n]))
}

fn open(t: &mut TransportState, body: &[u8]) -> Result<PairMsg> {
    let mut pl = Zeroizing::new(vec![0u8; BUF]);
    let n = t.read_message(body, &mut pl)?;
    Ok(serde_json::from_slice(&pl[..n])?)
}

fn relay_cfg(url: &str, key: &NodeKey, token: &PairingToken) -> RelayClientConfig {
    RelayClientConfig {
        url: url.to_owned(),
        key: key.clone(),
        network_id: token.network_id(),
        backoff_base: Duration::from_millis(250),
        backoff_max: Duration::from_secs(5),
    }
}

/// Next pairing frame from `peer` (or from anyone when `peer` is None) before `deadline`.
async fn next_frame(
    relay: &mut RelayHandle,
    peer: Option<NodeId>,
    deadline: Instant,
    fail_on_unavailable: bool,
) -> Result<(NodeId, u8, Vec<u8>)> {
    loop {
        let ev = tokio::time::timeout_at(deadline, relay.recv())
            .await
            .map_err(|_| NetError::Timeout)?
            .ok_or(NetError::Closed)?;
        match ev {
            Event::Frame { from, payload } if peer.is_none_or(|p| p == from) => {
                if let [v, kind, body @ ..] = payload.as_slice()
                    && *v == WIRE_VERSION
                {
                    return Ok((from, *kind, body.to_vec()));
                }
            }
            Event::RelayError(ERR_PEER_UNAVAILABLE) if fail_on_unavailable => {
                return Err(NetError::Pairing("peer unavailable".into()));
            }
            _ => {}
        }
    }
}

async fn recv_msg(
    relay: &mut RelayHandle,
    peer: NodeId,
    deadline: Instant,
    t: &mut TransportState,
) -> Result<PairMsg> {
    loop {
        let (_, kind, body) = next_frame(relay, Some(peer), deadline, false).await?;
        if kind == K_DATA {
            return open(t, &body);
        }
    }
}

// ---------------------------------------------------------------- host

pub struct PairHost {
    relay: Option<RelayHandle>,
    key: NodeKey,
    name: String,
    code: PairingCode,
    deadline: Instant,
}

pub struct HostPending {
    relay: RelayHandle,
    t: TransportState,
    host_key: NodeKey,
    host_name: String,
    peer: NodeId,
    peer_pub: [u8; 32],
    peer_name: String,
    sas: String,
    deadline: Instant,
}

impl PairHost {
    /// Registers a one-time code on the relay and waits until the relay link is up.
    pub async fn start(relay_url: &str, key: &NodeKey, name: &str, ttl: Duration) -> Result<Self> {
        let token = PairingToken::generate(key.node_id());
        let relay = RelayHandle::connect(relay_cfg(relay_url, key, &token));
        relay.wait_online(Duration::from_secs(10)).await?;
        Ok(Self {
            relay: Some(relay),
            key: key.clone(),
            name: name.to_owned(),
            code: PairingCode {
                relay_url: relay_url.to_owned(),
                token,
            },
            deadline: Instant::now() + ttl,
        })
    }

    pub fn code(&self) -> PairingCode {
        self.code.clone()
    }

    /// Waits for a joiner that completes the handshake with the right code. A failed handshake
    /// (wrong secret) does not consume the code; a successful one does (this consumes `relay`).
    pub async fn wait_peer(&mut self) -> Result<HostPending> {
        let mut relay = self
            .relay
            .take()
            .ok_or_else(|| NetError::Pairing("code already used".into()))?;
        let mut attempt: Option<(NodeId, HandshakeState)> = None;
        let mut buf = vec![0u8; BUF];
        loop {
            let (from, kind, body) = next_frame(&mut relay, None, self.deadline, false)
                .await
                .map_err(|e| {
                    if matches!(e, NetError::Timeout) {
                        NetError::Pairing("code expired".into())
                    } else {
                        e
                    }
                })?;
            match kind {
                K_HS1 => {
                    let mut hs = xx_state(false, &self.key, &self.code.token.secret)?;
                    if hs.read_message(&body, &mut buf).is_err() {
                        continue;
                    }
                    let n = hs.write_message(&[], &mut buf)?;
                    let _ = relay.send(from, frame(K_HS2, &buf[..n])).await;
                    attempt = Some((from, hs));
                }
                K_HS3 => {
                    let Some((p, mut hs)) = attempt.take() else {
                        continue;
                    };
                    if p != from {
                        attempt = Some((p, hs));
                        continue;
                    }
                    let n = match hs.read_message(&body, &mut buf) {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("hs3 err {e}");
                            continue;
                        }
                    };
                    let Ok(peer_name) = String::from_utf8(buf[..n].to_vec()) else {
                        continue;
                    };
                    let Some(Ok(peer_pub)) = hs.get_remote_static().map(<[u8; 32]>::try_from)
                    else {
                        continue;
                    };
                    if NodeId::from_pubkey(&peer_pub) != from || peer_name.len() > 64 {
                        continue;
                    }
                    let sas = sas_from_hash(hs.get_handshake_hash());
                    return Ok(HostPending {
                        relay,
                        t: hs.into_transport_mode()?,
                        host_key: self.key.clone(),
                        host_name: self.name.clone(),
                        peer: from,
                        peer_pub,
                        peer_name,
                        sas,
                        deadline: self.deadline,
                    });
                }
                _ => {}
            }
        }
    }
}

impl HostPending {
    pub fn sas(&self) -> &str {
        &self.sas
    }
    pub fn peer_name(&self) -> &str {
        &self.peer_name
    }
    pub fn peer_node_id(&self) -> NodeId {
        self.peer
    }

    /// Host user's decision. Waits for the joiner's own confirmation first, so the network key
    /// is only ever sent when both humans approved the same SAS.
    pub async fn finish(mut self, accept: bool, membership: &mut Membership) -> Result<Member> {
        match recv_msg(&mut self.relay, self.peer, self.deadline, &mut self.t).await? {
            PairMsg::Confirm => {}
            _ => return Err(NetError::Pairing("peer aborted".into())),
        }
        if !accept {
            let f = seal(&mut self.t, &PairMsg::Abort)?;
            let _ = self.relay.send(self.peer, f).await;
            self.relay.shutdown().await;
            return Err(NetError::Pairing("rejected".into()));
        }
        let member = Member::new(self.peer_pub, &self.peer_name);
        let welcome = PairMsg::Welcome {
            host: Member::new(self.host_key.public(), &self.host_name),
            members: membership
                .members()
                .iter()
                .filter(|m| m.node_id != member.node_id)
                .cloned()
                .collect(),
            network_key: HEXLOWER.encode(membership.network_key().as_bytes()),
        };
        membership.add(member.clone())?;
        let f = seal(&mut self.t, &welcome)?;
        self.relay.send(self.peer, f).await?;
        self.relay.shutdown().await;
        Ok(member)
    }
}

// ---------------------------------------------------------------- joiner

pub struct JoinPending {
    relay: RelayHandle,
    t: TransportState,
    host: NodeId,
    host_pub: [u8; 32],
    own: NodeId,
    sas: String,
    deadline: Instant,
}

/// Joiner side: connects, runs the handshake and returns once the SAS is available.
pub async fn join(
    code: &PairingCode,
    key: &NodeKey,
    name: &str,
    ttl: Duration,
) -> Result<JoinPending> {
    if name.len() > 64 {
        return Err(NetError::TooLarge);
    }
    let deadline = Instant::now() + ttl;
    let host = code.token.host;
    let mut relay = RelayHandle::connect(relay_cfg(&code.relay_url, key, &code.token));
    tokio::time::timeout_at(deadline, relay.wait_online(ttl))
        .await
        .map_err(|_| NetError::Timeout)??;
    let mut hs = xx_state(true, key, &code.token.secret)?;
    let mut buf = vec![0u8; BUF];
    let n = hs.write_message(&[], &mut buf)?;
    relay.send(host, frame(K_HS1, &buf[..n])).await?;
    let body = loop {
        let (_, kind, body) = next_frame(&mut relay, Some(host), deadline, true).await?;
        if kind == K_HS2 {
            break body;
        }
    };
    hs.read_message(&body, &mut buf)
        .map_err(|_| NetError::Pairing("handshake failed (wrong code?)".into()))?;
    let n = hs.write_message(name.as_bytes(), &mut buf)?;
    relay.send(host, frame(K_HS3, &buf[..n])).await?;
    let host_pub: [u8; 32] = hs
        .get_remote_static()
        .and_then(|k| k.try_into().ok())
        .ok_or(NetError::Malformed)?;
    if NodeId::from_pubkey(&host_pub) != host {
        return Err(NetError::Pairing("host key does not match code".into()));
    }
    let sas = sas_from_hash(hs.get_handshake_hash());
    Ok(JoinPending {
        relay,
        t: hs.into_transport_mode()?,
        host,
        host_pub,
        own: key.node_id(),
        sas,
        deadline,
    })
}

impl JoinPending {
    pub fn sas(&self) -> &str {
        &self.sas
    }

    /// Joiner user's decision; on accept waits for the host's welcome and adopts the network.
    /// Refuses (and aborts) if this node already belongs to a network with members.
    pub async fn finish(mut self, accept: bool, membership: &mut Membership) -> Result<()> {
        if !accept || !membership.members().is_empty() {
            let f = seal(&mut self.t, &PairMsg::Abort)?;
            let _ = self.relay.send(self.host, f).await;
            self.relay.shutdown().await;
            return Err(NetError::Pairing(if accept {
                "node already paired".into()
            } else {
                "rejected".into()
            }));
        }
        let f = seal(&mut self.t, &PairMsg::Confirm)?;
        self.relay.send(self.host, f).await?;
        match recv_msg(&mut self.relay, self.host, self.deadline, &mut self.t).await? {
            PairMsg::Welcome {
                host,
                mut members,
                network_key,
            } => {
                host.validate()?;
                if host.pubkey != self.host_pub {
                    return Err(NetError::Pairing("welcome host key mismatch".into()));
                }
                let key: [u8; 32] = HEXLOWER
                    .decode(network_key.as_bytes())
                    .map_err(|_| NetError::Malformed)?
                    .try_into()
                    .map_err(|_| NetError::Malformed)?;
                for m in &members {
                    m.validate()?;
                }
                members.retain(|m| m.node_id != self.own && m.node_id != host.node_id);
                members.push(host);
                membership.adopt(NetworkKey::from_bytes(key), members)?;
                self.relay.shutdown().await;
                Ok(())
            }
            _ => Err(NetError::Pairing("host rejected".into())),
        }
    }
}

// ---------------------------------------------------------------- membership broadcast

/// Plaintext for the host to send (over an established IK session) to every other member.
pub fn announcement(m: &Member) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"t": "member_added", "member": m})).unwrap_or_default()
}

/// Applies an announcement received over an IK session from `from`. Only members may announce.
pub fn apply_announcement(
    membership: &mut Membership,
    from: &NodeId,
    plaintext: &[u8],
) -> Result<()> {
    if membership.get(from).is_none() {
        return Err(NetError::NotMember);
    }
    #[derive(Deserialize)]
    struct A {
        t: String,
        member: Member,
    }
    let a: A = serde_json::from_slice(plaintext)?;
    if a.t != "member_added" {
        return Err(NetError::Malformed);
    }
    membership.add(a.member)
}
