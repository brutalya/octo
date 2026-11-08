use octo_core::providers::ProviderKind;
use octo_core::storage::{Approval, NewImport, Origin, Role, SourceMessage, Space, Storage};

fn db() -> Storage {
    Storage::open_in_memory().unwrap()
}

#[test]
fn migrations_set_version_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("octo.db");
    let s = Storage::open(&path).unwrap();
    s.create_thread_in(
        ProviderKind::Claude,
        Space::Public,
        Approval::Confirm,
        None,
        None,
    )
    .unwrap();
    drop(s);
    let s = Storage::open(&path).unwrap();
    assert_eq!(s.user_version().unwrap(), 4);
    assert_eq!(s.list_threads().unwrap().len(), 1);
}

#[test]
fn thread_crud_and_ordering() {
    let s = db();
    let a = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    let b = s
        .create_thread_in(
            ProviderKind::Antigravity,
            Space::Private,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    assert_eq!(b.space, Space::Private);
    assert_eq!(s.list_threads().unwrap()[0].id, b.id);
    s.add_message(&a.id, Role::User, "hi").unwrap();
    assert_eq!(
        s.list_threads().unwrap()[0].id,
        a.id,
        "updated thread moves to top"
    );
    s.delete_thread(&b.id).unwrap();
    assert_eq!(s.list_threads().unwrap().len(), 1);
    assert!(s.get_thread(&b.id).unwrap().is_none());
}

#[test]
fn messages_ordered_and_cascade() {
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    s.add_message(&t.id, Role::User, "1").unwrap();
    s.add_message(&t.id, Role::Assistant, "2").unwrap();
    s.add_message(&t.id, Role::Error, "3").unwrap();
    let m: Vec<_> = s
        .list_messages(&t.id)
        .unwrap()
        .into_iter()
        .map(|m| m.content)
        .collect();
    assert_eq!(m, ["1", "2", "3"]);
    s.delete_thread(&t.id).unwrap();
    assert!(s.list_messages(&t.id).unwrap().is_empty());
}

#[test]
fn title_and_session() {
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    assert_eq!(t.title, "");
    s.set_title_if_empty(&t.id, &"x".repeat(100)).unwrap();
    s.set_title_if_empty(&t.id, "other").unwrap();
    s.set_external_session(&t.id, "sid").unwrap();
    let t = s.get_thread(&t.id).unwrap().unwrap();
    assert_eq!(t.title.chars().count(), 60);
    assert_eq!(t.external_session_id.as_deref(), Some("sid"));
}

#[test]
fn touched_thread_always_moves_to_top() {
    for _ in 0..200 {
        let s = db();
        let a = s
            .create_thread_in(
                ProviderKind::Claude,
                Space::Public,
                Approval::Confirm,
                None,
                None,
            )
            .unwrap();
        s.create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
        s.add_message(&a.id, Role::User, "hi").unwrap();
        assert_eq!(s.list_threads().unwrap()[0].id, a.id);
    }
}

#[test]
fn approval_roundtrip_and_check_constraint() {
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Auto,
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        s.get_thread(&t.id).unwrap().unwrap().approval,
        Approval::Auto
    );
    assert!(s.set_approval(&t.id, Approval::Confirm).unwrap());
    assert_eq!(
        s.get_thread(&t.id).unwrap().unwrap().approval,
        Approval::Confirm
    );
    assert!(!s.set_approval("missing", Approval::Auto).unwrap());
}

#[test]
fn migration_2_keeps_old_threads_as_confirm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE threads(
        id TEXT PRIMARY KEY, title TEXT NOT NULL, provider TEXT NOT NULL,
        space TEXT NOT NULL CHECK(space IN('private','public')),
        external_session_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
     CREATE TABLE messages(
        id TEXT PRIMARY KEY,
        thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
        role TEXT NOT NULL CHECK(role IN('user','assistant','error')),
        content TEXT NOT NULL, created_at INTEGER NOT NULL);
     CREATE INDEX messages_thread ON messages(thread_id);
     INSERT INTO threads VALUES('x','t','claude','public',NULL,1,1);
     PRAGMA user_version=1;",
        )
        .unwrap();
    }
    let s = Storage::open(&path).unwrap();
    assert_eq!(s.user_version().unwrap(), 4);
    assert_eq!(
        s.get_thread("x").unwrap().unwrap().approval,
        Approval::Confirm
    );
}

#[test]
fn thread_model_roundtrip() {
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("llama3:8b"),
            None,
        )
        .unwrap();
    assert_eq!(
        s.get_thread(&t.id).unwrap().unwrap().model.as_deref(),
        Some("llama3:8b")
    );
    let c = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    assert_eq!(s.get_thread(&c.id).unwrap().unwrap().model, None);
}

#[test]
fn message_attachments_roundtrip() {
    use octo_core::storage::AttachmentRef;
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Ollama,
            Space::Private,
            Approval::Confirm,
            Some("m"),
            None,
        )
        .unwrap();
    let refs = vec![AttachmentRef {
        space: Space::Private,
        path: "a/b.txt".into(),
    }];
    s.add_message_with(&t.id, Role::User, "see", &refs).unwrap();
    s.add_message(&t.id, Role::Assistant, "ok").unwrap();
    let m = s.list_messages(&t.id).unwrap();
    assert_eq!(m[0].attachments, refs);
    assert!(m[1].attachments.is_empty());
}

