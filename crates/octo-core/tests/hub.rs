#![cfg(unix)] // uses tests/fixtures/fake_cli.sh; unix only
use octo_core::providers::{AgentEvent, ProviderKind};
use octo_core::storage::{Approval, Space};
use octo_core::{Config, Hub};
use std::path::PathBuf;

#[tokio::test]
async fn no_done_once_cancel_requested() {
    let dir = tempfile::tempdir().unwrap();
    let fake = PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_cli.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let hub = Hub::new(
        Config {
            data_dir: dir.path().to_path_buf(),
            claude_bin: fake.clone(),
            agy_bin: fake,
            openrouter_base: String::new(),
            ..Config::new(None)
        },
        "test-node",
    )
    .unwrap();
    for _ in 0..20 {
        let t = hub
            .create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm, None)
            .unwrap();
        let mut rx = hub.send_message(&t.id, "hello", &[]).await.unwrap();
        hub.cancel(&t.id);
        let mut events = vec![];
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Done)),
            "{events:?}"
        );
    }
}
