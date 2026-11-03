use crate::error::CoreError;

pub const MAX_TEXT_BYTES: usize = 200 * 1024;

/// Text files only: UTF-8, no NUL bytes, at most 200 KB.
pub fn decode_text(path: &str, data: &[u8]) -> Result<String, CoreError> {
    let unsupported = |why: &str| CoreError::UnsupportedMedia(format!("{path}: {why}"));
    if data.len() > MAX_TEXT_BYTES {
        return Err(unsupported("larger than 200 KB"));
    }
    if data.contains(&0) {
        return Err(unsupported("not a text file"));
    }
    String::from_utf8(data.to_vec()).map_err(|_| unsupported("not valid UTF-8 text"))
}

/// Appends each file as a fenced block headed by its path.
pub fn build_prompt(content: &str, files: &[(String, String)]) -> String {
    let mut out = content.to_string();
    for (path, body) in files {
        let fence = fence_for(body);
        out.push_str(&format!("\n\n{path}:\n{fence}\n{body}\n{fence}"));
    }
    out
}

/// Shortest backtick fence (min 3) longer than any backtick run in the body.
fn fence_for(body: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in body.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}
