use super::path::normalize;
use super::{FileEntry, FileKind, FilesError};
use crate::crypto::{Key, decrypt, encrypt};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const INDEX_AAD: &[u8] = b"octo-private-index-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// Random blob name for files; empty for directories.
    id: String,
    path: String,
    kind: Kind,
    size: u64,
    created_at: i64,
    modified: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    File,
    Dir,
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    entries: Vec<Entry>,
}

impl Index {
    fn find(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path)
    }

    fn has_children(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.entries.iter().any(|e| e.path.starts_with(&prefix))
    }

    /// Adds the missing ancestor directories of `path`.
    fn ensure_parents(&mut self, path: &str, now: i64) -> Result<(), FilesError> {
        let mut cur = String::new();
        let parts: Vec<&str> = path.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            if !cur.is_empty() {
                cur.push('/');
            }
            cur.push_str(part);
            match self.find(&cur) {
                Some(e) if e.kind == Kind::File => return Err(FilesError::NotDir),
                Some(_) => {}
                None => self.entries.push(dir_entry(&cur, now)),
            }
        }
        Ok(())
    }
}

fn dir_entry(path: &str, now: i64) -> Entry {
    Entry {
        id: String::new(),
        path: path.into(),
        kind: Kind::Dir,
        size: 0,
        created_at: now,
        modified: now,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Encrypted file store. Blobs are named by random id; names, sizes and the tree live
/// only in the encrypted `index.bin`. Plaintext is never written to disk.
pub struct PrivateFiles {
    root: PathBuf,
    key: Key,
    index: Mutex<Index>,
}

impl PrivateFiles {
    pub fn open(root: &Path, key: Key) -> Result<Self, FilesError> {
        std::fs::create_dir_all(root)?;
        let index = match std::fs::read(root.join("index.bin")) {
            Ok(bytes) => {
                let plain = decrypt(&key, &bytes, INDEX_AAD)?;
                serde_json::from_slice(&plain).map_err(|_| FilesError::Corrupt)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            root: root.into(),
            key,
            index: Mutex::new(index),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write_private(&self, path: &Path, data: &[u8]) -> Result<(), FilesError> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(data)?;
        f.sync_all()?;
        Ok(())
    }

    fn save_index(&self, index: &Index) -> Result<(), FilesError> {
        let plain = serde_json::to_vec(index).map_err(|_| FilesError::Corrupt)?;
        let ct = encrypt(&self.key, &plain, INDEX_AAD)?;
        let tmp = self.root.join("index.bin.tmp");
        let _ = std::fs::remove_file(&tmp);
        self.write_private(&tmp, &ct)?;
        std::fs::rename(&tmp, self.root.join("index.bin"))?;
        Ok(())
    }

    fn blob(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn list(&self) -> Result<Vec<FileEntry>, FilesError> {
        let mut out: Vec<FileEntry> = self
            .lock()
            .entries
            .iter()
            .map(|e| FileEntry {
                path: e.path.clone(),
                kind: match e.kind {
                    Kind::File => FileKind::File,
                    Kind::Dir => FileKind::Dir,
                },
                size: e.size,
                modified: e.modified,
            })
            .collect();
        super::sort_entries(&mut out);
        Ok(out)
    }

    pub fn write(&self, path: &str, data: &[u8]) -> Result<FileEntry, FilesError> {
        let path = non_root(path)?;
        let mut index = self.lock();
        if matches!(index.find(&path), Some(e) if e.kind == Kind::Dir) {
            return Err(FilesError::IsDir);
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let ct = encrypt(&self.key, data, id.as_bytes())?;
        let now = now_ms();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now)?;
        let old = next.entries.iter().position(|e| e.path == path);
        let created_at = old.map(|i| next.entries[i].created_at).unwrap_or(now);
        let old_id = old.map(|i| next.entries.remove(i).id);
        let entry = Entry {
            id: id.clone(),
            path: path.clone(),
            kind: Kind::File,
            size: data.len() as u64,
            created_at,
            modified: now,
        };
        next.entries.push(entry);
        self.write_private(&self.blob(&id), &ct)?;
        if let Err(e) = self.save_index(&next) {
            let _ = std::fs::remove_file(self.blob(&id));
            return Err(e);
        }
        *index = next;
        if let Some(old_id) = old_id {
            let _ = std::fs::remove_file(self.blob(&old_id));
        }
        Ok(FileEntry {
            path,
            kind: FileKind::File,
            size: data.len() as u64,
            modified: now,
        })
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, FilesError> {
        let path = non_root(path)?;
        let id = {
            let index = self.lock();
            let e = index.find(&path).ok_or(FilesError::NotFound)?;
            if e.kind == Kind::Dir {
                return Err(FilesError::IsDir);
            }
            e.id.clone()
        };
        let ct = std::fs::read(self.blob(&id))?;
        Ok(decrypt(&self.key, &ct, id.as_bytes())?)
    }

    pub fn mkdir(&self, path: &str) -> Result<(), FilesError> {
        let path = non_root(path)?;
        let mut index = self.lock();
        match index.find(&path) {
            Some(e) if e.kind == Kind::File => return Err(FilesError::Exists),
            Some(_) => return Ok(()),
            None => {}
        }
        let now = now_ms();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now)?;
        next.entries.push(dir_entry(&path, now));
        self.save_index(&next)?;
        *index = next;
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), FilesError> {
        let from = non_root(from)?;
        let to = non_root(to)?;
        if to == from || to.starts_with(&format!("{from}/")) {
            return Err(FilesError::Invalid);
        }
        let mut index = self.lock();
        if index.find(&from).is_none() {
            return Err(FilesError::NotFound);
        }
        if index.find(&to).is_some() {
            return Err(FilesError::Exists);
        }
        let now = now_ms();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&to, now)?;
        let prefix = format!("{from}/");
        for e in &mut next.entries {
            if e.path == from {
                e.path = to.clone();
            } else if let Some(rest) = e.path.strip_prefix(&prefix) {
                e.path = format!("{to}/{rest}");
            }
        }
        self.save_index(&next)?;
        *index = next;
        Ok(())
    }

    pub fn delete(&self, path: &str) -> Result<(), FilesError> {
        let path = non_root(path)?;
        let mut index = self.lock();
        let e = index.find(&path).ok_or(FilesError::NotFound)?;
        if e.kind == Kind::Dir && index.has_children(&path) {
            return Err(FilesError::NotEmpty);
        }
        let id = e.id.clone();
        let next = Index {
            entries: index
                .entries
                .iter()
                .filter(|e| e.path != path)
                .cloned()
                .collect(),
        };
        self.save_index(&next)?;
        *index = next;
        if !id.is_empty() {
            let _ = std::fs::remove_file(self.blob(&id));
        }
        Ok(())
    }
}

fn non_root(path: &str) -> Result<String, FilesError> {
    let n = normalize(path)?;
    if n.is_empty() {
        Err(FilesError::Invalid)
    } else {
        Ok(n)
    }
}
