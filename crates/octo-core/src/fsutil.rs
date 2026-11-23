use std::fs::{File, OpenOptions};
use std::path::Path;

/// Creates a new file for writing, failing if it exists. Owner-only (0600) on unix;
/// on Windows files inherit the per-user ACL of the data directory.
pub fn create_new_private(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Moves a fully written `tmp` to `path`, never overwriting an existing `path`
/// (`AlreadyExists` then). A hard link does this atomically; Android (SELinux) denies hard
/// links in app storage, so on `PermissionDenied` it renames instead if `path` is absent.
/// The caller removes `tmp` afterwards.
pub fn place_new(tmp: &Path, path: &Path) -> std::io::Result<()> {
    place_new_with(tmp, path, |a, b| std::fs::hard_link(a, b))
}

fn place_new_with(
    tmp: &Path,
    path: &Path,
    link: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    match link(tmp, path) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(std::io::ErrorKind::AlreadyExists.into());
            }
            std::fs::rename(tmp, path)
        }
        r => r,
    }
}

/// Current unix time in milliseconds (0 if the clock is before the epoch).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    fn denied(_: &Path, _: &Path) -> std::io::Result<()> {
        Err(Error::from(ErrorKind::PermissionDenied))
    }

    fn setup(content: &[u8]) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let tmp = d.path().join("key.tmp");
        std::fs::write(&tmp, content).unwrap();
        let path = d.path().join("key");
        (d, tmp, path)
    }

    #[test]
    fn place_new_puts_the_file_in_place() {
        let (_d, tmp, path) = setup(b"new");
        place_new(&tmp, &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn place_new_never_overwrites() {
        let (_d, tmp, path) = setup(b"new");
        std::fs::write(&path, b"old").unwrap();
        let e = place_new(&tmp, &path).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    /// Android (SELinux) refuses hard links in app storage with EACCES.
    #[test]
    fn place_new_falls_back_to_rename_when_links_are_denied() {
        let (_d, tmp, path) = setup(b"new");
        place_new_with(&tmp, &path, denied).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn place_new_fallback_never_overwrites() {
        let (_d, tmp, path) = setup(b"new");
        std::fs::write(&path, b"old").unwrap();
        let e = place_new_with(&tmp, &path, denied).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
    }

    #[test]
    fn place_new_passes_other_link_errors_through() {
        let (_d, tmp, path) = setup(b"new");
        let e = place_new_with(&tmp, &path, |_, _| Err(Error::other("disk on fire"))).unwrap_err();
        assert_eq!(e.to_string(), "disk on fire");
        assert!(!path.exists());
    }
}
