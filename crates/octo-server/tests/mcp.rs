#![cfg(unix)] // uses tests/fixtures/fake_cli.sh; unix only
use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use octo_core::providers::ProviderKind;
use octo_core::storage::{Approval, Space};
use octo_core::{Config, Hub};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

struct Env {
    app: Router,
    hub: Arc<Hub>,
    _dir: tempfile::TempDir,
}

fn env_with(token: Option<&str>) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let fake = PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_cli.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let cfg = Config {
        data_dir: dir.path().to_path_buf(),
        claude_bin: fake.clone(),
        agy_bin: fake,
        openrouter_base: String::new(),
        // a bound API address makes confirm-mode Claude runs use the approval endpoint
        local_api: Some("127.0.0.1:1".parse().unwrap()),
        ..Config::new(None)
    };
    let hub = Arc::new(Hub::new(cfg, "test-node").unwrap());
    let app = octo_server::app(
        hub.clone(),
        octo_server::AppConfig {
            token: token.map(String::from),
            ..Default::default()
        },
    );
    Env {
        app,
        hub,
        _dir: dir,
    }
}

/// A Claude thread with a run in progress (fake CLI hangs on SLOW); returns the thread id
/// and the run token. The receiver keeps the stream open.
async fn running(
    e: &Env,
) -> (
    String,
    String,
    tokio::sync::mpsc::Receiver<octo_core::providers::AgentEvent>,
) {
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
    let rx = e.hub.send_message(&t.id, "SLOW", &[]).await.unwrap();
    let token = e.hub.run_token(&t.id).expect("run token while running");
    (t.id, token, rx)
}

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    headers: &[(&str, &str)],
    body: Option<String>,
) -> Response<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    let body = body.map(Body::from).unwrap_or_else(Body::empty);
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn body_text(r: Response<Body>) -> String {
    String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

async fn rpc(app: &Router, token: &str, body: Value) -> (StatusCode, String) {
    let r = send(
        app,
        "POST",
        &format!("/mcp/{token}"),
        &[
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
        ],
        Some(body.to_string()),
    )
    .await;
    let s = r.status();
    (s, body_text(r).await)
}

async fn rpc_json(app: &Router, token: &str, body: Value) -> Value {
    let (s, t) = rpc(app, token, body).await;
    assert_eq!(s, StatusCode::OK, "{t}");
    serde_json::from_str(&t).unwrap()
}

fn approve_call(id: u64, input: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{
        "name":"approve",
        "arguments":{"tool_name":"Bash","input":input,"tool_use_id":"toolu_1"}}})
}

async fn pending(app: &Router, thread: &str) -> Vec<Value> {
    let r = send(
        app,
        "GET",
        &format!("/api/threads/{thread}/approvals"),
        &[],
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    serde_json::from_str::<Vec<Value>>(&body_text(r).await).unwrap()
}

async fn wait_pending(app: &Router, thread: &str) -> Value {
    for _ in 0..200 {
        if let Some(p) = pending(app, thread).await.into_iter().next() {
            return p;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("no pending approval appeared");
}

async fn decide(app: &Router, thread: &str, aid: &str, allow: bool) -> StatusCode {
    send(
        app,
        "POST",
        &format!("/api/threads/{thread}/approvals/{aid}"),
        &[("content-type", "application/json")],
        Some(json!({"allow": allow}).to_string()),
    )
    .await
    .status()
}

#[tokio::test]
async fn unknown_token_is_404() {
    let e = env_with(None);
    let (s, _) = rpc(
        &e.app,
        "0000",
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn initialize_announces_tools() {
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let v = rpc_json(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-03-26","capabilities":{},
            "clientInfo":{"name":"claude","version":"1"}}}),
    )
    .await;
    assert_eq!(v["jsonrpc"], "2.0");
    assert_eq!(v["id"], 1);
    assert_eq!(v["result"]["protocolVersion"], "2025-03-26");
    assert!(v["result"]["capabilities"]["tools"].is_object());
    assert_eq!(v["result"]["serverInfo"]["name"], "octo");
}

#[tokio::test]
async fn initialized_notification_is_202_without_body() {
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let (s, body) = rpc(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED);
    assert!(body.is_empty());
}

#[tokio::test]
async fn tools_list_has_the_approve_tool_only() {
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let v = rpc_json(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":7,"method":"tools/list"}),
    )
    .await;
    assert_eq!(v["id"], 7);
    let tools = v["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "approve");
    let props = &tools[0]["inputSchema"]["properties"];
    for k in ["tool_name", "input", "tool_use_id"] {
        assert!(props[k].is_object(), "{k}");
    }
}

#[tokio::test]
async fn unknown_method_and_tool_are_jsonrpc_errors() {
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let v = rpc_json(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}),
    )
    .await;
    assert_eq!(v["id"], 3);
    assert_eq!(v["error"]["code"], -32601);
    assert!(v.get("result").is_none());
    let v = rpc_json(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"rm","arguments":{}}}),
    )
    .await;
    assert_eq!(v["id"], 4);
    assert_eq!(v["error"]["code"], -32602);
}

