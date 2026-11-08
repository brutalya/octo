use octo_core::storage::Space;
use octo_net::membership::Trust;
use octo_server::policy::{
    TunnelReq, body_touches_private, filter_thread_list, thread_id, tunnel_allows,
};

fn req<'a>(method: &'a str, path: &'a str) -> TunnelReq<'a> {
    TunnelReq {
        method,
        path,
        thread_space: None,
        body_private: false,
    }
}

#[test]
fn full_trust_allows_everything_here() {
    let mut r = req("GET", "/api/files/private");
    assert!(tunnel_allows(Trust::Full, &r));
    r.thread_space = Some(Space::Private);
    r.body_private = true;
    assert!(tunnel_allows(Trust::Full, &r));
}

#[test]
fn public_trust_matrix() {
    let p = Trust::Public;
    // Public data and non-space routes are fine.
    for (m, path) in [
        ("GET", "/api/threads"),
        ("POST", "/api/threads"),
        ("GET", "/api/providers"),
        ("GET", "/api/providers/ollama/models"),
        ("GET", "/api/files/public"),
        ("POST", "/api/files/public"),
        ("GET", "/api/files/public/content"),
        ("GET", "/api/import/claude/sessions"),
        ("POST", "/api/import/claude"),
    ] {
        assert!(tunnel_allows(p, &req(m, path)), "{m} {path}");
    }
    // Private files, every verb and sub-route.
    for (m, path) in [
        ("GET", "/api/files/private"),
        ("POST", "/api/files/private"),
        ("PATCH", "/api/files/private"),
        ("DELETE", "/api/files/private"),
        ("GET", "/api/files/private/content"),
        ("POST", "/api/files/private/dir"),
    ] {
        assert!(!tunnel_allows(p, &req(m, path)), "{m} {path}");
    }
    // Private threads: every route addressing one.
    for (m, path) in [
        ("GET", "/api/threads/t1/messages"),
        ("POST", "/api/threads/t1/messages"),
        ("POST", "/api/threads/t1/cancel"),
        ("PATCH", "/api/threads/t1"),
        ("DELETE", "/api/threads/t1"),
    ] {
        let mut r = req(m, path);
        r.thread_space = Some(Space::Private);
        assert!(!tunnel_allows(p, &r), "{m} {path}");
        r.thread_space = Some(Space::Public);
        assert!(tunnel_allows(p, &r), "{m} {path} public");
        r.thread_space = None; // unknown thread: the router answers 404
        assert!(tunnel_allows(p, &r), "{m} {path} unknown");
    }
    // Bodies naming the private space: thread creation, import, attachments.
    for path in [
        "/api/threads",
        "/api/import/claude",
        "/api/threads/t1/messages",
    ] {
        let mut r = req("POST", path);
        r.body_private = true;
        assert!(!tunnel_allows(p, &r), "{path}");
    }
}

#[test]
fn body_inspection() {
    assert!(body_touches_private(
        br#"{"provider":"ollama","space":"private"}"#
    ));
    assert!(!body_touches_private(
        br#"{"provider":"ollama","space":"public"}"#
    ));
    assert!(body_touches_private(
        br#"{"content":"x","attachments":[{"space":"public","path":"a"},{"space":"private","path":"b"}]}"#
    ));
    assert!(!body_touches_private(
        br#"{"content":"x","attachments":[]}"#
    ));
    assert!(!body_touches_private(b""));
    // Garbage that contains the word is not JSON the router would accept anyway.
    assert!(!body_touches_private(b"not json private"));
}

#[test]
fn thread_id_from_path() {
    assert_eq!(thread_id("/api/threads/abc/messages"), Some("abc"));
    assert_eq!(thread_id("/api/threads/abc"), Some("abc"));
    assert_eq!(thread_id("/api/threads"), None);
    assert_eq!(thread_id("/api/threads?x=1"), None);
    assert_eq!(thread_id("/api/files/public"), None);
}

#[test]
fn thread_list_is_filtered() {
    let body = br#"[{"id":"a","space":"public","x":1},{"id":"b","space":"private"},{"id":"c","space":"public"}]"#;
    let out = filter_thread_list(body).unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let ids: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["a", "c"]);
    assert_eq!(v[0]["x"], 1);
    // Anything that is not a thread array is refused rather than passed through.
    assert!(filter_thread_list(b"{}").is_none());
    assert!(filter_thread_list(b"xx").is_none());
    assert!(filter_thread_list(br#"[{"id":"a"}]"#).is_none());
}

#[test]
fn sync_routes_denied_for_public_allowed_for_full() {
    for path in [
        "/api/sync/private/manifest",
        "/api/sync/private/blob/00",
        "/api/sync/private/blob/abc?x=1",
    ] {
        assert!(!tunnel_allows(Trust::Public, &req("GET", path)), "{path}");
        assert!(tunnel_allows(Trust::Full, &req("GET", path)), "{path}");
    }
}
