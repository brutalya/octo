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

fn varint(mut v: u64, out: &mut Vec<u8>) {
    while v >= 0x80 {
        out.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn field_varint(n: u64, v: u64, out: &mut Vec<u8>) {
    varint(n << 3, out);
    varint(v, out);
}

fn field_bytes(n: u64, v: &[u8], out: &mut Vec<u8>) {
    varint(n << 3 | 2, out);
    varint(v.len() as u64, out);
    out.extend_from_slice(v);
}

/// Step payload in the layout `agy` writes: `1` type, `5` metadata (`5.1` timestamp
/// seconds+nanos, noise fields around it), plus the type specific message.
fn step(kind: u64, secs: u64, body: Option<(u64, Vec<u8>)>) -> Vec<u8> {
    let mut ts = Vec::new();
    field_varint(1, secs, &mut ts);
    field_varint(2, 123_456_789, &mut ts);
    let mut meta = Vec::new();
    field_bytes(1, &ts, &mut meta);
    field_varint(3, 4, &mut meta);
    field_bytes(12, b"00000000-0000-4000-8000-000000000000", &mut meta);
    let mut out = Vec::new();
    field_varint(1, kind, &mut out);
    field_varint(4, 3, &mut out);
    field_bytes(5, &meta, &mut out);
    if let Some((n, b)) = body {
        field_bytes(n, &b, &mut out);
    }
    out
}

fn user_step(secs: u64, text: &str) -> Vec<u8> {
    let mut b = Vec::new();
    field_bytes(2, text.as_bytes(), &mut b);
    field_bytes(3, text.as_bytes(), &mut b);
    step(14, secs, Some((19, b)))
}

fn reply_step(secs: u64, text: &str) -> Vec<u8> {
    let mut b = Vec::new();
    field_bytes(1, text.as_bytes(), &mut b);
    field_varint(12, 2, &mut b);
    step(15, secs, Some((20, b)))
}

fn tool_step(secs: u64) -> Vec<u8> {
    step(101, secs, Some((114, b"\x0a\x03abc".to_vec())))
}

fn write_steps(dir: &Path, id: &str, payloads: &[Vec<u8>]) {
    std::fs::create_dir_all(dir.join("conversations")).unwrap();
    let c = rusqlite::Connection::open(dir.join("conversations").join(format!("{id}.db"))).unwrap();
    c.execute_batch(
        "CREATE TABLE `steps` (`idx` integer,`step_type` integer NOT NULL DEFAULT 0,\
         `status` integer NOT NULL DEFAULT 0,`step_payload` blob,PRIMARY KEY (`idx`));",
    )
    .unwrap();
    for (i, p) in payloads.iter().enumerate() {
        c.execute(
            "INSERT INTO steps (idx, step_type, step_payload) VALUES (?1, 0, ?2)",
            rusqlite::params![i as i64, p],
        )
        .unwrap();
    }
}

#[test]
fn agy_messages_are_decoded_from_conversation_steps() {
    let dir = agy_dir();
    write_steps(
        dir.path(),
        A1,
        &[
            user_step(1_772_359_200, "first question"),
            reply_step(1_772_359_210, "first answer"),
            tool_step(1_772_359_215),
            user_step(1_772_359_300, "second question"),
            reply_step(1_772_359_310, "let me look"),
            tool_step(1_772_359_320),
            reply_step(1_772_359_330, "second answer"),
        ],
    );
    let d = antigravity::read(dir.path(), A1).unwrap();
    let got: Vec<_> = d
        .messages
        .iter()
        .map(|m| (m.role, m.content.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            (Role::User, "first question"),
            (Role::Assistant, "first answer"),
            (Role::User, "second question"),
            (Role::Assistant, "let me look\n\nsecond answer"),
        ]
    );
    assert_eq!(d.messages[0].created_at, 1_772_359_200_123);
    assert_eq!(d.session.messages, 4);
}

#[test]
fn agy_undecodable_steps_fall_back_to_history() {
    let dir = agy_dir();
    // truncated varint, and a payload with no readable messages
    write_steps(dir.path(), A1, &[vec![0x0a, 0xff, 0xff], vec![0x08, 0x0e]]);
    let d = antigravity::read(dir.path(), A1).unwrap();
    assert_eq!(d.messages.len(), 3);
    assert_eq!(d.messages[2].content, antigravity::UNAVAILABLE);
    // a conversation db that is not sqlite
    std::fs::write(
        dir.path().join("conversations").join(format!("{A2}.db")),
        b"junk",
    )
    .unwrap();
    let d2 = antigravity::read(dir.path(), A2).unwrap();
    assert_eq!(
        d2.messages.last().unwrap().content,
        antigravity::UNAVAILABLE
    );
}

#[test]
fn agy_decoding_never_modifies_cli_storage() {
    let dir = agy_dir();
    write_steps(
        dir.path(),
        A1,
        &[
            user_step(1_772_359_200, "q"),
            reply_step(1_772_359_210, "a"),
        ],
    );
    let list = |d: &Path| {
        let mut v: Vec<_> = std::fs::read_dir(d.join("conversations"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        v.sort();
        v
    };
    let db = dir.path().join("conversations").join(format!("{A1}.db"));
    let before = (std::fs::read(&db).unwrap(), list(dir.path()));
    antigravity::read(dir.path(), A1).unwrap();
    assert_eq!((std::fs::read(&db).unwrap(), list(dir.path())), before);
}
