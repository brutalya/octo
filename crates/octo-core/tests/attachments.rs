use octo_core::CoreError;
use octo_core::attachments::{MAX_TEXT_BYTES, build_prompt, decode_text};

#[test]
fn prompt_without_attachments_is_unchanged() {
    assert_eq!(build_prompt("hi", &[]), "hi");
}

#[test]
fn inlines_as_fenced_blocks_with_path_header() {
    let p = build_prompt("explain", &[("notes/a.txt".into(), "line1\nline2".into())]);
    assert_eq!(p, "explain\n\nnotes/a.txt:\n```\nline1\nline2\n```");
}

#[test]
fn multiple_attachments_keep_order() {
    let p = build_prompt("q", &[("a".into(), "1".into()), ("b".into(), "2".into())]);
    assert!(p.find("a:\n").unwrap() < p.find("b:\n").unwrap());
}

#[test]
fn fence_grows_when_content_has_backticks() {
    let p = build_prompt("q", &[("a.md".into(), "x\n```rust\ny\n```\n".into())]);
    assert!(p.contains("\n````\nx\n```rust"), "{p}");
    assert!(p.ends_with("\n````"), "{p}");
}

#[test]
fn decode_accepts_utf8_up_to_limit() {
    assert_eq!(decode_text("a.txt", "привет".as_bytes()).unwrap(), "привет");
    assert!(decode_text("a.txt", &vec![b'a'; MAX_TEXT_BYTES]).is_ok());
    assert_eq!(MAX_TEXT_BYTES, 200 * 1024);
}

#[test]
fn decode_rejects_big_binary_and_invalid_utf8() {
    for data in [
        vec![b'a'; MAX_TEXT_BYTES + 1],
        vec![0xff, 0xfe, 0x00],
        b"a\0b".to_vec(),
    ] {
        assert!(matches!(
            decode_text("x.bin", &data),
            Err(CoreError::UnsupportedMedia(_))
        ));
    }
}
