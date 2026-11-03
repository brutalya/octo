use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use octo_core::{Config, Hub};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;

fn app() -> (Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let fake = PathBuf::from(format!(
        "{}/../../tests/fixtures/fake_cli.sh",
        env!("CARGO_MANIFEST_DIR")
    ));
    let cfg = Config {
        data_dir: dir,
        claude_bin: fake.clone(),
        agy_bin: fake,
    };
    (
        octo_server::app(Arc::new(Hub::new(cfg).unwrap()), None),
        tmp,
    )
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> Response<Body> {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    app.clone().oneshot(b.body(body).unwrap()).await.unwrap()
}

async fn text(r: Response<Body>) -> String {
    String::from_utf8(r.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap()
}

async fn json_of(r: Response<Body>) -> Value {
    serde_json::from_str(&text(r).await).unwrap()
}

async fn new_thread(app: &Router, provider: &str, space: &str) -> String {
    let r = call(
        app,
        "POST",
        "/api/threads",
        Some(json!({"provider": provider, "space": space})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    json_of(r).await["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn providers_listed() {
    let (app, _d) = app();
    let v = json_of(call(&app, "GET", "/api/providers", None).await).await;
    let v = v.as_array().unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[0]["kind"], "claude");
    assert_eq!(v[0]["installed"], true);
    assert_eq!(v[0]["version"], "9.9.9 (fake)");
    assert_eq!(v[0]["is_local"], false);
}

#[tokio::test]
async fn private_non_local_rejected_on_create() {
    let (app, _d) = app();
    for provider in ["claude", "antigravity"] {
        let r = call(
            &app,
            "POST",
            "/api/threads",
            Some(json!({"provider": provider, "space": "private"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    let t = json_of(call(&app, "GET", "/api/threads", None).await).await;
    assert!(t.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn send_streams_and_persists() {
    let (app, _d) = app();
    let id = new_thread(&app, "claude", "public").await;
    let r = call(
        &app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(json!({"content": "hello there"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let body = text(r).await;
    assert!(body.contains("event: session"), "{body}");
    assert!(body.contains("event: text_delta"), "{body}");
    assert!(body.contains(r#""text":"ok""#), "{body}");
    assert!(body.contains("event: done"), "{body}");

    let m = json_of(call(&app, "GET", &format!("/api/threads/{id}/messages"), None).await).await;
    assert_eq!(m[0]["role"], "user");
    assert_eq!(m[0]["content"], "hello there");
    assert_eq!(m[1]["role"], "assistant");
    assert_eq!(m[1]["content"], "ok");

    let t = json_of(call(&app, "GET", "/api/threads", None).await).await;
    assert_eq!(t[0]["title"], "hello there");

    // follow-up resumes (fake picks the resume fixture) and appends
    let r = call(
        &app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(json!({"content": "again"})),
    )
    .await;
    assert!(text(r).await.contains("event: done"));
    let m = json_of(call(&app, "GET", &format!("/api/threads/{id}/messages"), None).await).await;
    assert_eq!(m.as_array().unwrap().len(), 4);
}

#[tokio::test]
async fn antigravity_streams() {
    let (app, _d) = app();
    let id = new_thread(&app, "antigravity", "public").await;
    let r = call(
        &app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(json!({"content": "hi"})),
    )
    .await;
    let body = text(r).await;
    assert!(
        body.contains("event: text_delta") && body.contains("event: done"),
        "{body}"
    );
}

#[tokio::test]
async fn process_failure_persists_error() {
    let (app, _d) = app();
    let id = new_thread(&app, "claude", "public").await;
    let r = call(
        &app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(json!({"content": "FAIL"})),
    )
    .await;
    assert!(text(r).await.contains("event: error"));
    let m = json_of(call(&app, "GET", &format!("/api/threads/{id}/messages"), None).await).await;
    assert_eq!(m[1]["role"], "error");
    assert!(m[1]["content"].as_str().unwrap().contains("fake failure"));
}

#[tokio::test]
async fn concurrent_run_409_then_cancel() {
    let (app, _d) = app();
    let id = new_thread(&app, "claude", "public").await;
    let uri = format!("/api/threads/{id}/messages");
    let mut first = call(&app, "POST", &uri, Some(json!({"content": "SLOW"}))).await;
    assert_eq!(first.status(), StatusCode::OK);
    let frame = first.body_mut().frame().await.unwrap().unwrap();
    let mut body = String::from_utf8_lossy(frame.data_ref().unwrap()).to_string();
    let second = call(&app, "POST", &uri, Some(json!({"content": "x"}))).await;
    assert_eq!(second.status(), StatusCode::CONFLICT);
    let c = call(&app, "POST", &format!("/api/threads/{id}/cancel"), None).await;
    assert_eq!(c.status(), StatusCode::NO_CONTENT);
    body += &tokio::time::timeout(Duration::from_secs(5), text(first))
        .await
        .unwrap();
    assert!(body.contains("event: session"), "{body}");
    assert!(!body.contains("event: done"));
}

#[tokio::test]
async fn delete_thread_and_unknown() {
    let (app, _d) = app();
    let id = new_thread(&app, "claude", "public").await;
    assert_eq!(
        call(&app, "DELETE", &format!("/api/threads/{id}"), None)
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    let r = call(
        &app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(json!({"content": "x"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn serves_web_dir_with_index_fallback() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("index.html"), "<html>octo</html>").unwrap();
    let (_, d) = app();
    let cfg = Config {
        data_dir: d.path().to_path_buf(),
        claude_bin: "x".into(),
        agy_bin: "x".into(),
    };
    let app = octo_server::app(
        Arc::new(Hub::new(cfg).unwrap()),
        Some(tmp.path().to_path_buf()),
    );
    for uri in ["/", "/some/client/route"] {
        let r = call(&app, "GET", uri, None).await;
        assert_eq!(r.status(), StatusCode::OK, "{uri}");
        assert_eq!(text(r).await, "<html>octo</html>");
    }
    let r = call(&app, "GET", "/api/threads", None).await;
    assert_eq!(r.status(), StatusCode::OK);
}
