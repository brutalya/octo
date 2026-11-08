#![cfg(unix)] // shell-script based; unix only
use octo_core::providers::lines::{LineBuffer, MAX_LINE, Pumped, pump_with};
use octo_core::providers::{AgentEvent, ProviderKind};
use octo_core::registry::ProcessRegistry;
use octo_core::run::cli_events;
use octo_core::settings::SettingsPatch;
use octo_core::storage::{Approval, Space};
use octo_core::{Config, CoreError, Hub};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}/../../tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn hub_with(root: &Path, claude: PathBuf) -> Hub {
    let mut cfg = Config::new(Some(root.join("data")));
    cfg.claude_bin = claude;
    cfg.agy_bin = root.join("no-such-agy");
    cfg.claude_projects_dir = root.join("none");
    cfg.agy_dir = root.join("none");
    Hub::new(cfg, "test-node").unwrap()
}

fn new_cli(hub: &Hub, cwd: Option<&str>) -> Result<octo_core::storage::Thread, CoreError> {
    hub.create_thread_in(
        ProviderKind::Claude,
        Space::Public,
        Approval::Confirm,
        None,
        cwd,
    )
}

#[tokio::test]
async fn cli_cwd_inside_data_dir_is_rejected_except_public() {
    let root = tempfile::tempdir().unwrap();
    let hub = hub_with(root.path(), fixture("pwd_cli.sh"));
    let data = root.path().join("data");
    for bad in [
        data.clone(),
        data.join("private"),
        data.join("keys"),
        data.join("public").join("..").join("keys"),
    ] {
        std::fs::create_dir_all(&bad).unwrap();
        let r = new_cli(&hub, bad.to_str());
        assert!(matches!(r, Err(CoreError::Policy(_))), "{bad:?}");
    }
    // the default (public dir) and anything outside are fine
    assert!(new_cli(&hub, None).is_ok());
    let outside = root.path().join("work");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(new_cli(&hub, outside.to_str()).is_ok());
}

#[tokio::test]
async fn private_ollama_requires_a_local_url() {
    let root = tempfile::tempdir().unwrap();
    let hub = hub_with(root.path(), fixture("pwd_cli.sh"));
    let set = |u: &str| {
        hub.update_settings(SettingsPatch {
            ollama_url: Some(u.into()),
            ..Default::default()
        })
        .unwrap()
    };
    let mk = || {
        hub.create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
    };
    assert!(mk().is_ok(), "default URL is loopback");
    set("http://192.168.1.20:11434");
    let t = mk().unwrap();
    set("http://ollama.example.com:11434");
    assert!(matches!(mk(), Err(CoreError::Policy(_))));
    // an existing private thread cannot be used once Ollama points elsewhere
    let r = hub.send_message(&t.id, "hi", &[]).await;
    assert!(matches!(r, Err(CoreError::Policy(_))));
    // public threads are unaffected
    assert!(
        hub.create_thread_in(
            ProviderKind::Ollama,
            Space::Public,
            Approval::Confirm,
            Some("m"),
            None
        )
        .is_ok()
    );
    let statuses = hub.providers().await;
    let o = statuses
        .iter()
        .find(|s| s.kind == ProviderKind::Ollama)
        .unwrap();
    assert!(!o.is_local);
}

#[tokio::test]
async fn failed_spawn_stores_nothing() {
    let root = tempfile::tempdir().unwrap();
    let hub = hub_with(root.path(), root.path().join("no-such-binary"));
    let t = new_cli(&hub, None).unwrap();
    assert!(hub.send_message(&t.id, "hello", &[]).await.is_err());
    assert!(hub.list_messages(&t.id).unwrap().is_empty());
    assert!(!hub.is_running(&t.id));
}

#[tokio::test]
async fn providers_detects_versions_through_managed_processes() {
    let root = tempfile::tempdir().unwrap();
    let hub = hub_with(root.path(), fixture("fake_cli.sh"));
    let statuses = hub.providers().await;
    let c = statuses
        .iter()
        .find(|s| s.kind == ProviderKind::Claude)
        .unwrap();
    assert!(c.installed);
    assert_eq!(c.version.as_deref(), Some("9.9.9 (fake)"));
    let a = statuses
        .iter()
        .find(|s| s.kind == ProviderKind::Antigravity)
        .unwrap();
    assert!(!a.installed, "agy is not installed in the test config");
}

fn sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(script);
    c
}

#[tokio::test]
async fn lingering_process_is_cancelled_after_terminal_event() {
    let reg = ProcessRegistry::default();
    let mut h = reg
        .spawn(
            "k",
            sh("printf '{\"type\":\"result\",\"is_error\":false}\\n'; exec sleep 300"),
            String::new(),
        )
        .unwrap();
    let pid = h.pid.unwrap();
    let _ = &mut h;
    let mut rx = cli_events(ProviderKind::Claude, h, reg.clone(), "k".into());
    assert_eq!(rx.recv().await, Some(AgentEvent::Done));
    // channel closes after the 2 s grace, and the process is gone
    let closed = tokio::time::timeout(Duration::from_secs(6), rx.recv()).await;
    assert_eq!(closed, Ok(None));
    for _ in 0..100 {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("process still alive");
}

#[test]
fn line_buffer_caps_line_length() {
    let mut b = LineBuffer::default();
    let chunk = vec![b'x'; 1024 * 1024];
    for _ in 0..(MAX_LINE / chunk.len()) {
        assert!(b.push(&chunk).is_empty());
        assert!(!b.too_long());
    }
    b.push(b"y");
    assert!(b.too_long(), "no newline within the cap");
    assert!(b.push(b"\nok\n").is_empty(), "stays failed");

    let mut b = LineBuffer::default();
    let mut big = vec![b'x'; MAX_LINE + 1];
    big.push(b'\n');
    b.push(&big);
    assert!(b.too_long(), "long line terminated in the same chunk");
}

#[test]
fn line_buffer_many_lines_in_one_chunk() {
    let mut b = LineBuffer::default();
    let data: String = (0..50_000).map(|i| format!("l{i}\n")).collect();
    let lines = b.push(data.as_bytes());
    assert_eq!(lines.len(), 50_000);
    assert_eq!(lines[49_999], "l49999");
    assert!(b.push(b"a").is_empty());
    assert_eq!(b.push(b"b\n"), vec!["ab"]);
}

#[tokio::test]
async fn stalled_stream_times_out_with_an_error() {
    use axum::body::{Body, Bytes};
    use axum::response::IntoResponse;
    use futures_util::StreamExt;
    let app = axum::Router::new().route(
        "/hang",
        axum::routing::get(|| async {
            let first = tokio_stream::iter([Ok::<Bytes, std::convert::Infallible>(Bytes::from(
                "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
            ))]);
            Body::from_stream(first.chain(futures_util::stream::pending())).into_response()
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let resp = reqwest::get(format!("http://{addr}/hang")).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let end = pump_with(
        resp,
        octo_core::providers::openrouter::parse_line,
        |_| false,
        Duration::from_millis(300),
        &tx,
    )
    .await;
    assert_eq!(end, Pumped::Terminal);
    assert_eq!(
        rx.recv().await,
        Some(AgentEvent::TextDelta { text: "hi".into() })
    );
    assert!(matches!(rx.recv().await, Some(AgentEvent::Error { .. })));
}
