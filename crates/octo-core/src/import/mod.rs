//! Read-only readers for the CLIs' own session storage.
pub mod antigravity;
pub mod claude;
pub mod time;

use crate::storage::SourceMessage;
use serde::Serialize;
use ts_rs::TS;

/// A CLI session as offered in the import picker.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct SessionSummary {
    pub external_id: String,
    pub title: String,
    pub cwd: Option<String>,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number")]
    pub messages: usize,
    pub imported_thread_id: Option<String>,
}

/// One session found in a CLI's storage.
#[derive(Debug, Clone)]
pub struct Session {
    pub external_id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub updated_at: i64,
    pub messages: usize,
    /// Changes whenever the source content changes; used to refresh the cache.
    pub stamp: String,
}

#[derive(Debug, Clone)]
pub struct SessionData {
    pub session: Session,
    pub messages: Vec<SourceMessage>,
}

/// Session ids are used to look files up; only plain id characters are accepted.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) fn short_title(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect()
}

/// `file:///srv/x` -> `/srv/x`, `file:///C:/a%20b` -> `C:/a b`. Plain paths pass through.
pub(crate) fn uri_to_path(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_string();
    };
    let decoded = percent_decode(rest);
    let b = decoded.as_bytes();
    if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
        decoded[1..].to_string()
    } else {
        decoded
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_become_paths() {
        assert_eq!(uri_to_path("file:///srv/demo"), "/srv/demo");
        assert_eq!(uri_to_path("file:///C:/work/my%20app"), "C:/work/my app");
        assert_eq!(uri_to_path("/plain"), "/plain");
    }

    #[test]
    fn ids_are_plain() {
        assert!(valid_id("abc-1_2"));
        for bad in ["", "../x", "a/b", "a.b"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }
}
