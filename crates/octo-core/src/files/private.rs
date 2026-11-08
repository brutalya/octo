use super::path::normalize;
use super::{FileEntry, FileKind, FilesError};
use crate::crypto::{Key, decrypt, encrypt};
use crate::fsutil::now_ms;
use blake2::{Blake2s256, Digest};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    kind: FileKind,
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

    /// Adds the missing ancestor directories of `path`. Nothing changes on error.
    /// A tombstoned ancestor is revived with `revive`, any other missing one gets `fresh`.
    fn ensure_parents(
        &mut self,
        path: &str,
        now: i64,
        fresh: &Rev,
        revive: &Rev,
    ) -> Result<(), FilesError> {
        let mut cur = String::new();
        let parts: Vec<&str> = path.split('/').collect();
        let mut missing = vec![];
        for part in &parts[..parts.len() - 1] {
            if !cur.is_empty() {
                cur.push('/');
            }
            cur.push_str(part);
            match self.find(&cur) {
                Some(e) if e.kind == FileKind::File => return Err(FilesError::NotDir),
                Some(_) => {}
                None => missing.push(cur.clone()),
            }
        }
        for m in missing {
            let rev = match self.take(&m) {
                Some(_) => revive,
                None => fresh,
            };
            self.entries.push(dir_entry(&m, now, rev.clone()));
        }
        Ok(())
    }
}

