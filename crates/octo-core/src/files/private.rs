use super::path::normalize;
use super::{FileEntry, FileKind, FilesError};
use crate::crypto::{Key, decrypt, encrypt};
use blake2::{Blake2s256, Digest};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const INDEX_AAD: &[u8] = b"octo-private-index-v1";

/// Lamport revision: compared by clock, then node id.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Rev {
    pub clock: u64,
    pub node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    /// Random blob name for files; empty for directories and tombstones.
    id: String,
    path: String,
    kind: Kind,
    size: u64,
    created_at: i64,
    modified: i64,
    /// BLAKE2s-256 hex of the plaintext (files only).
    #[serde(default)]
    hash: String,
    #[serde(default)]
    rev: Rev,
    #[serde(default)]
    deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    File,
    Dir,
}

impl Kind {
    fn from_file_kind(k: FileKind) -> Self {
        match k {
            FileKind::File => Kind::File,
            FileKind::Dir => Kind::Dir,
        }
    }

    fn file_kind(self) -> FileKind {
        match self {
            Kind::File => FileKind::File,
            Kind::Dir => FileKind::Dir,
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Index {
    entries: Vec<Entry>,
}

impl Index {
    /// Live (not deleted) entry.
    fn find(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path == path && !e.deleted)
    }

    fn has_children(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.entries
            .iter()
            .any(|e| !e.deleted && e.path.starts_with(&prefix))
    }

    fn max_clock(&self) -> u64 {
        self.entries.iter().map(|e| e.rev.clock).max().unwrap_or(0)
    }

    fn next_rev(&self, node: &str) -> Rev {
        Rev {
            clock: self.max_clock() + 1,
            node: node.into(),
        }
    }

    /// Removes any entry (live or tombstone) at `path`, returning it.
    fn take(&mut self, path: &str) -> Option<Entry> {
        let i = self.entries.iter().position(|e| e.path == path)?;
        Some(self.entries.remove(i))
    }

    /// Adds the missing ancestor directories of `path` (tombstoned ones are revived).
    fn ensure_parents(&mut self, path: &str, now: i64, rev: &Rev) -> Result<(), FilesError> {
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
                None => {
                    self.take(&cur);
                    self.entries.push(dir_entry(&cur, now, rev.clone()));
                }
            }
        }
        Ok(())
    }
}

fn dir_entry(path: &str, now: i64, rev: Rev) -> Entry {
    Entry {
        id: String::new(),
        path: path.into(),
        kind: Kind::Dir,
        size: 0,
        created_at: now,
        modified: now,
        hash: String::new(),
        rev,
        deleted: false,
    }
}

