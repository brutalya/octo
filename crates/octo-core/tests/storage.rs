use octo_core::provider::ProviderKind;
use octo_core::storage::{Role, Space, Storage};

fn db() -> Storage {
    Storage::open_in_memory().unwrap()
}

#[test]
fn migrations_set_version_and_are_idempotent() {
    let dir = std::env::temp_dir().join(format!("octo-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("octo.db");
    let s = Storage::open(&path).unwrap();
    s.create_thread(ProviderKind::Claude, Space::Public)
        .unwrap();
    drop(s);
    let s = Storage::open(&path).unwrap();
    assert_eq!(s.user_version().unwrap(), 1);
    assert_eq!(s.list_threads().unwrap().len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn thread_crud_and_ordering() {
    let s = db();
    let a = s
        .create_thread(ProviderKind::Claude, Space::Public)
        .unwrap();
    let b = s
        .create_thread(ProviderKind::Antigravity, Space::Private)
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
        .create_thread(ProviderKind::Claude, Space::Public)
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
        .create_thread(ProviderKind::Claude, Space::Public)
        .unwrap();
    assert_eq!(t.title, "");
    s.set_title_if_empty(&t.id, &"x".repeat(100)).unwrap();
    s.set_title_if_empty(&t.id, "other").unwrap();
    s.set_external_session(&t.id, "sid").unwrap();
    let t = s.get_thread(&t.id).unwrap().unwrap();
    assert_eq!(t.title.chars().count(), 60);
    assert_eq!(t.external_session_id.as_deref(), Some("sid"));
}
