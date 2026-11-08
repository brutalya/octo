use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use octo_core::{Config, Hub};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const S1: &str = "11111111-1111-4111-8111-111111111111";
const A1: &str = "aaaaaaaa-0000-4000-8000-000000000001";

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn app() -> (Router, tempfile::TempDir) {
    let fixtures = PathBuf::from(format!(
        "{}/../../tests/fixtures/import",
        env!("CARGO_MANIFEST_DIR")
    ));
    let root = tempfile::tempdir().unwrap();
    copy_dir(
        &fixtures.join("claude/projects"),
        &root.path().join("projects"),
    );
    copy_dir(&fixtures.join("agy"), &root.path().join("agy"));
    // The sqlite index is generated from its committed SQL dump (*.db is gitignored).
    let sql = std::fs::read_to_string(fixtures.join("agy/conversation_summaries.sql")).unwrap();
    rusqlite::Connection::open(root.path().join("agy/conversation_summaries.db"))
        .unwrap()
        .execute_batch(&sql)
        .unwrap();
    let mut cfg = Config::new(Some(root.path().join("data")));
    cfg.claude_projects_dir = root.path().join("projects");
    cfg.agy_dir = root.path().join("agy");
    (
        octo_server::app(
            Arc::new(Hub::new(cfg, "test-node").unwrap()),
            Default::default(),
        ),
        root,
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

async fn json_of(r: Response<Body>) -> Value {
    let bytes = r.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn lists_sessions_newest_first_with_search() {
    let (app, _d) = app();
    let r = call(&app, "GET", "/api/import/claude/sessions", None).await;
    assert_eq!(r.status(), StatusCode::OK);
    let v = json_of(r).await;
    assert_eq!(v.as_array().unwrap().len(), 2);
    let first = &v[0];
    for k in ["external_id", "title", "cwd", "updated_at", "messages"] {
        assert!(first.get(k).is_some(), "{k}");
    }
    let v = json_of(call(&app, "GET", "/api/import/claude/sessions?q=windows", None).await).await;
    assert_eq!(v.as_array().unwrap().len(), 1);
    let v = json_of(call(&app, "GET", "/api/import/antigravity/sessions", None).await).await;
    assert_eq!(v.as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn import_is_idempotent_and_listing_links_the_thread() {
    let (app, _d) = app();
    let body = json!({"external_id": S1});
    let r = call(&app, "POST", "/api/import/claude", Some(body.clone())).await;
    assert_eq!(r.status(), StatusCode::OK);
    let t = json_of(r).await;
    assert_eq!(t["origin"], "imported");
    assert_eq!(t["space"], "public");
    assert_eq!(t["cwd"], "/srv/demo");
    let again = json_of(call(&app, "POST", "/api/import/claude", Some(body)).await).await;
    assert_eq!(again["id"], t["id"]);
    let threads = json_of(call(&app, "GET", "/api/threads", None).await).await;
    assert_eq!(threads.as_array().unwrap().len(), 1);
    let m = json_of(
        call(
            &app,
            "GET",
            &format!("/api/threads/{}/messages", t["id"].as_str().unwrap()),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(m.as_array().unwrap().len(), 4);
    let list = json_of(call(&app, "GET", "/api/import/claude/sessions", None).await).await;
    let s = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["external_id"] == S1)
        .unwrap();
    assert_eq!(s["imported_thread_id"], t["id"]);
}

#[tokio::test]
async fn antigravity_import_works() {
    let (app, _d) = app();
    let r = call(
        &app,
        "POST",
        "/api/import/antigravity",
        Some(json!({"external_id": A1})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(json_of(r).await["title"], "Refactor parser");
}

#[tokio::test]
async fn errors() {
    let (app, _d) = app();
    let r = call(
        &app,
        "POST",
        "/api/import/claude",
        Some(json!({"external_id": "nope"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = call(
        &app,
        "POST",
        "/api/import/claude",
        Some(json!({"external_id": "../etc"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let r = call(&app, "GET", "/api/import/ollama/sessions", None).await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = call(
        &app,
        "POST",
        "/api/import/openrouter",
        Some(json!({"external_id": "x"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn thread_can_be_created_with_a_cwd() {
    let (app, _d) = app();
    let r = call(
        &app,
        "POST",
        "/api/threads",
        Some(json!({"provider": "claude", "space": "public", "cwd": "/work/x"})),
    )
    .await;
    let t = json_of(r).await;
    assert_eq!(t["cwd"], "/work/x");
    assert_eq!(t["origin"], "octo");
}
