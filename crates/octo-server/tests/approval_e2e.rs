#![cfg(unix)] // bash fake CLI with /dev/tcp
use octo_core::providers::{AgentEvent, ProviderKind};
use octo_core::storage::{Approval, Role, Space};
use octo_core::{Config, Hub};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;

struct Env {
    hub: Arc<Hub>,
    _dir: tempfile::TempDir,
}

/// Real listener + fake Claude that calls the MCP endpoint named in its `--mcp-config`.
async fn env(with_endpoint: bool) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let fake = PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_claude_mcp.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake.clone(),
        agy_bin: fake,
        openrouter_base: String::new(),
        local_api: with_endpoint.then(|| listener.local_addr().unwrap()),
        ..Config::new(None)
    };
    let hub = Arc::new(Hub::new(cfg, "test-node").unwrap());
    let app = octo_server::app(hub.clone(), Default::default());
    tokio::spawn(octo_server::serve(listener, app, std::future::pending()));
    Env { hub, _dir: dir }
}

fn thread(e: &Env, approval: Approval) -> String {
    e.hub
        .create_thread_in(ProviderKind::Claude, Space::Public, approval, None, None)
        .unwrap()
        .id
}

async fn next(rx: &mut Receiver<AgentEvent>) -> Option<AgentEvent> {
    tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("event within 15 s")
}

async fn until_approval(rx: &mut Receiver<AgentEvent>) -> (String, String, String) {
    while let Some(e) = next(rx).await {
        if let AgentEvent::Approval {
            id, tool, summary, ..
        } = e
        {
            return (id, tool, summary);
        }
        assert!(
            !matches!(e, AgentEvent::Done | AgentEvent::Error { .. }),
            "{e:?} before any approval"
        );
    }
    panic!("stream ended before any approval");
}

async fn rest(rx: &mut Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut v = vec![];
    while let Some(e) = next(rx).await {
        v.push(e);
    }
    v
}

fn text_of(events: &[AgentEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assistant(e: &Env, id: &str) -> String {
    e.hub
        .list_messages(id)
        .unwrap()
        .into_iter()
        .rfind(|m| m.role == Role::Assistant)
        .map(|m| m.content)
        .unwrap_or_default()
}

#[tokio::test]
async fn confirm_allow_round_trip() {
    let e = env(true).await;
    let id = thread(&e, Approval::Confirm);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    let (aid, tool, summary) = until_approval(&mut rx).await;
    assert_eq!((tool.as_str(), summary.as_str()), ("Bash", "Bash: ls"));
    let pending = e.hub.pending_approvals(&id);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, aid);
    e.hub.resolve_approval(&id, &aid, true).unwrap();
    let events = rest(&mut rx).await;
    let text = text_of(&events);
    assert!(text.contains("> Allowed: Bash: ls"), "{text}");
    assert!(text.contains("RESULT:ALLOWED"), "{text}");
    assert!(
        matches!(events.last(), Some(AgentEvent::Done)),
        "{events:?}"
    );
    let saved = assistant(&e, &id);
    assert!(saved.contains("> Allowed: Bash: ls"), "{saved}");
    assert!(saved.contains("RESULT:ALLOWED"), "{saved}");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AgentEvent::ApprovalDone { id } if *id == aid)),
        "{events:?}"
    );
    assert!(e.hub.pending_approvals(&id).is_empty());
    assert!(e.hub.run_token(&id).is_none());
}

#[tokio::test]
async fn confirm_deny_round_trip() {
    let e = env(true).await;
    let id = thread(&e, Approval::Confirm);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    let (aid, _, _) = until_approval(&mut rx).await;
    e.hub.resolve_approval(&id, &aid, false).unwrap();
    let events = rest(&mut rx).await;
    let text = text_of(&events);
    assert!(text.contains("> Denied: Bash: ls"), "{text}");
    assert!(text.contains("RESULT:DENIED"), "{text}");
    assert!(assistant(&e, &id).contains("> Denied: Bash: ls"));
}

#[tokio::test]
async fn auto_mode_never_asks() {
    let e = env(true).await;
    let id = thread(&e, Approval::Auto);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    let events = rest(&mut rx).await;
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::Approval { .. })),
        "{events:?}"
    );
    assert!(text_of(&events).contains("NOMCP"));
}

#[tokio::test]
async fn confirm_without_a_bound_api_stays_non_interactive() {
    let e = env(false).await;
    let id = thread(&e, Approval::Confirm);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    let events = rest(&mut rx).await;
    assert!(text_of(&events).contains("NOMCP"));
}

#[tokio::test]
async fn cancel_while_waiting_denies_and_ends_without_done() {
    let e = env(true).await;
    let id = thread(&e, Approval::Confirm);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    until_approval(&mut rx).await;
    e.hub.cancel(&id);
    let events = rest(&mut rx).await;
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::Done)),
        "{events:?}"
    );
    assert!(e.hub.pending_approvals(&id).is_empty());
    for _ in 0..100 {
        if e.hub.run_token(&id).is_none() && !e.hub.is_running(&id) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run did not end after cancel");
}

#[tokio::test]
async fn timeout_denies_a_forgotten_request() {
    let dir = tempfile::tempdir().unwrap();
    let fake = PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_claude_mcp.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake.clone(),
        agy_bin: fake,
        openrouter_base: String::new(),
        local_api: Some(listener.local_addr().unwrap()),
        approval_timeout: Duration::from_millis(300),
        ..Config::new(None)
    };
    let hub = Arc::new(Hub::new(cfg, "test-node").unwrap());
    tokio::spawn(octo_server::serve(
        listener,
        octo_server::app(hub.clone(), Default::default()),
        std::future::pending(),
    ));
    let t = hub
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    let mut rx = hub.send_message(&t.id, "go", &[]).await.unwrap();
    until_approval(&mut rx).await;
    let events = rest(&mut rx).await;
    assert!(text_of(&events).contains("RESULT:DENIED"), "{events:?}");
    assert!(hub.pending_approvals(&t.id).is_empty());
}

#[tokio::test]
async fn a_dropped_mcp_request_is_forgotten_and_announced() {
    let e = env(true).await;
    let id = thread(&e, Approval::Confirm);
    let mut rx = e.hub.send_message(&id, "go", &[]).await.unwrap();
    let (first, _, _) = until_approval(&mut rx).await;
    let token = e.hub.run_token(&id).unwrap();
    let dropped = tokio::time::timeout(
        Duration::from_millis(200),
        e.hub
            .mcp_approve(&token, "Bash", &serde_json::json!({"command": "pwd"})),
    )
    .await;
    assert!(dropped.is_err(), "still waiting for a decision");
    let pending = e.hub.pending_approvals(&id);
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].id, first);
    // the UI is told: an Approval for the dropped request, then its ApprovalDone
    let mut asked = vec![];
    let mut done = vec![];
    while done.is_empty() {
        match next(&mut rx).await {
            Some(AgentEvent::Approval { id, .. }) => asked.push(id),
            Some(AgentEvent::ApprovalDone { id }) => done.push(id),
            Some(_) => {}
            None => panic!("stream ended"),
        }
    }
    assert_eq!(asked.last(), done.first());
    e.hub.resolve_approval(&id, &first, true).unwrap();
    rest(&mut rx).await;
}
