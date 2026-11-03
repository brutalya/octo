#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("path must be relative")]
    Absolute,
    #[error("path must not contain '..'")]
    Traversal,
    #[error("invalid path")]
    Invalid,
}

const MAX_COMPONENT: usize = 255;

/// Validates a user-supplied relative path and returns it normalized:
/// `/`-separated, no empty or `.` components, no trailing slash. The root is `""`.
pub fn normalize(path: &str) -> Result<String, PathError> {
    if path.starts_with('/') || has_drive_prefix(path) {
        return Err(PathError::Absolute);
    }
    if path.chars().any(|c| c == '\\' || c.is_control()) {
        return Err(PathError::Invalid);
    }
    let mut parts = vec![];
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => return Err(PathError::Traversal),
            c if c.len() > MAX_COMPONENT => return Err(PathError::Invalid),
            c => parts.push(c),
        }
    }
    Ok(parts.join("/"))
}

fn has_drive_prefix(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}
