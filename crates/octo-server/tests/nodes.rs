#![cfg(unix)] // uses tests/fixtures/fake_cli.sh
use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use octo_core::{Config, Hub};
use octo_net::relay::{Relay, router};
use octo_server::node::{Node, NodeOpts};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

struct N {
    app: Router,
    _node: Arc<Node>,
    dir: tempfile::TempDir,
    token: Option<String>,
}

async fn relay() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let app = router(Relay::new(Default::default()).unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(l, app).await;
    });
    format!("ws://{addr}/relay")
}

fn fake() -> PathBuf {
    PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_cli.sh",
        env!("CARGO_MANIFEST_DIR")
    ))
}

async fn start(relay_url: &str, name: &str, token: Option<&str>, seed: Option<&str>) -> N {
    let dir = tempfile::tempdir().unwrap();
    if let Some(m) = seed {
        std::fs::write(dir.path().join("membership.json"), m).unwrap();
    }
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake(),
        agy_bin: fake(),
        openrouter_base: String::new(),
        ..Config::new(None)
    };
    let hub = Arc::new(Hub::new(cfg, "test-node").unwrap());
    let node = Node::start(
        hub.clone(),
        dir.path(),
        NodeOpts {
            connect_timeout: Duration::from_millis(1500),
        },
    )
    .unwrap();
    let app = octo_server::app_with_node(hub, node.clone(), None, token.map(str::to_string));
    let n = N {
        app,
        _node: node,
        dir,
        token: token.map(str::to_string),
    };
    let r = n
        .call(
            "PATCH",
            "/api/settings",
            Some(json!({"relay_url": relay_url, "node_name": name})),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    n
}

