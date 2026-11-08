use octo_core::providers::ProviderKind;
use octo_core::storage::Space;
use octo_net::membership::Trust;
use octo_server::policy::{
    TunnelReq, body_agent_fields, body_touches_private, filter_thread_list, path_allowed,
    thread_id, tunnel_allows as allows,
};

/// Trust/agents matrix helper: existing tests grant agents so they keep testing trust only.
fn tunnel_allows(t: Trust, r: &TunnelReq) -> bool {
    allows(t, true, r)
}

fn req<'a>(method: &'a str, path: &'a str) -> TunnelReq<'a> {
    TunnelReq {
        method,
        path,
        thread_space: None,
        thread_provider: None,
        body_private: false,
        body_agents: false,
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

#[test]
fn agents_off_denies_cli_use_but_allows_http_providers() {
    for trust in [Trust::Full, Trust::Public] {
        let a = |r: &TunnelReq| allows(trust, false, r);
        // Import, list and run.
        assert!(!a(&req("GET", "/api/import/claude/sessions")));
        assert!(!a(&req("POST", "/api/import/antigravity")));
        // CLI thread: send and patch denied; read, cancel, delete allowed.
        for (m, p, ok) in [
            ("POST", "/api/threads/t1/messages", false),
            ("PATCH", "/api/threads/t1", false),
            ("GET", "/api/threads/t1/messages", true),
            ("POST", "/api/threads/t1/cancel", true),
            ("DELETE", "/api/threads/t1", true),
        ] {
            let mut r = req(m, p);
            r.thread_space = Some(Space::Public);
            r.thread_provider = Some(ProviderKind::Claude);
            assert_eq!(a(&r), ok, "{m} {p}");
            r.thread_provider = Some(ProviderKind::Antigravity);
            assert_eq!(a(&r), ok, "{m} {p} agy");
        }
        // HTTP provider threads stay usable (subject to the trust rules above).
        let mut r = req("POST", "/api/threads/t1/messages");
        r.thread_space = Some(Space::Public);
        r.thread_provider = Some(ProviderKind::Ollama);
        assert!(a(&r));
        // Creating a CLI thread, or setting approval / cwd, is denied.
        let mut r = req("POST", "/api/threads");
        r.body_agents = true;
        assert!(!a(&r));
        assert!(!allows(trust, false, &{
            let mut r = req("PATCH", "/api/threads/t1");
            r.body_agents = true;
            r
        }));
        assert!(
            a(&req("POST", "/api/threads")),
            "plain http thread creation"
        );
        // With agents on, all of it is allowed again.
        let mut r = req("POST", "/api/threads");
        r.body_agents = true;
        assert!(allows(trust, true, &r));
    }
}

#[test]
fn body_agent_fields_detection() {
    for yes in [
        r#"{"provider":"claude","space":"public"}"#,
        r#"{"provider":"antigravity","space":"public"}"#,
        r#"{"provider":"ollama","space":"public","approval":"confirm"}"#,
        r#"{"approval":"auto"}"#,
        r#"{"provider":"ollama","space":"public","cwd":"/tmp"}"#,
    ] {
        assert!(body_agent_fields(yes.as_bytes()), "{yes}");
    }
    for no in [
        r#"{"provider":"ollama","space":"public","model":"m"}"#,
        r#"{"provider":"openrouter","space":"public","model":"m","cwd":null}"#,
        r#"{"content":"hi"}"#,
        "",
        "not json",
    ] {
        assert!(!body_agent_fields(no.as_bytes()), "{no}");
    }
}

#[test]
fn path_allow_list_pins_percent_coupling() {
    for ok in [
        "/api/threads",
        "/api/threads/t/messages?x=1",
        "/api/files/public/content?path=a%20b",
        "/api/sync/private/manifest",
    ] {
        assert!(path_allowed(ok), "{ok}");
    }
    for no in [
        "/api/files/%70rivate",
        "/api/files/%70rivate/content",
        "/api/settings",
        "/api/nodes/x",
        "/api/pairing/invite",
        "/api/threads/../settings",
        "/api/threads/./x",
        "/api",
        "threads",
    ] {
        assert!(!path_allowed(no), "{no}");
    }
}
