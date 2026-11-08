use octo_core::crypto::{Key, encrypt};
use octo_core::files::private::{Action, PrivateFiles, Rev, SyncEntry, content_hash, plan};
use octo_core::files::{FileKind, FilesError};
use std::path::Path;

fn key() -> Key {
    Key::from_bytes([9u8; 32])
}

fn open(d: &Path, node: &str) -> PrivateFiles {
    PrivateFiles::open(d, key(), node).unwrap()
}

fn rev(clock: u64, node: &str) -> Rev {
    Rev {
        clock,
        node: node.into(),
    }
}

fn file(path: &str, data: &[u8], r: Rev) -> SyncEntry {
    SyncEntry {
        path: path.into(),
        kind: FileKind::File,
        size: data.len() as u64,
        modified: 1,
        hash: content_hash(data),
        rev: r,
        deleted: false,
    }
}

fn tomb(path: &str, r: Rev) -> SyncEntry {
    SyncEntry {
        path: path.into(),
        kind: FileKind::File,
        size: 0,
        modified: 1,
        hash: String::new(),
        rev: r,
        deleted: true,
    }
}

fn dir(path: &str, r: Rev) -> SyncEntry {
    SyncEntry {
        path: path.into(),
        kind: FileKind::Dir,
        size: 0,
        modified: 1,
        hash: String::new(),
        rev: r,
        deleted: false,
    }
}

fn find<'a>(m: &'a [SyncEntry], path: &str) -> &'a SyncEntry {
    m.iter()
        .find(|e| e.path == path)
        .expect("entry in manifest")
}

#[test]
fn plan_newer_remote_wins() {
    let local = [file("a", b"old", rev(1, "x"))];
    let remote = [file("a", b"new", rev(2, "y"))];
    let p = plan(&local, &remote);
    assert_eq!(p, vec![Action::Fetch(remote[0].clone())]);
}

#[test]
fn plan_older_remote_ignored() {
    let local = [file("a", b"new", rev(3, "x"))];
    let remote = [file("a", b"old", rev(2, "y"))];
    assert!(plan(&local, &remote).is_empty());
}

#[test]
fn plan_tie_broken_by_node_string() {
    let local = [file("a", b"l", rev(2, "a"))];
    let remote = [file("a", b"r", rev(2, "b"))];
    assert_eq!(plan(&local, &remote).len(), 1);
    let local = [file("a", b"l", rev(2, "b"))];
    let remote = [file("a", b"r", rev(2, "a"))];
    assert!(plan(&local, &remote).is_empty());
}

#[test]
fn plan_is_idempotent_for_equal_rev() {
    let e = [file("a", b"same", rev(2, "x"))];
    assert!(plan(&e, &e).is_empty());
}

#[test]
fn plan_tombstone_wins_over_older_file_and_is_adopted() {
    let local = [file("a", b"data", rev(1, "x"))];
    let remote = [tomb("a", rev(2, "y"))];
    assert_eq!(
        plan(&local, &remote),
        vec![Action::Adopt(remote[0].clone())]
    );
}

#[test]
fn plan_missing_locally_loses_to_anything() {
    let remote = [
        file("f", b"data", rev(0, "")),
        dir("d", rev(0, "")),
        tomb("t", rev(0, "")),
    ];
    let p = plan(&[], &remote);
    assert_eq!(
        p,
        vec![
            Action::Fetch(remote[0].clone()),
            Action::Adopt(remote[1].clone()),
            Action::Adopt(remote[2].clone()),
        ]
    );
}

#[test]
fn plan_fetch_vs_adopt_by_local_hash() {
    // Local has the blob under another path: adopt, no transfer.
    let local = [file("old/name", b"same bytes", rev(1, "x"))];
    let remote = [file("new/name", b"same bytes", rev(2, "y"))];
    assert_eq!(
        plan(&local, &remote),
        vec![Action::Adopt(remote[0].clone())]
    );
    // Unknown hash: fetch.
    let remote = [file("new/name", b"other bytes", rev(2, "y"))];
    assert_eq!(
        plan(&local, &remote),
        vec![Action::Fetch(remote[0].clone())]
    );
}

#[test]
fn local_changes_bump_lamport_clock_with_node_id() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "node-a");
    s.write("a.txt", b"1").unwrap();
    let c1 = find(&s.manifest(), "a.txt").rev.clone();
    assert_eq!(c1.node, "node-a");
    assert!(c1.clock >= 1);
    s.write("a.txt", b"2").unwrap();
    let c2 = find(&s.manifest(), "a.txt").rev.clone();
    assert!(c2.clock > c1.clock);
    s.mkdir("d").unwrap();
    let c3 = find(&s.manifest(), "d").rev.clone();
    assert!(c3.clock > c2.clock);
}

#[test]
fn rename_leaves_tombstone_and_keeps_hash() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("a.txt", b"hello").unwrap();
    let before = find(&s.manifest(), "a.txt").clone();
    s.rename("a.txt", "b.txt").unwrap();
    let m = s.manifest();
    let old = find(&m, "a.txt");
    let new = find(&m, "b.txt");
    assert!(old.deleted);
    assert!(!new.deleted);
    assert_eq!(new.hash, before.hash);
    assert_eq!(new.hash, content_hash(b"hello"));
    assert!(old.rev > before.rev);
    assert!(new.rev > before.rev);
    assert_eq!(s.read("b.txt").unwrap(), b"hello");
    assert!(matches!(s.read("a.txt"), Err(FilesError::NotFound)));
}

