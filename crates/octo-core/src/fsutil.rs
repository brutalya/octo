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

/// Current unix time in milliseconds (0 if the clock is before the epoch).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
