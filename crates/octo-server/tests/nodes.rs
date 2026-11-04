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
    let hub = Arc::new(Hub::new(cfg).unwrap());
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

/// Pairs b into a's network through the API. Returns (a, b).
async fn paired(tok_a: Option<&str>, tok_b: Option<&str>) -> (N, N, String) {
    let url = relay().await;
    let a = start(&url, "alpha", tok_a, None).await;
    let b = start(&url, "beta", tok_b, None).await;
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
    assert!(
        a.call("POST", "/api/pairing/confirm", None)
            .await
            .status()
            .is_success()
    );
    assert!(
        b.call("POST", "/api/pairing/confirm", None)
            .await
            .status()
            .is_success()
    );
    let id_b = b.id().await;
    // a lists b as online (eventually: relay link of a is re-established after pairing).
    let mut ok = false;
    for _ in 0..100 {
        let (_, v) = a.json("GET", "/api/nodes", None).await;
        let v = v.as_array().unwrap().clone();
        if v.len() == 2
            && v.iter()
                .any(|n| n["node_id"] == id_b.as_str() && n["online"] == true)
        {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ok, "peer never online");
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
