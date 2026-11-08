pub mod path;
pub mod private;
pub mod public;

use crate::crypto::{CryptoError, Key};
use crate::storage::Space;
use path::PathError;
use private::PrivateFiles;
use public::PublicFiles;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileKind {
    File,
    Dir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct FileEntry {
    pub path: String,
    pub kind: FileKind,
    #[ts(type = "number")]
    pub size: u64,
    /// Unix milliseconds.
    #[ts(type = "number")]
    pub modified: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum FilesError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("not found")]
    NotFound,
    #[error("already exists")]
    Exists,
    #[error("directory is not empty")]
    NotEmpty,
    #[error("is a directory")]
    IsDir,
    #[error("a parent is not a directory")]
    NotDir,
    #[error("content hash mismatch")]
    HashMismatch,
    #[error("invalid operation")]
    Invalid,
    #[error("private store: {0}")]
    Crypto(#[from] CryptoError),
    #[error("private index is corrupted")]
    Corrupt,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Both spaces on disk: `<data>/public` (plain) and `<data>/private` (encrypted).
pub struct Files {
    public: PublicFiles,
    private_root: PathBuf,
    key_path: PathBuf,
    /// Opened (and the key created) on first use.
    private: Mutex<Option<Arc<PrivateFiles>>>,
    node_id: String,
}

impl Files {
    pub fn open(data_dir: &std::path::Path, node_id: &str) -> Result<Self, FilesError> {
        Ok(Self {
            public: PublicFiles::open(&data_dir.join("public"))?,
            private_root: data_dir.join("private"),
            key_path: data_dir.join("keys").join("private.key"),
            private: Mutex::new(None),
            node_id: node_id.into(),
        })
    }

    pub fn public_dir(&self) -> &std::path::Path {
        self.public.root()
    }

    fn private(&self) -> Result<Arc<PrivateFiles>, FilesError> {
        let mut slot = self.private.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = slot.as_ref() {
            return Ok(p.clone());
        }
        let key = Key::load_or_create(&self.key_path)?;
        let p = Arc::new(PrivateFiles::open(&self.private_root, key, &self.node_id)?);
        *slot = Some(p.clone());
        Ok(p)
    }

    pub fn list(&self, space: Space) -> Result<Vec<FileEntry>, FilesError> {
        match space {
            Space::Public => self.public.list(),
            Space::Private => self.private()?.list(),
        }
    }

    pub fn write(&self, space: Space, path: &str, data: &[u8]) -> Result<FileEntry, FilesError> {
        match space {
            Space::Public => self.public.write(path, data),
            Space::Private => self.private()?.write(path, data),
        }
    }

    /// Private files are decrypted in memory only.
    pub fn read(&self, space: Space, path: &str) -> Result<Vec<u8>, FilesError> {
        match space {
            Space::Public => self.public.read(path),
            Space::Private => self.private()?.read(path),
        }
    }

    pub fn mkdir(&self, space: Space, path: &str) -> Result<(), FilesError> {
        match space {
            Space::Public => self.public.mkdir(path),
            Space::Private => self.private()?.mkdir(path),
        }
    }

    pub fn rename(&self, space: Space, from: &str, to: &str) -> Result<(), FilesError> {
        match space {
            Space::Public => self.public.rename(from, to),
            Space::Private => self.private()?.rename(from, to),
        }
    }

    pub fn delete(&self, space: Space, path: &str) -> Result<(), FilesError> {
        match space {
            Space::Public => self.public.delete(path),
            Space::Private => self.private()?.delete(path),
        }
    }
}

/// Component-wise order so a directory is immediately followed by its children.
pub(crate) fn sort_entries(v: &mut [FileEntry]) {
    v.sort_by(|a, b| a.path.split('/').cmp(b.path.split('/')));
}
