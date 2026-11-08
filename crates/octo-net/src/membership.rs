//! Network membership: the list of paired nodes plus the 32-byte network key.
use crate::error::{NetError, Result};
use crate::identity::{NodeId, create_private};
use blake2::{Blake2s256, Digest};
use data_encoding::HEXLOWER;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const NETWORK_ID_DOMAIN: &[u8] = b"octo-net-id-v1";
/// Longest display name, in characters.
pub const MAX_NAME_CHARS: usize = 64;

/// A display name is at most 64 characters and has no control characters.
pub fn valid_display_name(s: &str) -> bool {
    s.chars().count() <= MAX_NAME_CHARS && !s.chars().any(char::is_control)
}

/// Shared secret of a network (used for 4b sync). Zeroized on drop; Debug is redacted.
#[derive(Clone)]
pub struct NetworkKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for NetworkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NetworkKey(..)")
    }
}

impl NetworkKey {
    pub fn generate() -> Self {
        let mut k = Zeroizing::new([0u8; 32]);
        rand::fill(&mut k[..]);
        Self(k)
    }

    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Symmetric key of the private space (synced between full-trust members only). Same shape as
/// the network key; a distinct type so the two are never mixed up.
#[derive(Clone)]
pub struct PrivateKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateKey(..)")
    }
}

impl PrivateKey {
    pub fn generate() -> Self {
        let mut k = Zeroizing::new([0u8; 32]);
        rand::fill(&mut k[..]);
        Self(k)
    }

    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(Zeroizing::new(b))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Per-member trust, a local policy of each node. `Full` members may use this node's private
/// space and hold the private key; `Public` members only reach public data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    Full,
    #[default]
    Public,
}

/// `BLAKE2s("octo-net-id-v1" || network_key)`. One-way: the relay learns the id, not the key.
pub fn network_id_of(k: &NetworkKey) -> [u8; 32] {
    let mut h = Blake2s256::new();
    h.update(NETWORK_ID_DOMAIN);
    h.update(k.as_bytes());
    h.finalize().into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub node_id: NodeId,
    #[serde(with = "hex32")]
    pub pubkey: [u8; 32],
    pub name: String,
    pub added_at: u64,
    #[serde(default)]
    pub trust: Trust,
    /// Local opinion of this node: may the member run CLI agents here. Never sent to or
    /// accepted from other nodes.
    #[serde(default)]
    pub agents: bool,
}

impl Member {
    pub fn new(pubkey: [u8; 32], name: &str) -> Self {
        let added_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            node_id: NodeId::from_pubkey(&pubkey),
            pubkey,
            name: name
                .chars()
                .filter(|c| !c.is_control())
                .take(MAX_NAME_CHARS)
                .collect(),
            added_at,
            trust: Trust::Public,
            agents: false,
        }
    }

    /// A member read from the wire is only valid if node_id really derives from pubkey.
    pub fn validate(&self) -> Result<()> {
        if NodeId::from_pubkey(&self.pubkey) != self.node_id || !valid_display_name(&self.name) {
            return Err(NetError::Malformed);
        }
        Ok(())
    }
}

mod hex32 {
    use data_encoding::HEXLOWER;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&HEXLOWER.encode(v))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let v = HEXLOWER
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    network_key: String,
    members: Vec<Member>,
    #[serde(default)]
    private_key: Option<String>,
    #[serde(default)]
    private_epoch: u64,
}

#[derive(Clone)]
pub struct Membership {
    path: Option<PathBuf>,
    key: NetworkKey,
    members: Vec<Member>,
    /// Private space key and its epoch; `None` on a node that is only public-trusted.
    private: Option<(u64, PrivateKey)>,
}

fn decode_key(s: &str) -> Result<[u8; 32]> {
    HEXLOWER
        .decode(s.as_bytes())
        .map_err(|_| NetError::Malformed)?
        .try_into()
        .map_err(|_| NetError::Malformed)
}

impl Membership {
    /// In-memory store (tests, joiners before persistence is wired).
    pub fn ephemeral(key: NetworkKey) -> Self {
        Self {
            path: None,
            key,
            members: Vec::new(),
            private: Some((1, PrivateKey::generate())),
        }
    }