#[test]
fn migrates_a_version_2_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("octo.db");
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.execute_batch(
            "CREATE TABLE threads(id TEXT PRIMARY KEY, title TEXT NOT NULL, provider TEXT NOT NULL,
               space TEXT NOT NULL, external_session_id TEXT, created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL, approval TEXT NOT NULL DEFAULT 'confirm');
             CREATE TABLE messages(id TEXT PRIMARY KEY, thread_id TEXT NOT NULL, role TEXT NOT NULL,
               content TEXT NOT NULL, created_at INTEGER NOT NULL);
             INSERT INTO threads VALUES('t','x','claude','public',NULL,1,1,'confirm');
             INSERT INTO messages VALUES('m','t','user','hi',1);
             PRAGMA user_version=2;",
        )
        .unwrap();
    }
    let s = Storage::open(&path).unwrap();
    assert_eq!(s.user_version().unwrap(), 4);
    assert_eq!(s.get_thread("t").unwrap().unwrap().model, None);
    assert!(s.list_messages("t").unwrap()[0].attachments.is_empty());
}

#[test]
fn threads_have_cwd_and_origin() {
    let s = Storage::open_in_memory().unwrap();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    assert_eq!(t.cwd, None);
    assert_eq!(t.origin, Origin::Octo);
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            Some("/srv/x"),
        )
        .unwrap();
    assert_eq!(
        s.get_thread(&t.id).unwrap().unwrap().cwd.as_deref(),
        Some("/srv/x")
    );
}

fn imported(s: &Storage) -> octo_core::storage::Thread {
    s.import_thread(&NewImport {
        provider: ProviderKind::Claude,
        external_id: "ext-1",
        title: "T",
        cwd: Some("/srv/x"),
        updated_at: 5_000,
        stamp: "s1",
        messages: &[
            SourceMessage {
                role: Role::User,
                content: "a".into(),
                created_at: 1_000,
            },
            SourceMessage {
                role: Role::Assistant,
                content: "b".into(),
                created_at: 2_000,
            },
        ],
    })
    .unwrap()
}

#[test]
fn import_creates_public_imported_thread_with_messages() {
    let s = Storage::open_in_memory().unwrap();
    let t = imported(&s);
    assert_eq!(t.origin, Origin::Imported);
    assert_eq!(t.space, Space::Public);
    assert_eq!(t.updated_at, 5_000);
    assert_eq!(t.external_session_id.as_deref(), Some("ext-1"));
    let found = s
        .find_by_external(ProviderKind::Claude, "ext-1")
        .unwrap()
        .unwrap();
    assert_eq!(found.id, t.id);
    assert!(
        s.find_by_external(ProviderKind::Antigravity, "ext-1")
            .unwrap()
            .is_none()
    );
    let m = s.list_messages(&t.id).unwrap();
    assert_eq!(
        m.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(s.sync_stamp(&t.id).unwrap().as_deref(), Some("s1"));
}

#[test]
fn replace_messages_swaps_cache_and_stamp() {
    let s = Storage::open_in_memory().unwrap();
    let t = imported(&s);
    s.replace_messages(
        &t.id,
        &[SourceMessage {
            role: Role::User,
            content: "c".into(),
            created_at: 3_000,
        }],
        "s2",
        9_000,
    )
    .unwrap();
    let m = s.list_messages(&t.id).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].content, "c");
    assert_eq!(s.sync_stamp(&t.id).unwrap().as_deref(), Some("s2"));
    assert_eq!(s.get_thread(&t.id).unwrap().unwrap().updated_at, 9_000);
}

#[test]
fn newer_schema_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("octo.db");
    drop(Storage::open(&path).unwrap());
    let c = rusqlite::Connection::open(&path).unwrap();
    c.pragma_update(None, "user_version", 99).unwrap();
    drop(c);
    assert!(Storage::open(&path).is_err());
}

#[test]
fn corrupt_rows_are_errors_not_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("octo.db");
    let s = Storage::open(&path).unwrap();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    s.add_message(&t.id, Role::User, "x").unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    c.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    for (sql, what) in [
        ("UPDATE threads SET space='bogus'", "space"),
        ("UPDATE threads SET provider='bogus'", "provider"),
        ("UPDATE threads SET approval='bogus'", "approval"),
        ("UPDATE threads SET origin='bogus'", "origin"),
    ] {
        let fresh = tempfile::tempdir().unwrap();
        let p2 = fresh.path().join("o.db");
        std::fs::copy(&path, &p2).unwrap();
        rusqlite::Connection::open(&p2)
            .unwrap()
            .pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        let c2 = rusqlite::Connection::open(&p2).unwrap();
        c2.pragma_update(None, "ignore_check_constraints", true)
            .unwrap();
        c2.execute_batch(sql).unwrap();
        drop(c2);
        let s2 = Storage::open(&p2).unwrap();
        assert!(s2.list_threads().is_err(), "{what}");
    }
    c.pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    c.execute_batch("UPDATE messages SET role='bogus'").unwrap();
    drop(c);
    let s = Storage::open(&path).unwrap();
    assert!(s.list_messages(&t.id).is_err());
}

#[test]
fn attachments_roundtrip_in_one_transaction() {
    use octo_core::storage::AttachmentRef;
    let s = db();
    let t = s
        .create_thread_in(
            ProviderKind::Claude,
            Space::Public,
            Approval::Confirm,
            None,
            None,
        )
        .unwrap();
    let a = AttachmentRef {
        space: Space::Public,
        path: "a.txt".into(),
    };
    s.add_message_with(&t.id, Role::User, "hi", std::slice::from_ref(&a))
        .unwrap();
    assert_eq!(s.list_messages(&t.id).unwrap()[0].attachments, vec![a]);
}
