use octo_core::providers::ProviderKind;
use octo_core::storage::{Approval, Origin, Role, Space};
use octo_core::{Config, CoreError, Hub};
use std::path::{Path, PathBuf};

const S1: &str = "11111111-1111-4111-8111-111111111111";
const A1: &str = "aaaaaaaa-0000-4000-8000-000000000001";

fn fixtures() -> PathBuf {
    PathBuf::from(format!(
        "{}/../../tests/fixtures/import",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

struct Env {
    hub: Hub,
    root: tempfile::TempDir,
}

/// Hub over a private copy of the synthetic CLI storage.
fn env() -> Env {
    let root = tempfile::tempdir().unwrap();
    copy_dir(
        &fixtures().join("claude/projects"),
        &root.path().join("projects"),
    );
    copy_dir(&fixtures().join("agy"), &root.path().join("agy"));
    // The sqlite index is generated from its committed SQL dump (*.db is gitignored).
    let sql = std::fs::read_to_string(fixtures().join("agy/conversation_summaries.sql")).unwrap();
    rusqlite::Connection::open(root.path().join("agy/conversation_summaries.db"))
        .unwrap()
        .execute_batch(&sql)
        .unwrap();
    let mut cfg = Config::new(Some(root.path().join("data")));
    cfg.claude_projects_dir = root.path().join("projects");
    cfg.agy_dir = root.path().join("agy");
    Env {
        hub: Hub::new(cfg, "test-node").unwrap(),
        root,
    }
}

#[test]
fn sessions_are_searchable_by_title_and_cwd() {
    let e = env();
    let all = e.hub.import_sessions(ProviderKind::Claude, "").unwrap();
    assert_eq!(all.len(), 2);
    let q = e
        .hub
        .import_sessions(ProviderKind::Claude, "WINDOWS")
        .unwrap();
    assert_eq!(q.len(), 1);
    let q = e
        .hub
        .import_sessions(ProviderKind::Claude, "srv/demo")
        .unwrap();
    assert_eq!(q.len(), 1);
    assert_eq!(q[0].external_id, S1);
    assert!(
        e.hub
            .import_sessions(ProviderKind::Claude, "zzzz")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn non_cli_kinds_cannot_import() {
    let e = env();
    assert!(matches!(
        e.hub.import_sessions(ProviderKind::Ollama, ""),
        Err(CoreError::Invalid(_))
    ));
    assert!(matches!(
        e.hub.import_session(ProviderKind::OpenRouter, "x"),
        Err(CoreError::Invalid(_))
    ));
}

#[test]
fn import_is_idempotent_and_marks_the_listing() {
    let e = env();
    let a = e.hub.import_session(ProviderKind::Claude, S1).unwrap();
    let b = e.hub.import_session(ProviderKind::Claude, S1).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(e.hub.list_threads().unwrap().len(), 1);
    assert_eq!(a.origin, Origin::Imported);
    assert_eq!(a.space, Space::Public);
    assert_eq!(a.cwd.as_deref(), Some("/srv/demo"));
    assert_eq!(a.external_session_id.as_deref(), Some(S1));
    assert!(a.title.starts_with("Add a retry"));
    assert_eq!(e.hub.list_messages(&a.id).unwrap().len(), 4);
    let list = e.hub.import_sessions(ProviderKind::Claude, "").unwrap();
    let s = list.iter().find(|s| s.external_id == S1).unwrap();
    assert_eq!(s.imported_thread_id.as_deref(), Some(a.id.as_str()));
    assert!(list.iter().any(|s| s.imported_thread_id.is_none()));
}

#[test]
fn unknown_session_is_not_found() {
    let e = env();
    assert!(matches!(
        e.hub.import_session(ProviderKind::Claude, "nope"),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        e.hub.import_session(ProviderKind::Antigravity, "nope"),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn opening_refreshes_cache_when_the_source_changed() {
    let e = env();
    let t = e.hub.import_session(ProviderKind::Claude, S1).unwrap();
    assert_eq!(e.hub.list_messages(&t.id).unwrap().len(), 4);
    let file = e.root.path().join(format!("projects/-srv-demo/{S1}.jsonl"));
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(
        "{\"type\":\"user\",\"isSidechain\":false,\"cwd\":\"/srv/demo\",\"timestamp\":\"2026-03-01T11:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"one more\"}}\n",
    );
    std::fs::write(&file, text).unwrap();
    let m = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(m.len(), 5);
    assert_eq!(m[4].content, "one more");
    assert_eq!(m[4].role, Role::User);
    // unchanged source: same cache
    assert_eq!(e.hub.list_messages(&t.id).unwrap().len(), 5);
}

#[test]
fn vanished_source_keeps_the_cache() {
    let e = env();
    let t = e.hub.import_session(ProviderKind::Claude, S1).unwrap();
    std::fs::remove_file(e.root.path().join(format!("projects/-srv-demo/{S1}.jsonl"))).unwrap();
    assert_eq!(e.hub.list_messages(&t.id).unwrap().len(), 4);
}

#[test]
fn antigravity_import_shows_prompts_and_unavailable_line() {
    let e = env();
    let t = e.hub.import_session(ProviderKind::Antigravity, A1).unwrap();
    assert_eq!(t.title, "Refactor parser");
    assert_eq!(t.cwd.as_deref(), Some("/srv/demo"));
    let m = e.hub.list_messages(&t.id).unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(m[2].role, Role::Assistant);
}

#[test]
fn new_cli_threads_default_to_the_public_dir() {
    let e = env();
    let t = e
        .hub
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    let public = e.hub.files().public_dir().to_string_lossy().into_owned();
    assert_eq!(t.cwd.as_deref(), Some(public.as_str()));
    let own = e
        .hub
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            Some("/x/y"),
        )
        .unwrap();
    assert_eq!(own.cwd.as_deref(), Some("/x/y"));
    let http = e
        .hub
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    assert_eq!(http.cwd, None);
}

#[cfg(unix)]
mod runs {
    use super::*;

    fn pwd_hub() -> (Hub, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(Some(root.path().join("data")));
        cfg.claude_bin = PathBuf::from(format!(
            "{}/../../tests/fixtures/pwd_cli.sh",
            env!("CARGO_MANIFEST_DIR")
        ));
        cfg.claude_projects_dir = root.path().join("none");
        cfg.agy_dir = root.path().join("none");
        (Hub::new(cfg, "test-node").unwrap(), root)
    }

    async fn run(hub: &Hub, id: &str) -> String {
        let mut rx = hub.send_message(id, "where", &[]).await.unwrap();
        while rx.recv().await.is_some() {}
        hub.list_messages(id)
            .unwrap()
            .into_iter()
            .find(|m| m.role == Role::Assistant)
            .unwrap()
            .content
    }

    #[tokio::test]
    async fn run_uses_the_thread_cwd() {
        let (hub, root) = pwd_hub();
        let work = root.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let t = hub
            .create_thread_in(
                ProviderKind::Claude,
                Space::Public,
                Approval::Confirm,
                None,
                work.to_str(),
            )
            .unwrap();
        assert_eq!(
            run(&hub, &t.id).await,
            work.canonicalize().unwrap().to_string_lossy()
        );
    }

    #[tokio::test]
    async fn run_defaults_to_the_public_dir() {
        let (hub, _root) = pwd_hub();
        let t = hub
            .create_thread_in(
                ProviderKind::Claude,
                Space::Public,
                Approval::Confirm,
                None,
                None,
            )
            .unwrap();
        let public = hub.files().public_dir().canonicalize().unwrap();
        assert_eq!(run(&hub, &t.id).await, public.to_string_lossy());
    }

    #[tokio::test]
    async fn missing_cwd_fails_fast() {
        let (hub, root) = pwd_hub();
        let gone = root.path().join("gone");
        let t = hub
            .create_thread_in(
                ProviderKind::Claude,
                Space::Public,
                Approval::Confirm,
                None,
                gone.to_str(),
            )
            .unwrap();
        assert!(matches!(
            hub.send_message(&t.id, "x", &[]).await,
            Err(CoreError::Invalid(_))
        ));
        assert!(hub.list_messages(&t.id).unwrap().is_empty());
    }
}
