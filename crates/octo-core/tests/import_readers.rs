use octo_core::import::{antigravity, claude};
use octo_core::storage::Role;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

fn fixtures() -> PathBuf {
    PathBuf::from(format!(
        "{}/../../tests/fixtures/import",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn projects() -> PathBuf {
    fixtures().join("claude/projects")
}

const S1: &str = "11111111-1111-4111-8111-111111111111";
const S2: &str = "22222222-2222-4222-8222-222222222222";

#[test]
fn claude_list_has_title_cwd_count_and_skips_empty() {
    let list = claude::list(&projects());
    let ids: Vec<_> = list.iter().map(|s| s.external_id.as_str()).collect();
    assert_eq!(list.len(), 2, "{ids:?}");
    let s1 = list.iter().find(|s| s.external_id == S1).unwrap();
    assert_eq!(s1.cwd.as_deref(), Some("/srv/demo"));
    assert_eq!(s1.messages, 4);
    assert_eq!(s1.title.chars().count(), 60);
    assert!(s1.title.starts_with("Add a retry to the fetch helper"));
    assert!(s1.updated_at > 0);
    let s2 = list.iter().find(|s| s.external_id == S2).unwrap();
    assert_eq!(s2.cwd.as_deref(), Some("C:\\work\\demo"));
    assert_eq!(s2.title, "Windows project question");
}

#[test]
fn claude_messages_keep_text_and_tool_lines_only() {
    let d = claude::read(&projects(), S1).unwrap();
    let m = &d.messages;
    assert_eq!(m.len(), 4);
    assert_eq!(m[0].role, Role::User);
    assert!(m[0].content.starts_with("Add a retry"));
    assert_eq!(m[0].created_at, 1_772_359_200_000);
    assert_eq!(m[1].role, Role::Assistant);
    assert_eq!(
        m[1].content,
        "Looking at the helper.\n\n> Tool: Read /srv/demo/fetch.rs\n\nDone: retry added."
    );
    assert_eq!(m[2].role, Role::User);
    assert_eq!(m[2].content, "Thanks, now add a test");
    assert_eq!(m[3].content, "Test added.");
    let all: String = m.iter().map(|m| m.content.as_str()).collect();
    for hidden in ["hidden", "sidechain", "caveat", "file body"] {
        assert!(!all.contains(hidden), "{hidden}");
    }
}

#[test]
fn claude_read_rejects_bad_or_unknown_ids() {
    assert!(claude::read(&projects(), "nope").is_none());
    assert!(claude::read(&projects(), "../x").is_none());
    assert!(claude::read(&projects(), "").is_none());
}

#[test]
fn claude_missing_dir_is_empty() {
    assert!(claude::list(Path::new("/definitely/not/here")).is_empty());
}

#[test]
fn claude_stamp_changes_with_content() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("-p");
    std::fs::create_dir_all(&p).unwrap();
    let f = p.join("abc.jsonl");
    let line = r#"{"type":"user","cwd":"/x","message":{"role":"user","content":"hi"}}"#;
    std::fs::write(&f, format!("{line}\n")).unwrap();
    let a = claude::read(dir.path(), "abc").unwrap().session.stamp;
    std::fs::write(&f, format!("{line}\n{line}\n")).unwrap();
    let b = claude::read(dir.path(), "abc").unwrap().session.stamp;
    assert_ne!(a, b);
}

/// Antigravity storage with its sqlite index generated from the committed SQL
/// dump (`*.db` is gitignored, so it cannot ship with the fixtures).
fn agy_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = fixtures().join("agy");
    std::fs::copy(src.join("history.jsonl"), dir.path().join("history.jsonl")).unwrap();
    let sql = std::fs::read_to_string(src.join("conversation_summaries.sql")).unwrap();
    rusqlite::Connection::open(dir.path().join("conversation_summaries.db"))
        .unwrap()
        .execute_batch(&sql)
        .unwrap();
    dir
}

const A1: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const A2: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const A3: &str = "aaaaaaaa-0000-4000-8000-000000000003";

#[test]
fn agy_list_uses_summaries_and_skips_subagents_and_empty() {
    let list = antigravity::list(agy_dir().path());
    let ids: Vec<_> = list.iter().map(|s| s.external_id.as_str()).collect();
    assert_eq!(ids, [A3, A2, A1], "newest first");
    let a1 = &list[2];
    assert_eq!(a1.title, "Refactor parser");
    assert_eq!(a1.cwd.as_deref(), Some("/srv/demo"));
    assert_eq!(a1.updated_at, 1_772_359_500_123);
    assert_eq!(list[0].title, "preview text");
    assert_eq!(list[0].cwd.as_deref(), Some("C:/work/my app"));
}

#[test]
fn agy_messages_are_history_prompts_plus_unavailable_line() {
    let d = antigravity::read(agy_dir().path(), A1).unwrap();
    let got: Vec<_> = d
        .messages
        .iter()
        .map(|m| (m.role, m.content.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (Role::User, "first prompt in demo"),
            (Role::User, "second prompt in demo"),
            (Role::Assistant, antigravity::UNAVAILABLE),
        ]
    );
    assert_eq!(d.messages[0].created_at, 1_772_359_200_000);
    // prompts after the previous session belong to the next one in the same workspace
    let d2 = antigravity::read(agy_dir().path(), A2).unwrap();
    assert_eq!(d2.messages[0].content, "later session prompt");
    assert_eq!(d2.messages.len(), 2);
    let d3 = antigravity::read(agy_dir().path(), A3).unwrap();
    assert_eq!(d3.messages.len(), 1);
    assert_eq!(d3.session.messages, 1);
}

#[test]
fn agy_missing_dir_and_unknown_id() {
    assert!(antigravity::list(Path::new("/definitely/not/here")).is_empty());
    assert!(antigravity::read(agy_dir().path(), "zzz").is_none());
}

#[test]
fn claude_list_sorted_newest_first_by_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("-p");
    std::fs::create_dir_all(&p).unwrap();
    let line = r#"{"type":"user","cwd":"/x","message":{"role":"user","content":"hi"}}"#;
    for (name, age) in [("old", 100), ("new", 10)] {
        let f = p.join(format!("{name}.jsonl"));
        std::fs::write(&f, line).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age);
        std::fs::File::options()
            .write(true)
            .open(&f)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    let ids: Vec<_> = claude::list(dir.path())
        .into_iter()
        .map(|s| s.external_id)
        .collect();
    assert_eq!(ids, ["new", "old"]);
}
