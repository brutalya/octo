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
    if path
        .chars()
        .any(|c| matches!(c, '\\' | ':') || c.is_control())
    {
        return Err(PathError::Invalid);
    }
    let mut parts = vec![];
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => return Err(PathError::Traversal),
            c if c.len() > MAX_COMPONENT || is_reserved(c) => return Err(PathError::Invalid),
            c => parts.push(c),
        }
    }
    Ok(parts.join("/"))
}

fn has_drive_prefix(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Windows device names (`CON`, `NUL`, `COM1`, ... also with an extension) cannot be used
/// as file names there; refusing them everywhere keeps synced data portable.
fn is_reserved(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or("").trim_end();
    let u = stem.to_ascii_uppercase();
    matches!(
        u.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || (u.len() == 4
        && (u.starts_with("COM") || u.starts_with("LPT"))
        && matches!(u.as_bytes()[3], b'1'..=b'9'))
}
