#[cfg(unix)]
use octo_core::files::FilesError;
use octo_core::files::public::PublicFiles;

#[test]
fn files_are_plain_on_disk() {
    let d = tempfile::tempdir().unwrap();
    let s = PublicFiles::open(d.path()).unwrap();
    s.write("notes/a.txt", b"plain").unwrap();
    assert_eq!(
        std::fs::read(d.path().join("notes/a.txt")).unwrap(),
        b"plain"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_cannot_escape_the_root() {
    let d = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), b"s3cret").unwrap();
    let root = d.path().join("pub");
    let s = PublicFiles::open(&root).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), root.join("flink")).unwrap();
    assert!(matches!(
        s.read("link/secret.txt"),
        Err(FilesError::Path(_))
    ));
    assert!(matches!(s.read("flink"), Err(FilesError::Path(_))));
    assert!(matches!(
        s.write("link/new.txt", b"x"),
        Err(FilesError::Path(_))
    ));
    assert!(!outside.path().join("new.txt").exists());
    assert!(matches!(s.delete("flink"), Err(FilesError::Path(_))));
    assert!(s.list().unwrap().is_empty(), "symlinks are not listed");
}

#[cfg(unix)]
#[test]
fn non_utf8_names_are_skipped_in_listing() {
    use std::os::unix::ffi::OsStrExt;
    let d = tempfile::tempdir().unwrap();
    let s = PublicFiles::open(d.path()).unwrap();
    s.write("ok.txt", b"x").unwrap();
    let bad = std::ffi::OsStr::from_bytes(b"bad-\xff-name");
    if std::fs::write(d.path().join(bad), b"y").is_err() {
        return; // the filesystem rejects non-UTF-8 names (e.g. APFS), nothing to skip
    }
    let names: Vec<String> = s.list().unwrap().into_iter().map(|e| e.path).collect();
    assert_eq!(names, vec!["ok.txt"]);
}

#[test]
fn size_comes_from_metadata() {
    let d = tempfile::tempdir().unwrap();
    let s = PublicFiles::open(d.path()).unwrap();
    s.write("a.txt", b"12345").unwrap();
    s.mkdir("dir").unwrap();
    assert_eq!(s.size("a.txt").unwrap(), 5);
    assert!(s.size("dir").is_err());
    assert!(s.size("missing").is_err());
}
