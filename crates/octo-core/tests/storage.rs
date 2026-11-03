use octo_core::provider::ProviderKind;
use octo_core::storage::{Approval, Role, Space, Storage};

fn db() -> Storage {
    Storage::open_in_memory().unwrap()
}

#[test]
fn migrations_set_version_and_are_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("octo.db");
    let s = Storage::open(&path).unwrap();
    s.create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
        .unwrap();
    drop(s);
    let s = Storage::open(&path).unwrap();
    assert_eq!(s.user_version().unwrap(), 2);
    assert_eq!(s.list_threads().unwrap().len(), 1);
}

#[test]
fn thread_crud_and_ordering() {
    let s = db();
    let a = s
        .create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
        .unwrap();
    let b = s
        .create_thread(ProviderKind::Antigravity, Space::Private, Approval::Confirm)
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
        .create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
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
        .create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
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
            .create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
            .unwrap();
        s.create_thread(ProviderKind::Claude, Space::Public, Approval::Confirm)
            .unwrap();
        s.add_message(&a.id, Role::User, "hi").unwrap();
        assert_eq!(s.list_threads().unwrap()[0].id, a.id);
    }
}

#[test]
fn approval_roundtrip_and_check_constraint() {
    let s = db();
    let t = s
        .create_thread(ProviderKind::Claude, Space::Public, Approval::Auto)
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
    assert_eq!(s.user_version().unwrap(), 2);
    assert_eq!(
        s.get_thread("x").unwrap().unwrap().approval,
        Approval::Confirm
    );
}
