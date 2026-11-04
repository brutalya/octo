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
            name: name.chars().take(64).collect(),
            added_at,
        }
    }

    /// A member read from the wire is only valid if node_id really derives from pubkey.
    pub fn validate(&self) -> Result<()> {
        if NodeId::from_pubkey(&self.pubkey) != self.node_id {
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
}

pub struct Membership {
    path: Option<PathBuf>,
    key: NetworkKey,
    members: Vec<Member>,
}

impl Membership {
    /// In-memory store (tests, joiners before persistence is wired).
    pub fn ephemeral(key: NetworkKey) -> Self {
        Self {
            path: None,
            key,
            members: Vec::new(),
        }
    }

    /// Loads `path` (JSON, mode 0600) or creates a fresh single-node network.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(b) => {
                let b = Zeroizing::new(b);
                let s: Stored = serde_json::from_slice(&b)?;
                let k: [u8; 32] = HEXLOWER
                    .decode(s.network_key.as_bytes())
                    .map_err(|_| NetError::Malformed)?
                    .try_into()
                    .map_err(|_| NetError::Malformed)?;
                for m in &s.members {
                    m.validate()?;
                }
                Ok(Self {
                    path: Some(path.to_owned()),
                    key: NetworkKey::from_bytes(k),
                    members: s.members,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let m = Self {
                    path: Some(path.to_owned()),
                    key: NetworkKey::generate(),
                    members: Vec::new(),
                };
                m.save()?;
                Ok(m)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let stored = Stored {
            network_key: HEXLOWER.encode(self.key.as_bytes()),
            members: self.members.clone(),
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

    pub fn network_id(&self) -> [u8; 32] {
        network_id_of(&self.key)
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn get(&self, id: &NodeId) -> Option<&Member> {
        self.members.iter().find(|m| &m.node_id == id)
    }

    /// Adds (or replaces by node id) a member and persists.
    pub fn add(&mut self, m: Member) -> Result<()> {
        m.validate()?;
        self.members.retain(|x| x.node_id != m.node_id);
        self.members.push(m);
        self.save()
    }

    pub fn remove(&mut self, id: &NodeId) -> Result<bool> {
        let before = self.members.len();
        self.members.retain(|m| &m.node_id != id);
        let changed = self.members.len() != before;
        if changed {
            self.save()?;
        }
        Ok(changed)
    }

    /// Replaces key and members (a joining node adopting the host's network).
    pub(crate) fn adopt(&mut self, key: NetworkKey, members: Vec<Member>) -> Result<()> {
        self.key = key;
        self.members = members;
        self.save()
    }
}