    /// Loads `path` (JSON, mode 0600) or creates a fresh single-node network.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(b) => {
                let b = Zeroizing::new(b);
                let s: Stored = serde_json::from_slice(&b)?;
                let k = decode_key(&s.network_key)?;
                for m in &s.members {
                    m.validate()?;
                }
                let private = match &s.private_key {
                    Some(p) => Some((s.private_epoch, PrivateKey::from_bytes(decode_key(p)?))),
                    None => None,
                };
                Ok(Self {
                    path: Some(path.to_owned()),
                    key: NetworkKey::from_bytes(k),
                    members: s.members,
                    private,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let m = Self {
                    path: Some(path.to_owned()),
                    key: NetworkKey::generate(),
                    members: Vec::new(),
                    private: Some((1, PrivateKey::generate())),
                };
                m.write(&m.key, &m.members, m.private.as_ref())?;
                Ok(m)
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Persists the given state (not necessarily `self`'s), so callers can save first and only
    /// then assign: a failed save leaves memory untouched.
    fn write(
        &self,
        key: &NetworkKey,
        members: &[Member],
        private: Option<&(u64, PrivateKey)>,
    ) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let stored = Stored {
            network_key: HEXLOWER.encode(key.as_bytes()),
            members: members.to_vec(),
            private_key: private.map(|(_, k)| HEXLOWER.encode(k.as_bytes())),
            private_epoch: private.map_or(0, |(e, _)| *e),
        };
        let json = Zeroizing::new(serde_json::to_vec_pretty(&stored)?);
        // Write a private temp file, then rename over the target: atomic and never world-readable.
        let tmp = path.with_extension("tmp");
        let _ = std::fs::remove_file(&tmp);
        let mut f = create_private(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn network_key(&self) -> &NetworkKey {
        &self.key
    }

    /// Key of the private space; `None` when no full-trust member gave it to this node.
    pub fn private_key(&self) -> Option<&PrivateKey> {
        self.private.as_ref().map(|(_, k)| k)
    }

    /// Epoch of the private key (0 = none).
    pub fn private_epoch(&self) -> u64 {
        self.private.as_ref().map_or(0, |(e, _)| *e)
    }

    /// New random private key with the next epoch. `None` when this node holds no private key.
    pub fn rotate_private_key(&mut self) -> Result<Option<u64>> {
        let Some((e, _)) = &self.private else {
            return Ok(None);
        };
        let next = (e + 1, PrivateKey::generate());
        self.write(&self.key, &self.members, Some(&next))?;
        let e = next.0;
        self.private = Some(next);
        Ok(Some(e))
    }

    /// Takes a key received from a full-trust member if its epoch is newer than ours.
    pub fn adopt_private(&mut self, epoch: u64, key: &PrivateKey) -> Result<bool> {
        if epoch <= self.private_epoch() {
            return Ok(false);
        }
        let next = (epoch, key.clone());
        self.write(&self.key, &self.members, Some(&next))?;
        self.private = Some(next);
        Ok(true)
    }

    /// Changes a member's trust; false if there is no such member or nothing changed.
    pub fn set_trust(&mut self, id: &NodeId, trust: Trust) -> Result<bool> {
        let mut members = self.members.clone();
        let Some(m) = members.iter_mut().find(|m| &m.node_id == id) else {
            return Ok(false);
        };
        if m.trust == trust {
            return Ok(false);
        }
        m.trust = trust;
        self.write(&self.key, &members, self.private.as_ref())?;
        self.members = members;
        Ok(true)
    }

    /// Changes the local `agents` permission; false if no such member or nothing changed.
    pub fn set_agents(&mut self, id: &NodeId, agents: bool) -> Result<bool> {
        let mut members = self.members.clone();
        let Some(m) = members.iter_mut().find(|m| &m.node_id == id) else {
            return Ok(false);
        };
        if m.agents == agents {
            return Ok(false);
        }
        m.agents = agents;
        self.write(&self.key, &members, self.private.as_ref())?;
        self.members = members;
        Ok(true)
    }

    pub fn trust_of(&self, id: &NodeId) -> Option<Trust> {
        self.get(id).map(|m| m.trust)
    }

    pub fn network_id(&self) -> [u8; 32] {
        network_id_of(&self.key)
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn get(&self, id: &NodeId) -> Option<&Member> {
        self.members.iter().find(|m| &m.node_id == id)
    }

    /// Adds a member and persists. An existing node id is never overwritten: returns `false`.
    pub fn add(&mut self, m: Member) -> Result<bool> {
        if self.get(&m.node_id).is_some() {
            m.validate()?;
            return Ok(false);
        }
        self.put(m)?;
        Ok(true)
    }

    /// Adds or replaces by node id and persists (local decisions such as completing a pairing).
    pub fn put(&mut self, m: Member) -> Result<()> {
        m.validate()?;
        let mut members: Vec<Member> = self
            .members
            .iter()
            .filter(|x| x.node_id != m.node_id)
            .cloned()
            .collect();
        members.push(m);
        self.write(&self.key, &members, self.private.as_ref())?;
        self.members = members;
        Ok(())
    }

    pub fn remove(&mut self, id: &NodeId) -> Result<bool> {
        let members: Vec<Member> = self
            .members
            .iter()
            .filter(|m| &m.node_id != id)
            .cloned()
            .collect();
        if members.len() == self.members.len() {
            return Ok(false);
        }
        self.write(&self.key, &members, self.private.as_ref())?;
        self.members = members;
        Ok(true)
    }

    /// A copy that never writes to disk. Long protocol steps (pairing) run on it without holding
    /// the live membership, and their result is merged into the live one afterwards.
    pub fn detached(&self) -> Self {
        Self {
            path: None,
            ..self.clone()
        }
    }

    /// Takes network key, members and private key from `other` (a pairing run on a detached
    /// copy), but only while this membership is still empty. Returns whether it did.
    pub fn adopt_from(&mut self, other: &Membership) -> Result<bool> {
        if !self.members.is_empty() {
            return Ok(false);
        }
        self.adopt(
            other.key.clone(),
            other.members.clone(),
            other.private.clone(),
        )?;
        Ok(true)
    }

    /// Replaces key and members (a joining node adopting the host's network).
    pub(crate) fn adopt(
        &mut self,
        key: NetworkKey,
        members: Vec<Member>,
        private: Option<(u64, PrivateKey)>,
    ) -> Result<()> {
        self.write(&key, &members, private.as_ref())?;
        self.key = key;
        self.members = members;
        self.private = private;
        Ok(())
    }
}