#[tokio::test]
async fn malformed_body_is_a_parse_error() {
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let r = send(
        &e.app,
        "POST",
        &format!("/mcp/{token}"),
        &[("content-type", "application/json")],
        Some("{nope".into()),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let v: Value = serde_json::from_str(&body_text(r).await).unwrap();
    assert_eq!(v["error"]["code"], -32700);
    assert!(v["id"].is_null());
}

#[tokio::test]
async fn call_blocks_until_allowed() {
    let e = env_with(None);
    let (id, token, _rx) = running(&e).await;
    let call = tokio::spawn({
        let app = e.app.clone();
        async move { rpc_json(&app, &token, approve_call(9, json!({"command":"ls"}))).await }
    });
    let p = wait_pending(&e.app, &id).await;
    assert_eq!(p["tool"], "Bash");
    assert_eq!(p["summary"], "Bash: ls");
    assert!(!call.is_finished(), "must wait for the decision");
    let aid = p["id"].as_str().unwrap();
    assert_eq!(decide(&e.app, &id, aid, true).await, StatusCode::NO_CONTENT);
    let v = call.await.unwrap();
    assert_eq!(v["id"], 9);
    assert_ne!(v["result"]["isError"], true);
    assert_eq!(v["result"]["content"][0]["type"], "text");
    let decision: Value =
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        decision,
        json!({"behavior":"allow","updatedInput":{"command":"ls"}})
    );
    assert!(pending(&e.app, &id).await.is_empty());
}

#[tokio::test]
async fn call_returns_deny_with_message() {
    let e = env_with(None);
    let (id, token, _rx) = running(&e).await;
    let call = tokio::spawn({
        let app = e.app.clone();
        async move { rpc_json(&app, &token, approve_call(2, json!({"command":"rm -rf x"}))).await }
    });
    let p = wait_pending(&e.app, &id).await;
    let aid = p["id"].as_str().unwrap();
    assert_eq!(
        decide(&e.app, &id, aid, false).await,
        StatusCode::NO_CONTENT
    );
    let v = call.await.unwrap();
    let decision: Value =
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(decision["behavior"], "deny");
    assert!(!decision["message"].as_str().unwrap().is_empty());
}

#[tokio::test]
async fn cancelling_the_run_denies_and_invalidates_the_token() {
    let e = env_with(None);
    let (id, token, rx) = running(&e).await;
    let call = tokio::spawn({
        let app = e.app.clone();
        let token = token.clone();
        async move { rpc_json(&app, &token, approve_call(5, json!({"command":"ls"}))).await }
    });
    wait_pending(&e.app, &id).await;
    e.hub.cancel(&id);
    let v = call.await.unwrap();
    let decision: Value =
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(decision["behavior"], "deny");
    assert!(pending(&e.app, &id).await.is_empty());
    drop(rx);
    for _ in 0..200 {
        if !e.hub.is_running(&id) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (s, _) = rpc(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "token must die with the run");
    assert!(e.hub.run_token(&id).is_none());
}

#[tokio::test]
async fn each_run_gets_its_own_token() {
    let e = env_with(None);
    let (_a, ta, _ra) = running(&e).await;
    let (_b, tb, _rb) = running(&e).await;
    assert_ne!(ta, tb);
    assert_eq!(ta.len(), 64);
    assert!(ta.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn token_of_one_run_cannot_decide_for_another_thread() {
    let e = env_with(None);
    let (a, _ta, _ra) = running(&e).await;
    let (b, tb, _rb) = running(&e).await;
    let call = tokio::spawn({
        let app = e.app.clone();
        async move { rpc_json(&app, &tb, approve_call(1, json!({"command":"ls"}))).await }
    });
    let p = wait_pending(&e.app, &b).await;
    let aid = p["id"].as_str().unwrap();
    assert_eq!(decide(&e.app, &a, aid, true).await, StatusCode::NOT_FOUND);
    assert!(pending(&e.app, &a).await.is_empty());
    assert_eq!(decide(&e.app, &b, aid, false).await, StatusCode::NO_CONTENT);
    call.await.unwrap();
}

#[tokio::test]
async fn unknown_approval_or_thread_is_404() {
    let e = env_with(None);
    let (id, _t, _rx) = running(&e).await;
    assert_eq!(
        decide(&e.app, &id, "nope", true).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        decide(&e.app, "nope", "nope", true).await,
        StatusCode::NOT_FOUND
    );
    assert!(pending(&e.app, &id).await.is_empty());
}

#[tokio::test]
async fn mcp_route_is_outside_the_bearer_layer_but_inside_the_host_check() {
    // with an API token: no bearer needed on /mcp, still needed on /api
    let e = env_with(Some("secret"));
    let (_id, token, _rx) = running_with_bearer(&e).await;
    let (s, _) = rpc(
        &e.app,
        &token,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let r = send(&e.app, "GET", "/api/threads", &[], None).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);

    // without an API token the Host check guards /mcp
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let r = send(
        &e.app,
        "POST",
        &format!("/mcp/{token}"),
        &[
            ("host", "evil.example"),
            ("content-type", "application/json"),
        ],
        Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
    )
    .await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    let r = send(
        &e.app,
        "POST",
        &format!("/mcp/{token}"),
        &[
            ("host", "127.0.0.1:1234"),
            ("content-type", "application/json"),
        ],
        Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string()),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
}

async fn running_with_bearer(
    e: &Env,
) -> (
    String,
    String,
    tokio::sync::mpsc::Receiver<octo_core::providers::AgentEvent>,
) {
    running(e).await
}

#[tokio::test]
async fn get_on_the_mcp_route_is_not_a_stream() {
    // single JSON responses only: no server-initiated SSE stream
    let e = env_with(None);
    let (_id, token, _rx) = running(&e).await;
    let r = send(&e.app, "GET", &format!("/mcp/{token}"), &[], None).await;
    assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
}
