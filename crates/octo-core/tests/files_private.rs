use octo_core::crypto::Key;
use octo_core::files::FilesError;
use octo_core::files::private::PrivateFiles;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn key() -> Key {
    Key::from_bytes([9u8; 32])
}

fn all_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in std::fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(all_files(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[cfg(unix)]
#[test]
fn nothing_readable_on_disk_names_blobs_or_index() {
    let d = tempfile::tempdir().unwrap();
    let s = PrivateFiles::open(d.path(), key(), "n0").unwrap();
    s.write("diary/secret-name.txt", b"TOP-SECRET-CONTENT")
        .unwrap();
    s.mkdir("hidden-folder").unwrap();
    let files = all_files(d.path());
    assert!(files.iter().any(|p| p.file_name().unwrap() == "index.bin"));
    for p in &files {
        let name = p.to_string_lossy();
        assert!(
            !name.contains("secret-name")
                && !name.contains("diary")
                && !name.contains("hidden-folder"),
            "{name}"
        );
        let bytes = std::fs::read(p).unwrap();
        let hay = String::from_utf8_lossy(&bytes);
        assert!(
            !hay.contains("TOP-SECRET")
                && !hay.contains("secret-name")
                && !hay.contains("diary")
                && !hay.contains("hidden-folder"),
            "{name}"
        );
        assert_eq!(
            std::fs::metadata(p).unwrap().permissions().mode() & 0o777,
            0o600,
            "{name}"
        );
    }
    assert_eq!(files.len(), 2, "one blob + index: {files:?}");
}

#[test]
fn wrong_key_cannot_open_index() {
    let d = tempfile::tempdir().unwrap();
    PrivateFiles::open(d.path(), key(), "n0")
        .unwrap()
        .write("a", b"1")
        .unwrap();
    let r = PrivateFiles::open(d.path(), Key::from_bytes([1u8; 32]), "n0");
    assert!(matches!(r, Err(FilesError::Crypto(_))));
}

#[test]
fn tampered_index_is_detected() {
    let d = tempfile::tempdir().unwrap();
    PrivateFiles::open(d.path(), key(), "n0")
        .unwrap()
        .write("a", b"1")
        .unwrap();
    let p = d.path().join("index.bin");
    let mut b = std::fs::read(&p).unwrap();
    let last = b.len() - 1;
    b[last] ^= 1;
    std::fs::write(&p, b).unwrap();
    assert!(matches!(
        PrivateFiles::open(d.path(), key(), "n0"),
        Err(FilesError::Crypto(_))
    ));
}

#[test]
fn tampered_blob_is_detected() {
    let d = tempfile::tempdir().unwrap();
    let s = PrivateFiles::open(d.path(), key(), "n0").unwrap();
    s.write("a", b"content").unwrap();
    let blob = all_files(d.path())
        .into_iter()
        .find(|p| p.file_name().unwrap() != "index.bin")
        .unwrap();
    let mut b = std::fs::read(&blob).unwrap();
    b[30] ^= 1;
    std::fs::write(&blob, b).unwrap();
    assert!(matches!(s.read("a"), Err(FilesError::Crypto(_))));
}

#[test]
fn blobs_cannot_be_swapped_between_files() {
    let d = tempfile::tempdir().unwrap();
    let s = PrivateFiles::open(d.path(), key(), "n0").unwrap();
    s.write("a", b"AAAA").unwrap();
    s.write("b", b"BBBB").unwrap();
    let blobs: Vec<_> = all_files(d.path())
        .into_iter()
        .filter(|p| p.file_name().unwrap() != "index.bin")
        .collect();
    let (x, y) = (
        std::fs::read(&blobs[0]).unwrap(),
        std::fs::read(&blobs[1]).unwrap(),
    );
    std::fs::write(&blobs[0], y).unwrap();
    std::fs::write(&blobs[1], x).unwrap();
    assert!(matches!(s.read("a"), Err(FilesError::Crypto(_))));
    assert!(matches!(s.read("b"), Err(FilesError::Crypto(_))));
}

#[test]
fn delete_and_overwrite_remove_old_blobs() {
    let d = tempfile::tempdir().unwrap();
    let s = PrivateFiles::open(d.path(), key(), "n0").unwrap();
    s.write("a", b"1").unwrap();
    s.write("a", b"22").unwrap();
    assert_eq!(all_files(d.path()).len(), 2);
    s.delete("a").unwrap();
    assert_eq!(all_files(d.path()).len(), 1);
}

#[test]
fn rename_does_not_touch_blobs() {
    let d = tempfile::tempdir().unwrap();
    let s = PrivateFiles::open(d.path(), key(), "n0").unwrap();
    s.write("a", b"1").unwrap();
    let before = all_files(d.path());
    s.rename("a", "dir/b").unwrap();
    let mut after = all_files(d.path());
    let mut before = before;
    before.sort();
    after.sort();
    assert_eq!(before, after);
}