fn dir_entry(path: &str, now: i64, rev: Rev) -> Entry {
    Entry {
        id: String::new(),
        path: path.into(),
        kind: FileKind::Dir,
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
    /// Live files whose blob could not be read during the hash migration.
    unreadable: Vec<String>,
}

impl PrivateFiles {
    /// `node_id` stamps the revision of every local change. Opening also removes leftovers
    /// of interrupted writes: a stale `index.bin.tmp` and blobs no index entry refers to.
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
        // v1 indexes have no hashes: compute them from the blobs. A blob that cannot be
        // read keeps its (empty) hash, stays out of the sync manifest and is reported.
        let mut migrated = false;
        let mut unreadable = vec![];
        for e in &mut index.entries {
            if e.kind == FileKind::File && !e.deleted && e.hash.is_empty() {
                let plain = std::fs::read(root.join(&e.id))
                    .map_err(FilesError::from)
                    .and_then(|ct| Ok(decrypt(&key, &ct, e.id.as_bytes())?));
                match plain {
                    Ok(plain) => {
                        e.hash = content_hash(&plain);
                        migrated = true;
                    }
                    Err(_) => unreadable.push(e.path.clone()),
                }
            }
        }
        sweep(root, &index);
        let s = Self {
            root: root.into(),
            key,
            node_id: node_id.into(),
            index: Mutex::new(index),
            unreadable,
        };
        if migrated {
            s.save_index(&s.lock())?;
        }
        Ok(s)
    }

    /// Paths of live files whose content could not be read when the store was opened.
    pub fn unreadable(&self) -> &[String] {
        &self.unreadable
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
        let plain = serde_json::to_vec(index).map_err(|_| FilesError::Serialize)?;
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
                kind: e.kind,
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
        if matches!(index.find(&path), Some(e) if e.kind == FileKind::Dir) {
            return Err(FilesError::IsDir);
        }
        let now = now_ms();
        let rev = index.next_rev(&self.node_id);
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now, &rev, &rev)?;
        let old = next.take(&path);
        let created_at = old.as_ref().map(|e| e.created_at).unwrap_or(now);
        let old_id = old.map(|e| e.id).unwrap_or_default();
        let id = self.store_blob(data)?;
        next.entries.push(Entry {
            id: id.clone(),
            path: path.clone(),
            kind: FileKind::File,
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
        let mut attempt = 0;
        loop {
            let id = {
                let index = self.lock();
                let e = index.find(&path).ok_or(FilesError::NotFound)?;
                if e.kind == FileKind::Dir {
                    return Err(FilesError::IsDir);
                }
                e.id.clone()
            };
            match self.read_blob(&id) {
                // The file was replaced between lookup and read (its old blob is gone):
                // look it up again once.
                Err(FilesError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound && attempt == 0 =>
                {
                    attempt += 1;
                }
                r => return r,
            }
        }
    }

    pub fn mkdir(&self, path: &str) -> Result<(), FilesError> {
        let path = non_root(path)?;
        let mut index = self.lock();
        match index.find(&path) {
            Some(e) if e.kind == FileKind::File => return Err(FilesError::Exists),
            Some(_) => return Ok(()),
            None => {}
        }
        let now = now_ms();
        let rev = index.next_rev(&self.node_id);
        let mut next = Index {
            entries: index.entries.clone(),
        };
        next.ensure_parents(&path, now, &rev, &rev)?;
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
        next.ensure_parents(&to, now, &rev, &rev)?;
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
        if e.kind == FileKind::Dir && index.has_children(&path) {
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

    /// All entries including tombstones, ordered by path. Live files without a content
    /// hash (unreadable legacy blobs) are left out.
    pub fn manifest(&self) -> Vec<SyncEntry> {
        let mut out: Vec<SyncEntry> = self
            .lock()
            .entries
            .iter()
            .filter(|e| e.deleted || e.kind != FileKind::File || !e.hash.is_empty())
            .map(|e| SyncEntry {
                path: e.path.clone(),
                kind: e.kind,
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
        for attempt in 0..2 {
            let id = find_by_hash(&self.lock(), hash)?.id.clone();
            match self.read_blob(&id) {
                Ok(v) => return Some(v),
                // replaced between lookup and read: look the hash up again once
                Err(FilesError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound && attempt == 0 => {}
                Err(_) => return None,
            }
        }
        None
    }

    /// Applies a remote entry, keeping its revision. `data` is the file content, or
    /// `None` when it is a directory, a tombstone, or already present locally by hash.
    /// Nothing changes on error.
    pub fn apply(&self, entry: &SyncEntry, data: Option<&[u8]>) -> Result<(), FilesError> {
        self.apply_refs(&[(entry, data)])?
            .into_iter()
            .next()
            .unwrap_or(Err(FilesError::Invalid))
    }

    /// Like `apply` for many entries, in order, with a single index save. The outer error
    /// means the index could not be saved (nothing was applied); otherwise each entry has
    /// its own result and a failed entry changes nothing.
    pub fn apply_batch(
        &self,
        items: &[(SyncEntry, Option<Vec<u8>>)],
    ) -> Result<Vec<Result<(), FilesError>>, FilesError> {
        let refs: Vec<(&SyncEntry, Option<&[u8]>)> =
            items.iter().map(|(e, d)| (e, d.as_deref())).collect();
        self.apply_refs(&refs)
    }

    fn apply_refs(
        &self,
        items: &[(&SyncEntry, Option<&[u8]>)],
    ) -> Result<Vec<Result<(), FilesError>>, FilesError> {
        let mut index = self.lock();
        let mut next = Index {
            entries: index.entries.clone(),
        };
        let mut results = Vec::with_capacity(items.len());
        let (mut created, mut dropped) = (vec![], vec![]);
        let mut changed = false;
        for (entry, data) in items {
            match self.apply_into(&mut next, entry, *data, &mut created, &mut dropped) {
                Ok(c) => {
                    changed |= c;
                    results.push(Ok(()));
                }
                Err(e) => results.push(Err(e)),
            }
        }
        if changed {
            if let Err(e) = self.save_index(&next) {
                created.iter().for_each(|id| self.remove_blob(id));
                return Err(e);
            }
            *index = next;
            dropped.iter().for_each(|id| self.remove_blob(id));
        } else {
            created.iter().for_each(|id| self.remove_blob(id));
        }
        Ok(results)
    }

    /// Applies one entry to the working index. Returns whether anything changed; on error
    /// the index is untouched. New blob ids go to `created`, replaced ones to `dropped`.
    fn apply_into(
        &self,
        next: &mut Index,
        entry: &SyncEntry,
        data: Option<&[u8]>,
        created: &mut Vec<String>,
        dropped: &mut Vec<String>,
    ) -> Result<bool, FilesError> {
        let path = non_root(&entry.path)?;
        let live_file = !entry.deleted && entry.kind == FileKind::File;
        if !well_formed(entry) || (!live_file && data.is_some()) {
            return Err(FilesError::Invalid);
        }
        let plain: Option<Vec<u8>> = if live_file {
            let bytes = match data {
                Some(d) => d.to_vec(),
                None => blob_by_hash(self, next, &entry.hash)?.ok_or(FilesError::NotFound)?,
            };
            if content_hash(&bytes) != entry.hash {
                return Err(FilesError::HashMismatch);
            }
            Some(bytes)
        } else {
            None
        };
        if matches!(next.entries.iter().find(|e| e.path == path), Some(e) if e.rev >= entry.rev) {
            return Ok(false);
        }
        if next.find(&path).is_some_and(|e| e.kind == FileKind::Dir)
            && (live_file || entry.deleted)
            && next.has_children(&path)
        {
            return Err(FilesError::NotEmpty);
        }
        let now = now_ms();
        let new_blob = match &plain {
            Some(bytes) => Some(self.store_blob(bytes)?),
            None => None,
        };
        if !entry.deleted {
            // A tombstoned ancestor is revived with a revision newer than everything seen,
            // so the revival propagates over the tombstone on the other nodes.
            let revive = Rev {
                clock: next.max_clock().max(entry.rev.clock) + 1,
                node: self.node_id.clone(),
            };
            if let Err(e) = next.ensure_parents(&path, now, &Rev::default(), &revive) {
                if let Some(id) = &new_blob {
                    self.remove_blob(id);
                }
                return Err(e);
            }
        }
        let old = next.take(&path);
        let created_at = old.as_ref().map(|e| e.created_at).unwrap_or(now);
        if let Some(old) = old {
            dropped.push(old.id);
        }
        let new = if entry.deleted {
            Entry {
                id: String::new(),
                path,
                kind: entry.kind,
                size: 0,
                created_at,
                modified: entry.modified,
                hash: String::new(),
                rev: entry.rev.clone(),
                deleted: true,
            }
        } else if let (Some(bytes), Some(id)) = (&plain, new_blob) {
            created.push(id.clone());
            Entry {
                id,
                path,
                kind: FileKind::File,
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
        Ok(true)
    }
}

fn find_by_hash<'a>(index: &'a Index, hash: &str) -> Option<&'a Entry> {
    if hash.is_empty() {
        return None;
    }
    index
        .entries
        .iter()
        .find(|e| !e.deleted && e.kind == FileKind::File && e.hash == hash)
}

/// Plaintext of a live file with this content hash in `index`.
fn blob_by_hash(
    store: &PrivateFiles,
    index: &Index,
    hash: &str,
) -> Result<Option<Vec<u8>>, FilesError> {
    find_by_hash(index, hash)
        .map(|e| store.read_blob(&e.id))
        .transpose()
}

/// Removes leftovers of interrupted writes: a stale `index.bin.tmp` and blob files (32 hex
/// chars) that no index entry refers to. Unknown files are left alone.
fn sweep(root: &Path, index: &Index) {
    let _ = std::fs::remove_file(root.join("index.bin.tmp"));
    let used: HashSet<&str> = index
        .entries
        .iter()
        .filter(|e| !e.id.is_empty())
        .map(|e| e.id.as_str())
        .collect();
    let Ok(dir) = std::fs::read_dir(root) else {
        return;
    };
    for e in dir.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        let blob_like = name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit());
        if blob_like && !used.contains(name) && e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(e.path());
        }
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
///
/// Entries that `apply` would reject forever are left out, so a plan converges:
/// - a remote tombstone (or file) over a directory that keeps live children: the local
///   directory stays; when the remote later receives a child, it revives the directory
///   with a newer revision, which then propagates here;
/// - a remote entry below a path that stays a live file.
///
/// Tombstones come after the other actions, deepest first, so children go before parents.
pub fn plan(local: &[SyncEntry], remote: &[SyncEntry]) -> Vec<Action> {
    let by_path: HashMap<&str, &SyncEntry> = local.iter().map(|e| (e.path.as_str(), e)).collect();
    let have: HashSet<&str> = local
        .iter()
        .filter(|e| !e.deleted && e.kind == FileKind::File)
        .map(|e| e.hash.as_str())
        .collect();
    let winners: Vec<&SyncEntry> = remote
        .iter()
        .filter(|r| well_formed(r))
        .filter(|r| by_path.get(r.path.as_str()).is_none_or(|l| r.rev > l.rev))
        .collect();
    // live paths after all winners are applied
    let mut live: HashMap<&str, FileKind> = local
        .iter()
        .filter(|e| !e.deleted)
        .map(|e| (e.path.as_str(), e.kind))
        .collect();
    for r in &winners {
        if r.deleted {
            live.remove(r.path.as_str());
        } else {
            live.insert(r.path.as_str(), r.kind);
        }
    }
    let parents: HashSet<&str> = live
        .keys()
        .flat_map(|p| p.match_indices('/').map(|(i, _)| &p[..i]))
        .collect();
    let blocked = |r: &SyncEntry| {
        let local_dir = by_path
            .get(r.path.as_str())
            .is_some_and(|l| !l.deleted && l.kind == FileKind::Dir);
        let keeps_children = local_dir
            && (r.deleted || r.kind == FileKind::File)
            && parents.contains(r.path.as_str());
        let under_file = !r.deleted
            && r.path
                .match_indices('/')
                .any(|(i, _)| live.get(&r.path[..i]) == Some(&FileKind::File));
        keeps_children || under_file
    };
    let mut actions: Vec<(&SyncEntry, Action)> = winners
        .into_iter()
        .filter(|r| !blocked(r))
        .map(|r| {
            let needs_data =
                !r.deleted && r.kind == FileKind::File && !have.contains(r.hash.as_str());
            let a = if needs_data {
                Action::Fetch(r.clone())
            } else {
                Action::Adopt(r.clone())
            };
            (r, a)
        })
        .collect();
    actions.sort_by_key(|(r, _)| {
        if r.deleted {
            (1, usize::MAX - r.path.matches('/').count())
        } else {
            (0, 0)
        }
    });
    actions.into_iter().map(|(_, a)| a).collect()
}
