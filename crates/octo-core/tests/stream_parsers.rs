use octo_core::providers::AgentEvent::{Done, Error, TextDelta};
use octo_core::providers::lines::LineBuffer;
use octo_core::providers::{ollama, openrouter};

fn text(t: &str) -> octo_core::providers::AgentEvent {
    TextDelta { text: t.into() }
}

#[test]
fn line_buffer_joins_chunks_and_strips_cr() {
    let mut b = LineBuffer::default();
    assert_eq!(b.push(b"ab\ncd"), vec!["ab"]);
    assert_eq!(b.push(b"e\r\n\nf"), vec!["cde", ""]);
    assert_eq!(b.finish(), Some("f".to_string()));
    assert_eq!(b.finish(), None);
}

#[test]
fn line_buffer_keeps_multibyte_split_across_chunks() {
    let bytes = "привет\n".as_bytes();
    let mut b = LineBuffer::default();
    assert!(b.push(&bytes[..3]).is_empty());
    assert_eq!(b.push(&bytes[3..]), vec!["привет"]);
}

#[test]
fn ollama_content_and_done() {
    assert_eq!(
        ollama::parse_line(
            r#"{"model":"m","message":{"role":"assistant","content":"Hi"},"done":false}"#
        ),
        vec![text("Hi")]
    );
    assert_eq!(
        ollama::parse_line(
            r#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop"}"#
        ),
        vec![Done]
    );
    assert_eq!(
        ollama::parse_line(r#"{"message":{"content":"!"},"done":true}"#),
        vec![text("!"), Done]
    );
}

#[test]
fn ollama_error_and_garbage() {
    assert_eq!(
        ollama::parse_line(r#"{"error":"model 'x' not found"}"#),
        vec![Error {
            message: "model 'x' not found".into()
        }]
    );
    assert!(ollama::parse_line("").is_empty());
    assert!(ollama::parse_line("not json").is_empty());
}

#[test]
fn ollama_models_from_tags() {
    let m = ollama::parse_models(
        r#"{"models":[{"name":"llama3:8b","model":"llama3:8b"},{"name":"qwen"}]}"#,
    );
    assert_eq!(
        m.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["llama3:8b", "qwen"]
    );
    assert!(ollama::parse_models("oops").is_empty());
}

#[test]
fn openrouter_delta_done_and_comments() {
    assert_eq!(
        openrouter::parse_line(
            r#"data: {"choices":[{"delta":{"role":"assistant","content":"He"}}]}"#
        ),
        vec![text("He")]
    );
    assert!(openrouter::parse_line(": OPENROUTER PROCESSING").is_empty());
    assert!(openrouter::parse_line("").is_empty());
    assert!(openrouter::parse_line("event: ping").is_empty());
    assert!(
        openrouter::parse_line(
            r#"data: {"choices":[{"delta":{"content":""},"finish_reason":"stop"}]}"#
        )
        .is_empty()
    );
    assert_eq!(openrouter::parse_line("data: [DONE]"), vec![Done]);
    assert_eq!(openrouter::parse_line("data:[DONE]"), vec![Done]);
}

#[test]
fn openrouter_midstream_error() {
    assert_eq!(
        openrouter::parse_line(
            r#"data: {"error":{"code":429,"message":"rate limited"},"choices":[{"delta":{"content":""},"finish_reason":"error"}]}"#
        ),
        vec![Error {
            message: "rate limited".into()
        }]
    );
}

#[test]
fn openrouter_models() {
    let m = openrouter::parse_models(r#"{"data":[{"id":"a/b","name":"A: B"},{"id":"c/d"}]}"#);
    assert_eq!(m[0].name, "A: B");
    assert_eq!(m[1].name, "c/d");
}

#[test]
fn openrouter_http_error_message() {
    assert_eq!(
        openrouter::error_message(401, r#"{"error":{"message":"No auth","code":401}}"#),
        "No auth"
    );
    assert_eq!(openrouter::error_message(502, "<html>"), "HTTP 502");
}