fn tombstone(old: &Entry, now: i64, rev: Rev) -> Entry {
    Entry {
        id: String::new(),
        path: old.path.clone(),
        kind: old.kind,
        size: 0,
        created_at: old.created_at,
        modified: now,
        hash: String::new(),
        rev,
        deleted: true,
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// BLAKE2s-256 hex of plaintext.
pub fn content_hash(data: &[u8]) -> String {
    Blake2s256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Encrypted file store. Blobs are named by random id; names, sizes and the tree live
/// only in the encrypted `index.bin`. Plaintext is never written to disk.
pub struct PrivateFiles {
    root: PathBuf,
    key: Key,
    node_id: String,
    index: Mutex<Index>,
}

impl PrivateFiles {
    /// `node_id` stamps the revision of every local change.
    pub fn open(root: &Path, key: Key, node_id: &str) -> Result<Self, FilesError> {
        std::fs::create_dir_all(root)?;
        let mut index: Index = match std::fs::read(root.join("index.bin")) {
            Ok(bytes) => {
                let plain = decrypt(&key, &bytes, INDEX_AAD)?;
                serde_json::from_slice(&plain).map_err(|_| FilesError::Corrupt)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Index::default(),
            Err(e) => return Err(e.into()),
        };
        // v1 indexes have no hashes: compute them from the blobs.
        let mut migrated = false;
        for e in &mut index.entries {
            if e.kind == Kind::File && !e.deleted && e.hash.is_empty() {
                let plain = std::fs::read(root.join(&e.id))
                    .map_err(FilesError::from)
                    .and_then(|ct| Ok(decrypt(&key, &ct, e.id.as_bytes())?));
                if let Ok(plain) = plain {
                    e.hash = content_hash(&plain);
                    migrated = true;
                }
            }
        }
        let s = Self {
            root: root.into(),
            key,
            node_id: node_id.into(),
            index: Mutex::new(index),
        };
        if migrated {
            s.save_index(&s.lock())?;
        }
        Ok(s)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn write_private(&self, path: &Path, data: &[u8]) -> Result<(), FilesError> {
        let mut f = crate::fsutil::create_new_private(path)?;
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

    fn read_blob(&self, id: &str) -> Result<Vec<u8>, FilesError> {
        let ct = std::fs::read(self.blob(id))?;
        Ok(decrypt(&self.key, &ct, id.as_bytes())?)
    }

    /// Encrypts `data` into a new blob and returns its id.
    fn store_blob(&self, data: &[u8]) -> Result<String, FilesError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let ct = encrypt(&self.key, data, id.as_bytes())?;
        self.write_private(&self.blob(&id), &ct)?;
        Ok(id)
    }

    fn remove_blob(&self, id: &str) {
        if !id.is_empty() {
            let _ = std::fs::remove_file(self.blob(id));
        }
    }

    pub fn list(&self) -> Result<Vec<FileEntry>, FilesError> {
        let mut out: Vec<FileEntry> = self
            .lock()
            .entries
            .iter()
            .filter(|e| !e.deleted)
            .map(|e| FileEntry {
                path: e.path.clone(),
                kind: e.kind.file_kind(),
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
        let now = now_ms();
        let rev = index.next_rev(&self.node_id);
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now, &rev)?;
        let old = next.take(&path);
        let created_at = old.as_ref().map(|e| e.created_at).unwrap_or(now);
        let old_id = old.map(|e| e.id).unwrap_or_default();
        let id = self.store_blob(data)?;
        next.entries.push(Entry {
            id: id.clone(),
            path: path.clone(),
            kind: Kind::File,
            size: data.len() as u64,
            created_at,
            modified: now,
            hash: content_hash(data),
            rev,
            deleted: false,
        });
        if let Err(e) = self.save_index(&next) {
            self.remove_blob(&id);
            return Err(e);
        }
        *index = next;
        self.remove_blob(&old_id);
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
        self.read_blob(&id)
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
        let rev = index.next_rev(&self.node_id);
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now, &rev)?;
        next.take(&path);
        next.entries.push(dir_entry(&path, now, rev));
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
        let rev = index.next_rev(&self.node_id);
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&to, now, &rev)?;
        let prefix = format!("{from}/");
        let moving: Vec<Entry> = next
            .entries
            .iter()
            .filter(|e| !e.deleted && (e.path == from || e.path.starts_with(&prefix)))
            .cloned()
            .collect();
        for old in moving {
            let new_path = match old.path.strip_prefix(&prefix) {
                Some(rest) => format!("{to}/{rest}"),
                None => to.clone(),
            };
            next.take(&old.path);
            next.take(&new_path);
            next.entries.push(tombstone(&old, now, rev.clone()));
            next.entries.push(Entry {
                path: new_path,
                rev: rev.clone(),
                ..old
            });
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
        let old = e.clone();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.take(&path);
        next.entries
            .push(tombstone(&old, now_ms(), index.next_rev(&self.node_id)));
        self.save_index(&next)?;
        *index = next;
        self.remove_blob(&old.id);
        Ok(())
    }

    /// All entries including tombstones, ordered by path.
    pub fn manifest(&self) -> Vec<SyncEntry> {
        let mut out: Vec<SyncEntry> = self
            .lock()
            .entries
            .iter()
            .map(|e| SyncEntry {
                path: e.path.clone(),
                kind: e.kind.file_kind(),
                size: e.size,
                modified: e.modified,
                hash: e.hash.clone(),
                rev: e.rev.clone(),
                deleted: e.deleted,
            })
            .collect();
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Plaintext of any live file with this content hash.
    pub fn read_by_hash(&self, hash: &str) -> Option<Vec<u8>> {
        if hash.is_empty() {
            return None;
        }
        let id = self
            .lock()
            .entries
            .iter()
            .find(|e| !e.deleted && e.kind == Kind::File && e.hash == hash)?
            .id
            .clone();
        self.read_blob(&id).ok()
    }

    /// Applies a remote entry, keeping its revision. `data` is the file content, or
    /// `None` when it is a directory, a tombstone, or already present locally by hash.
    /// Nothing changes on error.
    pub fn apply(&self, entry: &SyncEntry, data: Option<&[u8]>) -> Result<(), FilesError> {
        let path = non_root(&entry.path)?;
        let live_file = !entry.deleted && entry.kind == FileKind::File;
        if !well_formed(entry) || path != entry.path || (!live_file && data.is_some()) {
            return Err(FilesError::Invalid);
        }
        let plain: Option<Vec<u8>> = if live_file {
            let bytes = match data {
                Some(d) => d.to_vec(),
                None => self.read_by_hash(&entry.hash).ok_or(FilesError::NotFound)?,
            };
            if content_hash(&bytes) != entry.hash {
                return Err(FilesError::HashMismatch);
            }
            Some(bytes)
        } else {
            None
        };
        let mut index = self.lock();
        let any = index.entries.iter().find(|e| e.path == path);
        if matches!(any, Some(e) if e.rev >= entry.rev) {
            return Ok(());
        }
        if index.find(&path).is_some_and(|e| e.kind == Kind::Dir)
            && (live_file || entry.deleted)
            && index.has_children(&path)
        {
            return Err(FilesError::NotEmpty);
        }
        let now = now_ms();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        if !entry.deleted {
            next.ensure_parents(&path, now, &Rev::default())?;
        }
        let old = next.take(&path);
        let created_at = old.as_ref().map(|e| e.created_at).unwrap_or(now);
        let old_id = old.as_ref().map(|e| e.id.clone()).unwrap_or_default();
        let mut new_id = String::new();
        let new = if entry.deleted {
            Entry {
                id: String::new(),
                path: path.clone(),
                kind: Kind::from_file_kind(entry.kind),
                size: 0,
                created_at,
                modified: entry.modified,
                hash: String::new(),
                rev: entry.rev.clone(),
                deleted: true,
            }
        } else if let Some(bytes) = &plain {
            new_id = self.store_blob(bytes)?;
            Entry {
                id: new_id.clone(),
                path: path.clone(),
                kind: Kind::File,
                size: bytes.len() as u64,
                created_at,
                modified: entry.modified,
                hash: entry.hash.clone(),
                rev: entry.rev.clone(),
                deleted: false,
            }
        } else {
            Entry {
                modified: entry.modified,
                ..dir_entry(&path, created_at, entry.rev.clone())
            }
        };
        next.entries.push(new);
        if let Err(e) = self.save_index(&next) {
            self.remove_blob(&new_id);
            return Err(e);
        }
        *index = next;
        self.remove_blob(&old_id);
        Ok(())
    }
}

fn is_hash(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A remote entry is acceptable if its path is already normalized and non-root, and a live
/// file carries a well-formed content hash.
fn well_formed(e: &SyncEntry) -> bool {
    let path_ok = non_root(&e.path).is_ok_and(|p| p == e.path);
    path_ok && (e.deleted || e.kind != FileKind::File || is_hash(&e.hash))
}

fn non_root(path: &str) -> Result<String, FilesError> {
    let n = normalize(path)?;
    if n.is_empty() {
        Err(FilesError::Invalid)
    } else {
        Ok(n)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncEntry {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    pub modified: i64,
    pub hash: String,
    pub rev: Rev,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The content is not present locally: download it, then `apply`.
    Fetch(SyncEntry),
    /// Everything needed is local (dir, tombstone, or known hash): `apply` with no data.
    Adopt(SyncEntry),
}

/// What to do with the remote entries given the local manifest. Per path the remote
/// wins iff its revision is newer; a path missing locally loses to anything.
pub fn plan(local: &[SyncEntry], remote: &[SyncEntry]) -> Vec<Action> {
    let by_path: HashMap<&str, &SyncEntry> = local.iter().map(|e| (e.path.as_str(), e)).collect();
    let have: HashSet<&str> = local
        .iter()
        .filter(|e| !e.deleted && e.kind == FileKind::File)
        .map(|e| e.hash.as_str())
        .collect();
    remote
        .iter()
        .filter(|r| well_formed(r))
        .filter(|r| by_path.get(r.path.as_str()).is_none_or(|l| r.rev > l.rev))
        .map(|r| {
            let needs_data =
                !r.deleted && r.kind == FileKind::File && !have.contains(r.hash.as_str());
            if needs_data {
                Action::Fetch(r.clone())
            } else {
                Action::Adopt(r.clone())
            }
        })
        .collect()
}