impl N {
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> Response<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = &self.token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let body = match body {
            Some(v) => {
                b = b.header("content-type", "application/json");
                Body::from(v.to_string())
            }
            None => Body::empty(),
        };
        self.app
            .clone()
            .oneshot(b.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn json(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        let r = self.call(method, uri, body).await;
        let s = r.status();
        let b = r.into_body().collect().await.unwrap().to_bytes();
        (s, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn id(&self) -> String {
        let (_, v) = self.json("GET", "/api/nodes", None).await;
        v.as_array()
            .unwrap()
            .iter()
            .find(|n| n["is_self"] == true)
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn pending(&self) -> Value {
        for _ in 0..100 {
            let (s, v) = self.json("GET", "/api/pairing/pending", None).await;
            if s == StatusCode::OK {
                return v;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no pending pairing");
    }
}

/// Runs the pairing a -> b through the API; `trust` is the host's choice (None = no body).
async fn pair_nodes(a: &N, b: &N, trust: Option<bool>) {
    pair_nodes_agents(a, b, trust, None, None).await;
}

/// Like [`pair_nodes`]; `host_agents` / `join_agents` are the `allow_agents` each side sends.
async fn pair_nodes_agents(
    a: &N,
    b: &N,
    trust: Option<bool>,
    host_agents: Option<bool>,
    join_agents: Option<bool>,
) {
    let (s, inv) = a.json("POST", "/api/pairing/invite", None).await;
    assert_eq!(s, StatusCode::OK, "{inv}");
    assert!(inv["expires_at"].as_u64().unwrap() > 0);
    let code = inv["code"].as_str().unwrap().to_string();
    let (s, _) = b
        .json(
            "POST",
            "/api/pairing/join",
            Some(json!({"code": code, "name": "beta"})),
        )
        .await;
    assert!(s.is_success());
    let (pa, pb) = (a.pending().await, b.pending().await);
    assert_eq!(pa["sas"], pb["sas"]);
    assert_eq!(pa["sas"].as_str().unwrap().len(), 6);
    assert_eq!(pa["peer_name"], "beta");
    // Busy while a pairing is in flight.
    let (s, _) = a.json("POST", "/api/pairing/invite", None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    let mut body = serde_json::Map::new();
    if let Some(t) = trust {
        body.insert("trust_private".into(), json!(t));
    }
    if let Some(g) = host_agents {
        body.insert("allow_agents".into(), json!(g));
    }
    let body = (!body.is_empty()).then_some(Value::Object(body));
    assert!(
        a.call("POST", "/api/pairing/confirm", body)
            .await
            .status()
            .is_success()
    );
    let body = join_agents.map(|g| json!({"allow_agents": g}));
    assert!(
        b.call("POST", "/api/pairing/confirm", body)
            .await
            .status()
            .is_success()
    );
}

async fn wait_online(a: &N, id: &str) {
    for _ in 0..100 {
        let (_, v) = a.json("GET", "/api/nodes", None).await;
        let v = v.as_array().unwrap().clone();
        if v.iter().any(|n| n["node_id"] == id && n["online"] == true) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("peer never online");
}

/// Pairs b into a's network through the API. Returns (a, b).
async fn paired(tok_a: Option<&str>, tok_b: Option<&str>) -> (N, N, String) {
    paired_trust(tok_a, tok_b, None).await
}

async fn paired_trust(
    tok_a: Option<&str>,
    tok_b: Option<&str>,
    trust: Option<bool>,
) -> (N, N, String) {
    let url = relay().await;
    let a = start(&url, "alpha", tok_a, None).await;
    let b = start(&url, "beta", tok_b, None).await;
    pair_nodes(&a, &b, trust).await;
    let id_b = b.id().await;
    wait_online(&a, &id_b).await;
    (a, b, id_b)
}

#[tokio::test]
async fn pairing_via_api_and_nodes_list() {
    let (a, b, id_b) = paired(None, None).await;
    let id_a = a.id().await;
    let (_, v) = b.json("GET", "/api/nodes", None).await;
    let v = v.as_array().unwrap();
    assert_eq!(v.len(), 2);
    let peer = v.iter().find(|n| n["node_id"] == id_a.as_str()).unwrap();
    assert_eq!(peer["name"], "alpha");
    assert_eq!(peer["is_self"], false);
    let (_, v) = a.json("GET", "/api/nodes", None).await;
    let peer = v
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == id_b.as_str())
        .unwrap()
        .clone();
    assert_eq!(peer["name"], "beta");
    // No pending pairing any more.
    let (s, _) = a.json("GET", "/api/pairing/pending", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    for f in ["membership.json", "keys/node.key"] {
        assert!(a.dir.path().join(f).exists(), "{f}");
    }
}

#[tokio::test]
async fn reject_ends_pairing() {
    let url = relay().await;
    let a = start(&url, "alpha", None, None).await;
    let b = start(&url, "beta", None, None).await;
    let (_, inv) = a.json("POST", "/api/pairing/invite", None).await;
    b.json(
        "POST",
        "/api/pairing/join",
        Some(json!({"code": inv["code"], "name": "beta"})),
    )
    .await;
    a.pending().await;
    assert!(
        a.call("POST", "/api/pairing/reject", None)
            .await
            .status()
            .is_success()
    );
    let (s, _) = a.json("GET", "/api/pairing/pending", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, v) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn tunneled_get_threads_without_forwarding_token() {
    let (a, b, id_b) = paired(Some("ta"), Some("tb")).await;
    allow_agents(&b, &a).await;
    // b's own token is required locally...
    let r = Request::builder()
        .uri("/api/threads")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        b.app.clone().oneshot(r).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let (s, t) = b
        .json(
            "POST",
            "/api/threads",
            Some(json!({"provider": "claude", "space": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    // ...but not through the tunnel (tunnel identity is the auth); a's token is not forwarded.
    let (s, v) = a
        .json("GET", &format!("/api/nodes/{id_b}/threads"), None)
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["id"], t["id"]);
    // a's own list is empty.
    let (_, mine) = a.json("GET", "/api/threads", None).await;
    assert_eq!(mine.as_array().unwrap().len(), 0);
    // Query string and POST body survive.
    let (s, q) = a
        .json(
            "GET",
            &format!("/api/nodes/{id_b}/import/claude/sessions?q=zzz"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{q}");
    let (s, _) = a
        .json(
            "POST",
            &format!("/api/nodes/{id_b}/threads"),
            Some(json!({"provider": "claude", "space": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, v) = b.json("GET", "/api/threads", None).await;
    assert_eq!(v.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn tunneled_sse_streams_incrementally() {
    let (a, b, id_b) = paired(None, None).await;
    allow_agents(&b, &a).await;
    let (_, t) = b
        .json(
            "POST",
            "/api/threads",
            Some(json!({"provider": "claude", "space": "public"})),
        )
        .await;
    let tid = t["id"].as_str().unwrap();
    let mut r = a
        .call(
            "POST",
            &format!("/api/nodes/{id_b}/threads/{tid}/messages"),
            Some(json!({"content": "SLOW"})),
        )
        .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["content-type"], "text/event-stream");
    // The fake CLI emits one line and then hangs: this frame must arrive before the run ends.
    let frame = tokio::time::timeout(Duration::from_secs(5), r.body_mut().frame())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let mut body = String::from_utf8_lossy(frame.data_ref().unwrap()).to_string();
    assert!(body.contains("event:"), "{body}");
    // Cancel through the tunnel; the stream then finishes.
    let (s, _) = a
        .json(
            "POST",
            &format!("/api/nodes/{id_b}/threads/{tid}/cancel"),
            None,
        )
        .await;
    assert!(s.is_success());
    body += &String::from_utf8_lossy(
        &tokio::time::timeout(Duration::from_secs(10), r.into_body().collect())
            .await
            .unwrap()
            .unwrap()
            .to_bytes(),
    );
    assert!(body.contains("event: session"), "{body}");
}

#[tokio::test]
async fn non_member_and_unknown_nodes_rejected() {
    let (a, b, _id_b) = paired(None, None).await;
    let id_a = a.id().await;
    // Unknown node id.
    let (s, _) = a
        .json("GET", "/api/nodes/aaaaaaaaaaaaaaaaaaaaaaaaaa/threads", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = a.json("GET", "/api/nodes/!!/threads", None).await;
    assert!(s.is_client_error());
    // c knows a (copy of b's membership: a's pubkey + network key) but a does not know c.
    let seed = std::fs::read_to_string(b.dir.path().join("membership.json")).unwrap();
    let url = relay_of(&a).await;
    let c = start(&url, "gamma", None, Some(&seed)).await;
    let (s, v) = c
        .json("GET", &format!("/api/nodes/{id_a}/threads"), None)
        .await;
    assert!(s.is_server_error(), "{s} {v}");
    let (_, list) = c.json("GET", "/api/nodes", None).await;
    let peer = list
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == id_a.as_str())
        .unwrap()
        .clone();
    assert_eq!(peer["online"], false);
}

async fn relay_of(n: &N) -> String {
    let (_, s) = n.json("GET", "/api/settings", None).await;
    s["relay_url"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn re_forwarding_is_rejected() {
    let (a, b, id_b) = paired(None, None).await;
    let id_a = a.id().await;
    // b receives /api/nodes/<a>/threads through the tunnel: it must not forward again.
    let (s, v) = a
        .json(
            "GET",
            &format!("/api/nodes/{id_b}/nodes/{id_a}/threads"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (s, _) = a
        .json("GET", &format!("/api/nodes/{id_b}/nodes"), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = a
        .json("POST", &format!("/api/nodes/{id_b}/pairing/invite"), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = a
        .json("GET", &format!("/api/nodes/{id_b}/../nodes"), None)
        .await;
    assert!(s.is_client_error());
    // Nothing leaked into b's pairing state.
    let (s, _) = b.json("GET", "/api/pairing/pending", None).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn private_policy_enforced_on_target() {
    let (a, b, id_b) = paired(None, None).await;
    let (s, v) = a
        .json(
            "POST",
            &format!("/api/nodes/{id_b}/threads"),
            Some(json!({"provider": "claude", "space": "private"})),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let (_, v) = b.json("GET", "/api/threads", None).await;
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn forwarding_to_self_is_local_and_not_reforwardable() {
    let (a, _b, _) = paired(None, None).await;
    let id_a = a.id().await;
    let (s, v) = a
        .json("GET", &format!("/api/nodes/{id_a}/threads"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(v.as_array().unwrap().is_empty());
    let (s, _) = a
        .json("GET", &format!("/api/nodes/{id_a}/nodes"), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn tunnel_allow_list_denies_everything_else() {
    let (a, b, id_b) = paired(None, None).await;
    let denied: [(&str, &str, Option<Value>); 6] = [
        ("PATCH", "settings", Some(json!({"node_name": "evil"}))),
        ("GET", "settings", None),
        ("GET", "pairing/pending", None),
        (
            "POST",
            "pairing/join",
            Some(json!({"code": "x", "name": "y"})),
        ),
        ("GET", "nodes", None),
        ("GET", "unknown", None),
    ];
    for (m, p, body) in denied {
        let (s, v) = a.json(m, &format!("/api/nodes/{id_b}/{p}"), body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {p}: {v}");
    }
    for p in ["threads/../settings", "threads/%2e%2e/settings"] {
        let (s, _) = a.json("GET", &format!("/api/nodes/{id_b}/{p}"), None).await;
        assert!(s.is_client_error(), "{p}: {s}");
    }
    // Nothing changed on b.
    let (_, s) = b.json("GET", "/api/settings", None).await;
    assert_ne!(s["node_name"], "evil");
    // The allow-listed classes still work.
    for p in ["threads", "providers", "files/public"] {
        let (s, v) = a.json("GET", &format!("/api/nodes/{id_b}/{p}"), None).await;
        assert!(s.is_success(), "{p}: {s} {v}");
    }
}

// ---------------------------------------------------------------- trust levels

async fn mk_thread(n: &N, space: &str) -> String {
    let (s, t) = n
        .json(
            "POST",
            "/api/threads",
            Some(json!({"provider": "ollama", "space": space, "model": "m"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{t}");
    t["id"].as_str().unwrap().to_string()
}

fn trust_of(list: &Value, id: &str) -> Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == id)
        .unwrap()["trust"]
        .clone()
}

#[tokio::test]
async fn public_trust_cannot_reach_private_data() {
    // Default: the host did not tick "trust private"; b sees a as public.
    let (a, b, id_b) = paired(None, None).await;
    let id_a = a.id().await;
    let (_, la) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&la, &id_b), "public");
    let (_, lb) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&lb, &id_a), "public");
    let (pubt, privt) = (
        mk_thread(&b, "public").await,
        mk_thread(&b, "private").await,
    );
    let base = format!("/api/nodes/{id_b}");
    // List is filtered.
    let (s, v) = a.json("GET", &format!("{base}/threads"), None).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let ids: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].clone())
        .collect();
    assert_eq!(ids, [json!(pubt)]);
    // Everything addressing the private thread is 403.
    for (m, p, body) in [
        ("GET", format!("threads/{privt}/messages"), None),
        (
            "POST",
            format!("threads/{privt}/messages"),
            Some(json!({"content": "hi"})),
        ),
        ("POST", format!("threads/{privt}/cancel"), None),
        (
            "PATCH",
            format!("threads/{privt}"),
            Some(json!({"approval": "auto"})),
        ),
        ("DELETE", format!("threads/{privt}"), None),
        ("GET", "files/private".into(), None),
        ("GET", "files/private/content?path=x".into(), None),
        (
            "POST",
            "threads".into(),
            Some(json!({"provider": "ollama", "space": "private", "model": "m"})),
        ),
        (
            "POST",
            format!("threads/{pubt}/messages"),
            Some(json!({"content": "x", "attachments": [{"space": "private", "path": "p"}]})),
        ),
    ] {
        let (s, v) = a.json(m, &format!("{base}/{p}"), body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {p}: {v}");
    }
    // Nothing happened to the private thread; public stuff works.
    let (_, mine) = b.json("GET", "/api/threads", None).await;
    assert_eq!(mine.as_array().unwrap().len(), 2);
    let (s, _) = a
        .json("GET", &format!("{base}/threads/{pubt}/messages"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, _) = a.json("GET", &format!("{base}/files/public"), None).await;
    assert_eq!(s, StatusCode::OK);
}

#[tokio::test]
async fn full_trust_reaches_private_and_trust_is_changeable() {
    let (a, b, id_b) = paired_trust(None, None, Some(true)).await;
    let id_a = a.id().await;
    let (_, la) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&la, &id_b), "full");
    let privt = mk_thread(&b, "private").await;
    let base = format!("/api/nodes/{id_b}");
    let (_, v) = a.json("GET", &format!("{base}/threads"), None).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    let (s, _) = a
        .json("GET", &format!("{base}/threads/{privt}/messages"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    // b took a for full as well (it received the private key); b downgrades a locally.
    let (_, lb) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&lb, &id_a), "full");
    let (s, _) = b
        .json(
            "PATCH",
            &format!("/api/nodes/{id_a}/trust"),
            Some(json!({"trust": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (s, _) = a
        .json("GET", &format!("{base}/threads/{privt}/messages"), None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, v) = a.json("GET", &format!("{base}/threads"), None).await;
    assert!(v.as_array().unwrap().is_empty());
    // And upgrades it again.
    b.json(
        "PATCH",
        &format!("/api/nodes/{id_a}/trust"),
        Some(json!({"trust": "full"})),
    )
    .await;
    let (s, _) = a
        .json("GET", &format!("{base}/threads/{privt}/messages"), None)
        .await;
    assert_eq!(s, StatusCode::OK);
    // Bad input.
    let (s, _) = b
        .json(
            "PATCH",
            &format!("/api/nodes/{id_a}/trust"),
            Some(json!({"trust": "root"})),
        )
        .await;
    assert!(s.is_client_error());
    let (s, _) = b
        .json(
            "PATCH",
            "/api/nodes/aaaaaaaaaaaaaaaaaaaaaaaaaa/trust",
            Some(json!({"trust": "full"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

fn private_state(n: &N) -> (u64, Option<String>) {
    let v: Value =
        serde_json::from_slice(&std::fs::read(n.dir.path().join("membership.json")).unwrap())
            .unwrap();
    (
        v["private_epoch"].as_u64().unwrap_or(0),
        v["private_key"].as_str().map(str::to_string),
    )
}

async fn wait_epoch(n: &N, epoch: u64) {
    for _ in 0..100 {
        if private_state(n).0 == epoch {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("epoch {epoch} never arrived: {:?}", private_state(n).0);
}

/// a hosts b and c, both full. Returns (a, b, c, id_b, id_c).
async fn trio() -> (N, N, N, String, String) {
    let url = relay().await;
    let a = start(&url, "alpha", None, None).await;
    let b = start(&url, "beta", None, None).await;
    let c = start(&url, "gamma", None, None).await;
    pair_nodes(&a, &b, Some(true)).await;
    let id_b = b.id().await;
    wait_online(&a, &id_b).await;
    pair_nodes(&a, &c, Some(true)).await;
    let id_c = c.id().await;
    wait_online(&a, &id_c).await;
    // b learns about c from a's announcement.
    for _ in 0..100 {
        let (_, v) = b.json("GET", "/api/nodes", None).await;
        if v.as_array().unwrap().len() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (_, v) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&v, &id_c), "full");
    (a, b, c, id_b, id_c)
}

#[tokio::test]
async fn downgrade_rotates_private_key_without_the_downgraded() {
    let (a, b, c, _id_b, id_c) = trio().await;
    let (e0, k0) = private_state(&a);
    assert_eq!(private_state(&b), (e0, k0.clone()));
    assert_eq!(private_state(&c), (e0, k0.clone()));
    let (s, _) = a
        .json(
            "PATCH",
            &format!("/api/nodes/{id_c}/trust"),
            Some(json!({"trust": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (ea, ka) = private_state(&a);
    assert_eq!(ea, e0 + 1);
    assert_ne!(ka, k0);
    wait_epoch(&b, e0 + 1).await;
    assert_eq!(private_state(&b).1, ka);
    // The downgraded node keeps the old epoch and key.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(private_state(&c), (e0, k0));
    // Upgrading again sends the current key.
    a.json(
        "PATCH",
        &format!("/api/nodes/{id_c}/trust"),
        Some(json!({"trust": "full"})),
    )
    .await;
    wait_epoch(&c, e0 + 1).await;
    assert_eq!(private_state(&c).1, ka);
    assert_eq!(private_state(&a).0, e0 + 1);
}

#[tokio::test]
async fn removing_a_member_rotates_and_is_local_only() {
    let (a, b, c, id_b, id_c) = trio().await;
    let (e0, _) = private_state(&a);
    // Not tunnelable.
    for (m, p) in [
        ("PATCH", format!("/api/nodes/{id_b}/nodes/{id_c}/trust")),
        ("DELETE", format!("/api/nodes/{id_b}/nodes/{id_c}")),
    ] {
        let (s, _) = a.json(m, &p, Some(json!({"trust": "public"}))).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {p}");
    }
    let (s, _) = a
        .json("DELETE", "/api/nodes/aaaaaaaaaaaaaaaaaaaaaaaaaa", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = a.json("DELETE", &format!("/api/nodes/{id_c}"), None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, v) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(v.as_array().unwrap().len(), 2);
    let (ea, ka) = private_state(&a);
    assert_eq!(ea, e0 + 1);
    wait_epoch(&b, e0 + 1).await;
    assert_eq!(private_state(&b).1, ka);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(private_state(&c).0, e0);
    // Cannot talk to the removed node any more.
    let (s, _) = a
        .json("GET", &format!("/api/nodes/{id_c}/threads"), None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------- private sync (stage 4b, step 2)

const MULTIPART_B: &str = "XBOUNDARYX";

async fn put_private(n: &N, path: &str, data: &[u8]) {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let mut body = format!(
        "--{MULTIPART_B}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{MULTIPART_B}--\r\n").as_bytes());
    let mut b = Request::builder()
        .method("POST")
        .uri(format!("/api/files/private?path={dir}"))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={MULTIPART_B}"),
        );
    if let Some(t) = &n.token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let r = n
        .app
        .clone()
        .oneshot(b.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.status());
}

async fn raw(n: &N, method: &str, uri: &str) -> (StatusCode, Vec<u8>) {
    let r = n.call(method, uri, None).await;
    let s = r.status();
    (
        s,
        r.into_body().collect().await.unwrap().to_bytes().to_vec(),
    )
}

async fn read_private(n: &N, path: &str) -> Option<Vec<u8>> {
    let (s, b) = raw(n, "GET", &format!("/api/files/private/content?path={path}")).await;
    (s == StatusCode::OK).then_some(b)
}

/// Polls until `path` has exactly `want` (None = absent), within a timeout.
async fn wait_private(n: &N, path: &str, want: Option<&[u8]>) {
    for _ in 0..150 {
        if read_private(n, path).await.as_deref() == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "{path}: expected {want:?}, got {:?}",
        read_private(n, path).await
    );
}

/// Stops the node and returns its data dir, to start it again with `restart`.
fn take_offline(n: N) -> tempfile::TempDir {
    n.dir
}

async fn restart(dir: tempfile::TempDir) -> N {
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake(),
        agy_bin: fake(),
        openrouter_base: String::new(),
        ..Config::new(None)
    };
    let hub = Arc::new(Hub::new(cfg, "test-node").unwrap());
    let node = Node::start(
        hub.clone(),
        dir.path(),
        NodeOpts {
            connect_timeout: Duration::from_millis(1500),
        },
    )
    .unwrap();
    let app = octo_server::app_with_node(hub, node.clone(), None, None);
    N {
        app,
        _node: node,
        dir,
        token: None,
    }
}

#[tokio::test]
async fn sync_routes_local_manifest_and_blob() {
    let url = relay().await;
    let a = start(&url, "alpha", None, None).await;
    put_private(&a, "docs/a.txt", b"hello").await;
    let (s, m) = a.json("GET", "/api/sync/private/manifest", None).await;
    assert_eq!(s, StatusCode::OK, "{m}");
    let e = m
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["path"] == "docs/a.txt")
        .unwrap()
        .clone();
    let hash = e["hash"].as_str().unwrap().to_string();
    assert_eq!(e["deleted"], false);
    let (s, body) = raw(&a, "GET", &format!("/api/sync/private/blob/{hash}")).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(body, b"hello");
    let unknown = "0".repeat(64);
    let (s, _) = raw(&a, "GET", &format!("/api/sync/private/blob/{unknown}")).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    for bad in ["zz", "not-a-hash", "ABCDEF"] {
        let (s, _) = raw(&a, "GET", &format!("/api/sync/private/blob/{bad}")).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
}

#[tokio::test]
async fn sync_routes_reachable_through_tunnel_by_full_member() {
    let (a, b, id_b) = paired_trust(None, None, Some(true)).await;
    put_private(&b, "x.txt", b"x").await;
    let (s, m) = a
        .json(
            "GET",
            &format!("/api/nodes/{id_b}/sync/private/manifest"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{m}");
    assert!(m.as_array().unwrap().iter().any(|e| e["path"] == "x.txt"));
}

#[tokio::test]
async fn private_file_syncs_to_full_member_and_delete_propagates() {
    let (a, b, _) = paired_trust(None, None, Some(true)).await;
    put_private(&a, "notes/n.txt", b"v1").await;
    wait_private(&b, "notes/n.txt", Some(b"v1")).await;
    // B deletes -> A loses it.
    let (s, _) = raw(&b, "DELETE", "/api/files/private?path=notes/n.txt").await;
    assert!(s.is_success(), "{s}");
    wait_private(&a, "notes/n.txt", None).await;
    wait_private(&b, "notes/n.txt", None).await;
}

#[tokio::test]
async fn public_member_gets_403_and_no_private_files() {
    // a hosts b (full) and c (public).
    let url = relay().await;
    let a = start(&url, "alpha", None, None).await;
    let b = start(&url, "beta", None, None).await;
    let c = start(&url, "gamma", None, None).await;
    pair_nodes(&a, &b, Some(true)).await;
    let id_b = b.id().await;
    wait_online(&a, &id_b).await;
    pair_nodes(&a, &c, Some(false)).await;
    let (id_a, id_c) = (a.id().await, c.id().await);
    wait_online(&a, &id_c).await;
    for p in [
        format!("/api/nodes/{id_a}/sync/private/manifest"),
        format!("/api/nodes/{id_a}/sync/private/blob/{}", "0".repeat(64)),
    ] {
        let (s, _) = raw(&c, "GET", &p).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "c -> a {p}");
    }
    for p in [
        format!("/api/nodes/{id_c}/sync/private/manifest"),
        format!("/api/nodes/{id_c}/sync/private/blob/{}", "0".repeat(64)),
    ] {
        let (s, _) = raw(&a, "GET", &p).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "a -> c {p}");
    }
    put_private(&a, "secret.txt", b"s").await;
    // Positive control: the full member gets it, so a sync round has certainly happened.
    wait_private(&b, "secret.txt", Some(b"s")).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (s, list) = c.json("GET", "/api/files/private", None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

#[tokio::test]
async fn offline_member_catches_up_after_reconnect() {
    let (a, b, _) = paired_trust(None, None, Some(true)).await;
    let id_b = b.id().await;
    put_private(&a, "one.txt", b"1").await;
    wait_private(&b, "one.txt", Some(b"1")).await;
    let dir = take_offline(b);
    put_private(&a, "two.txt", b"2").await;
    let (s, _) = raw(&a, "DELETE", "/api/files/private?path=one.txt").await;
    assert!(s.is_success());
    let b = restart(dir).await;
    wait_online(&a, &id_b).await;
    wait_private(&b, "two.txt", Some(b"2")).await;
    wait_private(&b, "one.txt", None).await;
}

#[tokio::test]
async fn private_key_epoch_catches_up_for_member_offline_during_rotation() {
    let (a, b, c, id_b, id_c) = trio().await;
    let (e0, _) = private_state(&a);
    let dir_c = take_offline(c);
    // Downgrading b rotates the key; c is full but offline and misses it.
    let (s, _) = a
        .json(
            "PATCH",
            &format!("/api/nodes/{id_b}/trust"),
            Some(json!({"trust": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (ea, ka) = private_state(&a);
    assert_eq!(ea, e0 + 1);
    let c = restart(dir_c).await;
    wait_online(&a, &id_c).await;
    wait_epoch(&c, e0 + 1).await;
    assert_eq!(private_state(&c).1, ka);
    drop(b);
}

/// `target` lets `member` run CLI agents through the tunnel.
async fn allow_agents(target: &N, member: &N) {
    let id = member.id().await;
    let (s, _) = target
        .json(
            "PATCH",
            &format!("/api/nodes/{id}/agents"),
            Some(json!({"agents": true})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

async fn wait_member(n: &N, id: &str) {
    for _ in 0..100 {
        let (_, v) = n.json("GET", "/api/nodes", None).await;
        if v.as_array().unwrap().iter().any(|x| x["node_id"] == id) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("member never appeared");
}

fn agents_of(list: &Value, id: &str) -> Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|n| n["node_id"] == id)
        .unwrap()["agents"]
        .clone()
}

#[tokio::test]
async fn agents_permission_gates_cli_use_through_the_tunnel() {
    // b is the target; a is a (public-trust) member of b. Default: agents off.
    let (a, b, id_b) = paired(None, None).await;
    let id_a = a.id().await;
    let (_, lb) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(agents_of(&lb, &id_a), false);
    assert_eq!(agents_of(&lb, &b.id().await), Value::Null);
    let base = format!("/api/nodes/{id_b}");
    let http_thread = mk_thread(&b, "public").await;
    let (s, cli) = b
        .json(
            "POST",
            "/api/threads",
            Some(json!({"provider": "claude", "space": "public"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{cli}");
    let cli = cli["id"].as_str().unwrap().to_string();
    let denied = [
        (
            "POST",
            "threads".to_string(),
            Some(json!({"provider": "claude", "space": "public"})),
        ),
        (
            "POST",
            "threads".into(),
            Some(json!({"provider": "antigravity", "space": "public"})),
        ),
        (
            "POST",
            "threads".into(),
            Some(
                json!({"provider": "ollama", "space": "public", "model": "m", "approval": "confirm"}),
            ),
        ),
        (
            "POST",
            "threads".into(),
            Some(json!({"provider": "ollama", "space": "public", "model": "m", "cwd": "/tmp"})),
        ),
        (
            "POST",
            format!("threads/{cli}/messages"),
            Some(json!({"content": "hi"})),
        ),
        (
            "PATCH",
            format!("threads/{cli}"),
            Some(json!({"approval": "auto"})),
        ),
        (
            "PATCH",
            format!("threads/{http_thread}"),
            Some(json!({"approval": "confirm"})),
        ),
        ("GET", "import/claude/sessions".into(), None),
        (
            "POST",
            "import/claude".into(),
            Some(json!({"external_id": "x"})),
        ),
    ];
    for (m, p, body) in &denied {
        let (s, v) = a.json(m, &format!("{base}/{p}"), body.clone()).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{m} {p}: {v}");
    }
    // HTTP providers stay available.
    let (s, v) = a
        .json(
            "POST",
            &format!("{base}/threads"),
            Some(json!({"provider": "ollama", "space": "public", "model": "m"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // Local-only API; turning it on opens the CLI routes.
    let (s, _) = b
        .json(
            "PATCH",
            &format!("/api/nodes/{id_a}/agents"),
            Some(json!({"agents": true})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    let (_, lb) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(agents_of(&lb, &id_a), true);
    for (m, p, body) in &denied[..2] {
        let (s, v) = a.json(m, &format!("{base}/{p}"), body.clone()).await;
        assert!(s.is_success(), "{m} {p}: {s} {v}");
    }
    let (s, _) = a
        .json("GET", &format!("{base}/import/claude/sessions"), None)
        .await;
    assert_ne!(s, StatusCode::FORBIDDEN);
    // The other side never learns this opinion.
    let (_, la) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(agents_of(&la, &id_b), false);
    let (s, _) = b
        .json(
            "PATCH",
            "/api/nodes/aaaaaaaaaaaaaaaaaaaaaaaaaa/agents",
            Some(json!({"agents": true})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn pairing_confirm_carries_allow_agents_on_both_sides() {
    let url = relay().await;
    let a = start(&url, "alpha", None, None).await;
    let b = start(&url, "beta", None, None).await;
    pair_nodes_agents(&a, &b, Some(false), Some(true), Some(false)).await;
    let (id_a, id_b) = (a.id().await, b.id().await);
    wait_online(&a, &id_b).await;
    wait_member(&b, &id_a).await;
    let (_, la) = a.json("GET", "/api/nodes", None).await;
    let (_, lb) = b.json("GET", "/api/nodes", None).await;
    assert_eq!(agents_of(&la, &id_b), true, "host allowed the joiner");
    assert_eq!(
        agents_of(&lb, &id_a),
        false,
        "joiner did not allow the host"
    );
    let c = start(&url, "gamma", None, None).await;
    pair_nodes_agents(&b, &c, Some(false), None, Some(true)).await;
    let id_c = c.id().await;
    let id_b2 = b.id().await;
    wait_member(&c, &id_b2).await;
    let _ = id_c;
    let (_, lc) = c.json("GET", "/api/nodes", None).await;
    assert_eq!(agents_of(&lc, &id_b), true, "joiner allowed the host");
}

#[tokio::test]
async fn thread_lookup_failure_denies_tunnel_callers_and_hides_details() {
    let (a, b, id_b) = paired(None, None).await;
    // Break b's thread store; `cancel` itself never touches it, so only the policy lookup fails.
    let db = rusqlite::Connection::open(b.dir.path().join("octo.db")).unwrap();
    db.execute_batch("DROP TABLE threads;").unwrap();
    let (s, v) = a
        .json(
            "POST",
            &format!("/api/nodes/{id_b}/threads/t1/cancel"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR, "{v}");
    assert!(
        !v.to_string().contains("threads"),
        "no internals leaked: {v}"
    );
    // The local caller is not affected by the lookup (cancel is always 204).
    let (s, _) = b.json("POST", "/api/threads/t1/cancel", None).await;
    assert_eq!(s, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn pairing_does_not_overwrite_changes_made_while_it_runs() {
    let (a, _b, id_b) = paired(None, None).await;
    let url = relay_of(&a).await;
    let c = start(&url, "gamma", None, None).await;
    let (_, inv) = a.json("POST", "/api/pairing/invite", None).await;
    c.json(
        "POST",
        "/api/pairing/join",
        Some(json!({"code": inv["code"], "name": "gamma"})),
    )
    .await;
    a.pending().await;
    c.pending().await;
    // The host decides first; the exchange then waits for the joiner's confirmation.
    assert!(
        a.call("POST", "/api/pairing/confirm", None)
            .await
            .status()
            .is_success()
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (s, _) = a
        .json(
            "PATCH",
            &format!("/api/nodes/{id_b}/trust"),
            Some(json!({"trust": "full"})),
        )
        .await;
    assert_eq!(s, StatusCode::NO_CONTENT);
    assert!(
        c.call("POST", "/api/pairing/confirm", None)
            .await
            .status()
            .is_success()
    );
    let id_c = c.id().await;
    wait_online(&a, &id_c).await;
    let (_, la) = a.json("GET", "/api/nodes", None).await;
    assert_eq!(trust_of(&la, &id_b), "full", "{la}");
    assert_eq!(la.as_array().unwrap().len(), 3);
}
