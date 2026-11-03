use octo_core::provider::{AgentEvent, ProviderKind, parse_line};
use std::fs;

fn events(kind: ProviderKind, name: &str) -> Vec<AgentEvent> {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .flat_map(|l| parse_line(kind, l))
        .collect()
}

fn text(evs: &[AgentEvent]) -> String {
    evs.iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn claude_new() {
    let evs = events(ProviderKind::Claude, "claude_new.jsonl");
    assert!(matches!(&evs[0], AgentEvent::Session { id } if id.len() == 36));
    assert_eq!(text(&evs).trim(), "ok");
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
    assert_eq!(evs.iter().filter(|e| **e == AgentEvent::Done).count(), 1);
}

#[test]
fn claude_resume_keeps_session() {
    let a = events(ProviderKind::Claude, "claude_new.jsonl");
    let b = events(ProviderKind::Claude, "claude_resume.jsonl");
    assert_eq!(a[0], b[0]);
    assert_eq!(b.last(), Some(&AgentEvent::Done));
}

#[test]
fn claude_tool_use_and_error() {
    let l = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls -la"}}]}}"#;
    assert_eq!(
        parse_line(ProviderKind::Claude, l),
        vec![AgentEvent::Tool {
            name: "Bash".into(),
            summary: "ls -la".into()
        }]
    );
    let e = r#"{"type":"result","is_error":true,"result":"boom"}"#;
    assert_eq!(
        parse_line(ProviderKind::Claude, e),
        vec![AgentEvent::Error {
            message: "boom".into()
        }]
    );
}

#[test]
fn garbage_is_ignored() {
    assert!(parse_line(ProviderKind::Claude, "not json").is_empty());
    assert!(parse_line(ProviderKind::Antigravity, "").is_empty());
}

#[test]
fn antigravity_new() {
    let evs = events(ProviderKind::Antigravity, "antigravity_new.jsonl");
    assert!(matches!(&evs[0], AgentEvent::Session { id } if id.len() == 36));
    assert_eq!(text(&evs).trim(), "ok");
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
}

#[test]
fn antigravity_resume() {
    let evs = events(ProviderKind::Antigravity, "antigravity_resume.jsonl");
    assert!(matches!(&evs[0], AgentEvent::Session { .. }));
    assert!(!text(&evs).trim().is_empty());
    assert_eq!(evs.last(), Some(&AgentEvent::Done));
}

#[test]
fn antigravity_error_result() {
    let l = r#"{"event":"result","result":{"status":"ERROR","error":"bad"}}"#;
    assert_eq!(
        parse_line(ProviderKind::Antigravity, l),
        vec![AgentEvent::Error {
            message: "bad".into()
        }]
    );
}
