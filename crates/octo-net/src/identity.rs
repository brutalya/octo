//! Node identity: static X25519 key (Noise) and the node id derived from its public half.
use crate::error::{NetError, Result};
use blake2::{Blake2s256, Digest};
use data_encoding::BASE32_NOPAD;
use std::fmt;
use std::io::Write;
use std::path::Path;
use zeroize::Zeroizing;

pub const NODE_ID_LEN: usize = 16;

/// First 16 bytes of BLAKE2s(pubkey).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId([u8; NODE_ID_LEN]);

impl NodeId {
    pub fn from_pubkey(pk: &[u8; 32]) -> Self {
        let h = Blake2s256::digest(pk);
        let mut b = [0u8; NODE_ID_LEN];
        b.copy_from_slice(&h[..NODE_ID_LEN]);
        Self(b)
    }

    pub fn from_bytes(b: [u8; NODE_ID_LEN]) -> Self {
        Self(b)
    }

    pub fn as_bytes(&self) -> &[u8; NODE_ID_LEN] {
        &self.0
    }

    pub fn parse(s: &str) -> Result<Self> {
        let v = BASE32_NOPAD
            .decode(s.to_ascii_uppercase().as_bytes())
            .map_err(|_| NetError::Malformed)?;
        Ok(Self(v.try_into().map_err(|_| NetError::Malformed)?))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&BASE32_NOPAD.encode(&self.0).to_ascii_lowercase())
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({self})")
    }
}

impl serde::Serialize for NodeId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for NodeId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        NodeId::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// Static X25519 keypair. The secret is zeroized on drop and never printed.
#[derive(Clone)]
pub struct NodeKey {
    secret: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}

impl fmt::Debug for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeKey({})", self.node_id())
    }
}

impl NodeKey {
    pub fn generate() -> Self {
        let mut s = Zeroizing::new([0u8; 32]);
        rand::fill(&mut s[..]);
        Self::from_secret(s)
    }

    fn from_secret(secret: Zeroizing<[u8; 32]>) -> Self {
        let public = x25519_dalek::x25519(*secret, x25519_dalek::X25519_BASEPOINT_BYTES);
        Self { secret, public }
    }

    /// Reads the key file or creates it (mode 0600, 32 random bytes). A file of the
    /// wrong size is an error and is never overwritten.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(b) => return Self::from_file_bytes(Zeroizing::new(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let key = Self::generate();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        match create_private(path) {
            Ok(mut f) => {
                f.write_all(&key.secret[..])?;
                f.sync_all()?;
                Ok(key)
            }
            // Lost a creation race: use the winner's key.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Self::from_file_bytes(Zeroizing::new(std::fs::read(path)?))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn from_file_bytes(b: Zeroizing<Vec<u8>>) -> Result<Self> {
        let arr: [u8; 32] = b.as_slice().try_into().map_err(|_| NetError::BadKeyFile)?;
        Ok(Self::from_secret(Zeroizing::new(arr)))
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    pub fn node_id(&self) -> NodeId {
        NodeId::from_pubkey(&self.public)
    }

    pub(crate) fn secret(&self) -> &[u8; 32] {
        &self.secret
    }
}

/// Creates a new file readable only by the owner (0600 on unix).
pub(crate) fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}
