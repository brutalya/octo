use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::io::Write;
use std::path::Path;
use zeroize::Zeroize;

const NONCE_LEN: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("decryption failed (wrong key or corrupted data)")]
    Decrypt,
    #[error("encryption failed")]
    Encrypt,
    #[error("key file must be exactly 32 bytes")]
    BadKeyFile,
    #[error("key file: {0}")]
    Io(#[from] std::io::Error),
}

/// 32-byte symmetric key, zeroized on drop. Not `Clone`; `Debug` never prints the bytes.
pub struct Key([u8; 32]);

impl Drop for Key {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(..)")
    }
}

fn read_key(path: &Path) -> std::io::Result<Result<Key, CryptoError>> {
    let mut bytes = std::fs::read(path)?;
    let key = <[u8; 32]>::try_from(bytes.as_slice())
        .map(Key)
        .map_err(|_| CryptoError::BadKeyFile);
    bytes.zeroize();
    Ok(key)
}

impl Key {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    /// Reads the key file, or creates it (mode 0600) with 32 random bytes.
    /// An existing file of the wrong size is never overwritten. Creation is atomic and
    /// race-free: the key is written to a temp file and hard-linked into place (the link
    /// fails if the file exists), so a reader never sees a partial key and concurrent
    /// creators all end up with the same key.
    pub fn load_or_create(path: &Path) -> Result<Self, CryptoError> {
        match read_key(path) {
            Ok(k) => return k,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut bytes = [0u8; 32];
        rand::fill(&mut bytes[..]);
        let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
        let written = (|| {
            let mut f = crate::fsutil::create_new_private(&tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()
        })();
        let linked = written.and_then(|()| std::fs::hard_link(&tmp, path));
        let _ = std::fs::remove_file(&tmp);
        match linked {
            Ok(()) => Ok(Self(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                bytes.zeroize();
                // another creator won: its file is complete
                read_key(path)?
            }
            Err(e) => {
                bytes.zeroize();
                Err(e.into())
            }
        }
    }
}

/// Returns `nonce(24) || ciphertext+tag`. `aad` is authenticated but not stored.
pub fn encrypt(key: &Key, plaintext: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::fill(&mut nonce[..]);
    let ct = cipher
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CryptoError::Encrypt)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(key: &Key, data: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if data.len() < NONCE_LEN {
        return Err(CryptoError::Decrypt);
    }
    let (nonce, ct) = data.split_at(NONCE_LEN);
    let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| CryptoError::Decrypt)?;
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    cipher
        .decrypt(&XNonce::from(nonce), Payload { msg: ct, aad })
        .map_err(|_| CryptoError::Decrypt)
}
