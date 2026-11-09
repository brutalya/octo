//! Stage 6 over HTTP: private image to a (fake) local Ollama, and the 415 cases.
use axum::Router;
use axum::body::{Body, Bytes};
use axum::http::{Request, Response, StatusCode};
use axum::routing::post;
use base64::Engine as _;
use http_body_util::BodyExt;
use octo_core::settings::SettingsPatch;
use octo_core::{Config, Hub};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01";
const B: &str = "XBOUNDARYX";

type Bodies = Arc<Mutex<Vec<Value>>>;

/// Fake Ollama: `/api/show` reports `caps`; `/api/chat` records the body and answers "ok".
async fn fake_ollama(caps: &'static [&'static str]) -> (String, Bodies, Arc<AtomicUsize>) {
    let chats = Bodies::default();
    let shows = Arc::new(AtomicUsize::new(0));
    let s2 = shows.clone();
    let c2 = chats.clone();
    let app = Router::new()
        .route(
            "/api/show",
            post(move || {
                let shows = s2.clone();
                async move {
                    shows.fetch_add(1, Ordering::SeqCst);
                    json!({"capabilities": caps}).to_string()
                }
            }),
        )
        .route(
            "/api/chat",
            post(move |body: Bytes| {
                let chats = c2.clone();
                async move {
                    chats.lock().unwrap().push(serde_json::from_slice(&body).unwrap());
                    "{\"message\":{\"role\":\"assistant\",\"content\":\"ok\"},\"done\":false}\n{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n"
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), chats, shows)
}

fn app(ollama: &str) -> (Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let hub = Hub::new(Config::new(Some(tmp.path().to_path_buf())), "test-node").unwrap();
    hub.update_settings(SettingsPatch {
        ollama_url: Some(ollama.to_string()),
        ..Default::default()
    })
    .unwrap();
    (octo_server::app(Arc::new(hub), Default::default()), tmp)
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

async fn upload(app: &Router, space: &str, name: &str, data: &[u8]) {
    let mut body = format!(
        "--{B}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{B}--\r\n").as_bytes());
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/files/{space}?path="))
        .header("content-type", format!("multipart/form-data; boundary={B}"))
        .body(Body::from(body))
        .unwrap();
    let r = app.clone().oneshot(req).await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
}

async fn new_thread(app: &Router, model: &str) -> String {
    let r = call(
        app,
        "POST",
        "/api/threads",
        Some(json!({"provider": "ollama", "space": "private", "model": model})),
    )
    .await;
    let v: Value = serde_json::from_str(&text(r).await).unwrap();
    v["id"].as_str().unwrap().to_string()
}

async fn send(app: &Router, id: &str, path: &str) -> Response<Body> {
    call(
        app,
        "POST",
        &format!("/api/threads/{id}/messages"),
        Some(
            json!({"content": "what is this", "attachments": [{"space": "private", "path": path}]}),
        ),
    )
    .await
}

async fn no_messages_stored(app: &Router, id: &str) {
    let r = call(app, "GET", &format!("/api/threads/{id}/messages"), None).await;
    assert_eq!(text(r).await, "[]");
}

#[tokio::test]
async fn private_image_reaches_local_vision_model() {
    let (url, chats, _) = fake_ollama(&["completion", "vision"]).await;
    let (app, _d) = app(&url);
    upload(&app, "private", "photo.png", PNG).await;
    let id = new_thread(&app, "llava").await;
    let r = send(&app, &id, "photo.png").await;
    assert_eq!(r.status(), StatusCode::OK);
    let body = text(r).await;
    assert!(body.contains("event: done"), "{body}");
    let sent = chats.lock().unwrap()[0].clone();
    let b64 = base64::engine::general_purpose::STANDARD.encode(PNG);
    assert_eq!(sent["messages"][0]["images"], json!([b64]));
    assert_eq!(sent["messages"][0]["content"], "what is this");
}

#[tokio::test]
async fn image_for_a_model_without_vision_is_415() {
    let (url, chats, shows) = fake_ollama(&["completion"]).await;
    let (app, _d) = app(&url);
    upload(&app, "private", "photo.png", PNG).await;
    let id = new_thread(&app, "llama3").await;
    assert_eq!(
        send(&app, &id, "photo.png").await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        shows.load(Ordering::SeqCst),
        1,
        "rejected by the capability check"
    );
    assert!(chats.lock().unwrap().is_empty());
    no_messages_stored(&app, &id).await;
}

#[tokio::test]
async fn unknown_binary_and_scanned_pdf_are_415() {
    let (url, _, _) = fake_ollama(&["completion", "vision"]).await;
    let (app, _d) = app(&url);
    upload(&app, "private", "blob.dat", &[0xff, 0, 1, 2]).await;
    upload(&app, "private", "scan.pdf", &no_text_pdf()).await;
    let id = new_thread(&app, "llava").await;
    for p in ["blob.dat", "scan.pdf"] {
        assert_eq!(
            send(&app, &id, p).await.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{p}"
        );
    }
    no_messages_stored(&app, &id).await;
}

#[tokio::test]
async fn pdf_with_text_layer_is_accepted() {
    let (url, chats, _) = fake_ollama(&["completion"]).await;
    let (app, _d) = app(&url);
    upload(&app, "private", "lab.pdf", &text_pdf("Ferritin 40 ug/L")).await;
    let id = new_thread(&app, "llama3").await;
    let r = send(&app, &id, "lab.pdf").await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(text(r).await.contains("event: done"));
    let sent = chats.lock().unwrap()[0].clone();
    let prompt = sent["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains("Ferritin 40 ug/L"));
    assert!(!prompt.contains("%PDF"), "raw PDF syntax leaked");
}

fn no_text_pdf() -> Vec<u8> {
    pdf("")
}

fn text_pdf(line: &str) -> Vec<u8> {
    pdf(&format!("BT /F1 10 Tf 40 800 Td ({line}) Tj ET"))
}

fn pdf(content: &str) -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objs.len() + 1
        )
        .as_bytes(),
    );
    out
}