#[test]
fn delete_leaves_tombstone_and_removes_blob() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("a.txt", b"hello").unwrap();
    let before = find(&s.manifest(), "a.txt").clone();
    s.delete("a.txt").unwrap();
    let m = s.manifest();
    let t = find(&m, "a.txt");
    assert!(t.deleted);
    assert!(t.rev > before.rev);
    assert_eq!(s.read_by_hash(&before.hash), None);
}

#[test]
fn recreating_tombstoned_path_is_normal_write_with_new_rev() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("a.txt", b"one").unwrap();
    s.delete("a.txt").unwrap();
    let t = find(&s.manifest(), "a.txt").rev.clone();
    s.write("a.txt", b"two").unwrap();
    let m = s.manifest();
    let e = find(&m, "a.txt");
    assert!(!e.deleted);
    assert!(e.rev > t);
    assert_eq!(s.read("a.txt").unwrap(), b"two");
}

#[test]
fn list_and_read_ignore_tombstones() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("a.txt", b"x").unwrap();
    s.delete("a.txt").unwrap();
    assert!(s.list().unwrap().is_empty());
    assert!(matches!(s.read("a.txt"), Err(FilesError::NotFound)));
    // A tombstoned dir does not make its parent non-empty or block mkdir.
    s.mkdir("dir").unwrap();
    s.delete("dir").unwrap();
    s.mkdir("dir").unwrap();
    assert_eq!(s.list().unwrap().len(), 1);
}

#[test]
fn v1_index_migrates_with_rev_zero_and_computed_hash() {
    let d = tempfile::tempdir().unwrap();
    let id = "0123456789abcdef0123456789abcdef";
    let blob = encrypt(&key(), b"legacy", id.as_bytes()).unwrap();
    std::fs::write(d.path().join(id), blob).unwrap();
    let idx = serde_json::json!({"entries": [
        {"id": id, "path": "old.txt", "kind": "file", "size": 6, "created_at": 5, "modified": 5},
        {"id": "", "path": "dir", "kind": "dir", "size": 0, "created_at": 5, "modified": 5},
    ]});
    let ct = encrypt(
        &key(),
        &serde_json::to_vec(&idx).unwrap(),
        b"octo-private-index-v1",
    )
    .unwrap();
    std::fs::write(d.path().join("index.bin"), ct).unwrap();

    let s = open(d.path(), "n");
    let m = s.manifest();
    let f = find(&m, "old.txt");
    assert_eq!(f.rev, rev(0, ""));
    assert_eq!(f.hash, content_hash(b"legacy"));
    assert!(!f.deleted);
    assert_eq!(find(&m, "dir").rev, rev(0, ""));
    assert_eq!(s.read("old.txt").unwrap(), b"legacy");
    assert_eq!(s.read_by_hash(&f.hash).unwrap(), b"legacy");
    // Saved back once: a second open sees the same migrated data.
    drop(s);
    let s2 = open(d.path(), "n");
    assert_eq!(
        find(&s2.manifest(), "old.txt").hash,
        content_hash(b"legacy")
    );
}

#[test]
fn apply_rejects_hash_mismatch_and_changes_nothing() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    let e = file("a.txt", b"expected", rev(5, "peer"));
    let r = s.apply(&e, Some(b"tampered"));
    assert!(r.is_err());
    assert!(s.list().unwrap().is_empty());
    assert!(s.manifest().is_empty());
    assert!(s.apply(&e, None).is_err());
}

#[test]
fn apply_stores_file_with_remote_rev_and_bumps_clock() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    let e = file("sub/a.txt", b"remote data", rev(41, "peer"));
    s.apply(&e, Some(b"remote data")).unwrap();
    assert_eq!(s.read("sub/a.txt").unwrap(), b"remote data");
    let m = s.manifest();
    assert_eq!(find(&m, "sub/a.txt").rev, rev(41, "peer"));
    assert_eq!(s.read_by_hash(&e.hash).unwrap(), b"remote data");
    // Next local change is ordered after everything seen.
    s.write("b.txt", b"x").unwrap();
    let b = find(&s.manifest(), "b.txt").rev.clone();
    assert_eq!(b.clock, 42);
    assert_eq!(b.node, "n");
}

#[test]
fn apply_tombstone_removes_file_and_keeps_remote_rev() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("a.txt", b"x").unwrap();
    let t = tomb("a.txt", rev(50, "peer"));
    s.apply(&t, None).unwrap();
    assert!(s.list().unwrap().is_empty());
    assert_eq!(find(&s.manifest(), "a.txt").rev, rev(50, "peer"));
}

#[test]
fn apply_adopt_reuses_local_blob_by_hash() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("old.txt", b"shared").unwrap();
    let e = file("new.txt", b"shared", rev(9, "peer"));
    s.apply(&e, None).unwrap();
    assert_eq!(s.read("new.txt").unwrap(), b"shared");
}

#[test]
fn apply_dir_over_nonempty_dir_or_under_file_is_skipped() {
    let d = tempfile::tempdir().unwrap();
    let s = open(d.path(), "n");
    s.write("d/inner.txt", b"x").unwrap();
    s.write("f", b"file").unwrap();
    // Tombstone over a non-empty dir.
    assert!(s.apply(&tomb("d", rev(99, "peer")), None).is_err());
    // File under a file parent.
    let e = file("f/child.txt", b"c", rev(99, "peer"));
    assert!(s.apply(&e, Some(b"c")).is_err());
    assert_eq!(s.read("d/inner.txt").unwrap(), b"x");
    assert_eq!(s.read("f").unwrap(), b"file");
    assert!(s.manifest().iter().all(|m| m.path != "f/child.txt"));
}
