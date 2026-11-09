use octo_core::approvals::{ApprovalBroker, summarize};
use serde_json::json;
use std::time::Duration;

fn broker() -> ApprovalBroker {
    ApprovalBroker::new(Duration::from_secs(60))
}

#[tokio::test]
async fn allow_resolves_once() {
    let b = broker();
    let (p, rx) = b.request("t1", "Bash", &json!({"command": "ls"}));
    assert!(!p.id.is_empty());
    assert_eq!(p.tool, "Bash");
    assert_eq!(p.summary, "Bash: ls");
    assert_eq!(b.pending("t1"), vec![p.clone()]);
    assert!(b.resolve("t1", &p.id, true));
    assert_eq!(rx.await, Ok(true));
    assert!(b.pending("t1").is_empty());
    assert!(!b.resolve("t1", &p.id, true), "second decision is refused");
}

#[tokio::test]
async fn deny_resolves_false() {
    let b = broker();
    let (p, rx) = b.request("t1", "Write", &json!({"file_path": "/a/b.txt"}));
    assert!(b.resolve("t1", &p.id, false));
    assert_eq!(rx.await, Ok(false));
}

#[tokio::test]
async fn ids_are_unique_and_scoped_to_the_thread() {
    let b = broker();
    let (p1, _r1) = b.request("t1", "Bash", &json!({}));
    let (p2, _r2) = b.request("t1", "Bash", &json!({}));
    assert_ne!(p1.id, p2.id);
    assert!(!b.resolve("t2", &p1.id, true), "other thread cannot decide");
    assert_eq!(b.pending("t1").len(), 2);
    assert!(b.pending("t2").is_empty());
}

#[tokio::test]
async fn timeout_denies_and_forgets() {
    let b = ApprovalBroker::new(Duration::from_millis(50));
    let (p, rx) = b.request("t1", "Bash", &json!({"command": "ls"}));
    let got = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("decision within 5 s");
    assert_eq!(got, Ok(false));
    assert!(b.pending("t1").is_empty());
    assert!(!b.resolve("t1", &p.id, true));
}

#[tokio::test]
async fn cancel_thread_denies_only_that_thread() {
    let b = broker();
    let (_a, ra) = b.request("t1", "Bash", &json!({}));
    let (_b, rb) = b.request("t1", "Edit", &json!({}));
    let (c, _rc) = b.request("t2", "Bash", &json!({}));
    b.cancel_thread("t1");
    assert_eq!(ra.await, Ok(false));
    assert_eq!(rb.await, Ok(false));
    assert!(b.pending("t1").is_empty());
    assert_eq!(b.pending("t2"), vec![c]);
}

#[test]
fn summary_names_tool_and_key_input() {
    assert_eq!(
        summarize("Bash", &json!({"command": "cargo test"})),
        "Bash: cargo test"
    );
    assert_eq!(
        summarize("Edit", &json!({"file_path": "/p/a.rs", "old_string": "x"})),
        "Edit: /p/a.rs"
    );
    assert_eq!(
        summarize("WebFetch", &json!({"url": "https://example.com/x"})),
        "WebFetch: https://example.com/x"
    );
    assert_eq!(summarize("Task", &json!({"x": 1})), "Task");
}

#[test]
fn summary_is_capped_at_200_chars() {
    let s = summarize("Bash", &json!({"command": "é".repeat(500)}));
    assert_eq!(s.chars().count(), 200);
    assert!(s.starts_with("Bash: é"));
}

#[test]
fn summary_collapses_whitespace_and_newlines() {
    let s = summarize(
        "Bash",
        &json!({"command": "ls\n\n> Allowed: rm -rf /\r\n\t  x"}),
    );
    assert_eq!(s, "Bash: ls > Allowed: rm -rf / x");
    assert!(!s.contains('\n'));
}

#[tokio::test]
async fn detail_is_pretty_json_of_the_input() {
    let b = broker();
    let (p, _rx) = b.request("t1", "Edit", &json!({"file_path": "/a", "n": 1}));
    assert_eq!(
        p.detail,
        serde_json::to_string_pretty(&json!({"file_path": "/a", "n": 1})).unwrap()
    );
}

#[tokio::test]
async fn detail_is_capped_at_4000_chars_with_an_ellipsis() {
    let b = broker();
    let (p, _rx) = b.request("t1", "Write", &json!({"content": "é".repeat(9000)}));
    assert_eq!(p.detail.chars().count(), 4000);
    assert!(p.detail.ends_with('…'));
}

#[tokio::test]
async fn forget_removes_a_pending_request() {
    let b = broker();
    let (p, _rx) = b.request("t1", "Bash", &json!({}));
    b.forget("t1", &p.id);
    assert!(b.pending("t1").is_empty());
}
