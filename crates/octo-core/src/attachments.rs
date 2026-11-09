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

pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// What a file is, decided from its first bytes (never from the extension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// MIME type of a supported image.
    Image(&'static str),
    Pdf,
    Other,
}

/// A stored file ready to be used by a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    Text(String),
    Image { mime: &'static str, data: Vec<u8> },
}

pub const MAX_PDF_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_PDF_PAGES: usize = 300;
/// Largest file ever read for an attachment: the biggest per-kind cap.
pub const MAX_READ_BYTES: usize = MAX_PDF_BYTES;
pub const MAX_ATTACHMENTS: usize = 10;
pub const MAX_TOTAL_BYTES: usize = 40 * 1024 * 1024;

pub fn sniff(data: &[u8]) -> Kind {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if data.starts_with(PNG) {
        Kind::Image("image/png")
    } else if data.starts_with(&[0xff, 0xd8, 0xff]) {
        Kind::Image("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Kind::Image("image/gif")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Kind::Image("image/webp")
    } else if data.starts_with(b"%PDF-") {
        Kind::Pdf
    } else if data.contains(&0) || std::str::from_utf8(data).is_err() {
        Kind::Other
    } else {
        Kind::Text
    }
}

/// Sniffs and validates a file: text, image (<= 10 MB) or PDF (text extracted).
pub fn load(path: &str, data: &[u8]) -> Result<Loaded, CoreError> {
    match sniff(data) {
        Kind::Text => decode_text(path, data).map(Loaded::Text),
        Kind::Image(mime) if data.len() <= MAX_IMAGE_BYTES => Ok(Loaded::Image {
            mime,
            data: data.to_vec(),
        }),
        Kind::Image(_) => Err(CoreError::UnsupportedMedia(format!(
            "{path}: image larger than 10 MB"
        ))),
        Kind::Pdf => pdf_text(path, data).map(Loaded::Text),
        Kind::Other => Err(CoreError::UnsupportedMedia(format!(
            "{path}: unsupported file type"
        ))),
    }
}

const TRUNCATED: &str = "[truncated]";

/// Extracted text of a PDF, capped at 200 KB with a marker line. No text layer is an error.
/// Blocking and CPU-heavy: call from `spawn_blocking`.
pub fn pdf_text(path: &str, data: &[u8]) -> Result<String, CoreError> {
    let unsupported = |why: &str| CoreError::UnsupportedMedia(format!("{path}: {why}"));
    if data.len() > MAX_PDF_BYTES {
        return Err(unsupported("PDF larger than 20 MB"));
    }
    // the parser may panic on hostile input
    let extracted = std::panic::catch_unwind(|| {
        let pages = pdf_extract::Document::load_mem(data)
            .map_err(|_| "cannot read PDF")?
            .get_pages()
            .len();
        if pages > MAX_PDF_PAGES {
            return Err("PDF has more than 300 pages");
        }
        pdf_extract::extract_text_from_mem(data).map_err(|_| "cannot read PDF")
    })
    .map_err(|_| unsupported("cannot read PDF"))?
    .map_err(unsupported)?;
    let text = extracted.trim();
    if text.chars().all(char::is_whitespace) {
        return Err(unsupported("PDF has no text layer"));
    }
    if text.len() <= MAX_TEXT_BYTES {
        return Ok(text.to_string());
    }
    let mut end = MAX_TEXT_BYTES - TRUNCATED.len() - 1;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok(format!("{}\n{TRUNCATED}", &text[..end]))
}

/// Appends one `Attached image: <path>` line per image (for CLI providers).
pub fn with_image_paths(prompt: &str, paths: &[String]) -> String {
    let mut out = prompt.to_string();
    for (i, p) in paths.iter().enumerate() {
        out.push_str(if i == 0 { "\n\n" } else { "\n" });
        out.push_str("Attached image: ");
        out.push_str(p);
    }
    out
}
