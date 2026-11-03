use axum::Router;
use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use http_body_util::BodyExt;
use octo_core::{Config, Hub};
use serde_json::{Value, json};
use std::sync::Arc;
use tower::ServiceExt;

fn app() -> (Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::new(Some(tmp.path().to_path_buf()));
    (
        octo_server::app(Arc::new(Hub::new(cfg).unwrap()), None),
        tmp,
    )
}

async fn send(app: &Router, req: Request<Body>) -> Response<Body> {
    app.clone().oneshot(req).await.unwrap()
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
    send(app, b.body(body).unwrap()).await
}

async fn bytes(r: Response<Body>) -> Vec<u8> {
    r.into_body().collect().await.unwrap().to_bytes().to_vec()
}

async fn json_of(r: Response<Body>) -> Value {
    serde_json::from_slice(&bytes(r).await).unwrap()
}

const B: &str = "XBOUNDARYX";

fn multipart(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = vec![];
    for (name, data) in parts {
        out.extend_from_slice(
            format!("--{B}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
        );
        out.extend_from_slice(data);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{B}--\r\n").as_bytes());
    out
}

async fn upload(app: &Router, space: &str, dir: &str, parts: &[(&str, &[u8])]) -> Response<Body> {
    let uri = format!("/api/files/{space}?path={dir}");
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", format!("multipart/form-data; boundary={B}"))
        .body(Body::from(multipart(parts)))
        .unwrap();
    send(app, req).await
}

#[tokio::test]
async fn upload_list_download_rename_delete_in_both_spaces() {
    let (app, _d) = app();
    for space in ["public", "private"] {
        let r = upload(
            &app,
            space,
            "docs",
            &[("a.txt", b"alpha"), ("b.bin", &[0, 1, 2])],
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK, "{space}");
        let saved = json_of(r).await;
        assert_eq!(saved[0]["path"], "docs/a.txt");
        assert_eq!(saved[0]["kind"], "file");
        assert_eq!(saved[0]["size"], 5);

        let tree = json_of(call(&app, "GET", &format!("/api/files/{space}"), None).await).await;
        let paths: Vec<_> = tree
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["docs", "docs/a.txt", "docs/b.bin"], "{space}");
        assert_eq!(tree[0]["kind"], "dir");

        let r = call(
            &app,
            "GET",
            &format!("/api/files/{space}/content?path=docs/a.txt"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()["content-type"], "application/octet-stream");
        assert_eq!(r.headers()["x-content-type-options"], "nosniff");
        assert_eq!(bytes(r).await, b"alpha");

        let r = call(
            &app,
            "POST",
            &format!("/api/files/{space}/dir?path=empty/sub"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::CREATED);

        let r = call(
            &app,
            "PATCH",
            &format!("/api/files/{space}?path=docs/a.txt"),
            Some(json!({"new_path": "empty/sub/moved.txt"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = call(
            &app,
            "GET",
            &format!("/api/files/{space}/content?path=empty/sub/moved.txt"),
            None,
        )
        .await;
        assert_eq!(bytes(r).await, b"alpha");

        let r = call(
            &app,
            "DELETE",
            &format!("/api/files/{space}?path=empty"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::CONFLICT, "non-empty dir");
        let r = call(
            &app,
            "DELETE",
            &format!("/api/files/{space}?path=empty/sub/moved.txt"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = call(
            &app,
            "DELETE",
            &format!("/api/files/{space}?path=empty/sub"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NO_CONTENT);
        let r = call(
            &app,
            "GET",
            &format!("/api/files/{space}/content?path=empty/sub/moved.txt"),
            None,
        )
        .await;
        assert_eq!(r.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn spaces_are_isolated() {
    let (app, _d) = app();
    upload(&app, "private", "", &[("only-private.txt", b"x")]).await;
    let r = call(
        &app,
        "GET",
        "/api/files/public/content?path=only-private.txt",
        None,
    )
    .await;
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        json_of(call(&app, "GET", "/api/files/public", None).await).await,
        json!([])
    );
}

#[tokio::test]
async fn private_files_are_encrypted_on_disk_public_are_not() {
    let (app, d) = app();
    upload(&app, "private", "", &[("diary.txt", b"PRIVATE-PLAINTEXT")]).await;
    upload(&app, "public", "", &[("pub.txt", b"PUBLIC-PLAINTEXT")]).await;
    assert_eq!(
        std::fs::read(d.path().join("public/pub.txt")).unwrap(),
        b"PUBLIC-PLAINTEXT"
    );
    assert!(!d.path().join("private/diary.txt").exists());
    assert!(d.path().join("keys/private.key").exists());
    fn scan(p: &std::path::Path, needle: &[u8]) -> bool {
        std::fs::read_dir(p).unwrap().any(|e| {
            let p = e.unwrap().path();
            if p.is_dir() {
                scan(&p, needle)
            } else {
                std::fs::read(&p)
                    .unwrap()
                    .windows(needle.len())
                    .any(|w| w == needle)
            }
        })
    }
    assert!(!scan(&d.path().join("private"), b"PRIVATE-PLAINTEXT"));
    assert!(!scan(&d.path().join("private"), b"diary"));
}

#[tokio::test]
async fn path_traversal_attempts_are_rejected_over_http() {
    let (app, d) = app();
    std::fs::write(d.path().join("octo.txt"), b"outside").unwrap();
    for space in ["public", "private"] {
        for p in [
            "../octo.txt",
            "..%2Focto.txt",
            "%2e%2e/octo.txt",
            "a/../../octo.txt",
            "/etc/passwd",
            "%2Fetc%2Fpasswd",
            "a%5C..%5Cb",
        ] {
            let r = call(
                &app,
                "GET",
                &format!("/api/files/{space}/content?path={p}"),
                None,
            )
            .await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "GET {space} {p}");
            let r = call(
                &app,
                "POST",
                &format!("/api/files/{space}/dir?path={p}"),
                None,
            )
            .await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "mkdir {space} {p}");
            let r = call(
                &app,
                "DELETE",
                &format!("/api/files/{space}?path={p}"),
                None,
            )
            .await;
            assert_eq!(r.status(), StatusCode::BAD_REQUEST, "delete {space} {p}");
            let r = call(
                &app,
                "PATCH",
                &format!("/api/files/{space}?path={p}"),
                Some(json!({"new_path": "x"})),
            )
            .await;
            assert_eq!(
                r.status(),
                StatusCode::BAD_REQUEST,
                "rename-from {space} {p}"
            );
        }
        let r = call(
            &app,
            "PATCH",
            &format!("/api/files/{space}?path=nofile"),
            Some(json!({"new_path": "../x"})),
        )
        .await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        // traversal via the upload directory and via the file name
        let r = upload(&app, space, "../escape", &[("a.txt", b"x")]).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = upload(&app, space, "", &[("../escape.txt", b"x")]).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        let r = upload(&app, space, "", &[("/abs.txt", b"x")]).await;
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
    assert!(!d.path().join("escape.txt").exists());
    assert!(!d.path().parent().unwrap().join("escape").exists());
    assert_eq!(
        std::fs::read(d.path().join("octo.txt")).unwrap(),
        b"outside"
    );
}

#[tokio::test]
async fn upload_without_file_and_unknown_space() {
    let (app, _d) = app();
    let req = Request::builder()
        .method("POST")
        .uri("/api/files/public")
        .header("content-type", format!("multipart/form-data; boundary={B}"))
        .body(Body::from(format!("--{B}--\r\n")))
        .unwrap();
    assert_eq!(send(&app, req).await.status(), StatusCode::BAD_REQUEST);
    let r = call(&app, "GET", "/api/files/secret", None).await;
    assert!(r.status().is_client_error());
}

#[tokio::test]
async fn upload_over_50mb_is_rejected() {
    let (app, d) = app();
    let big = vec![b'a'; 51 * 1024 * 1024];
    let r = upload(&app, "public", "", &[("big.txt", &big)]).await;
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!d.path().join("public/big.txt").exists());
}

#[tokio::test]
async fn settings_get_patch_mask_and_validate() {
    let (app, d) = app();
    let v = json_of(call(&app, "GET", "/api/settings", None).await).await;
    assert_eq!(v["ollama_url"], "http://127.0.0.1:11434");
    assert_eq!(v["openrouter_api_key"], Value::Null);
    assert_eq!(v["history_char_budget"], 24000);

    let r = call(
        &app,
        "PATCH",
        "/api/settings",
        Some(json!({"openrouter_api_key": "sk-or-v1-SECRET-abcd"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::OK);
    let body = String::from_utf8(bytes(r).await).unwrap();
    assert!(
        body.contains("sk-...abcd") && !body.contains("SECRET"),
        "{body}"
    );
    let again =
        String::from_utf8(bytes(call(&app, "GET", "/api/settings", None).await).await).unwrap();
    assert!(again.contains("sk-...abcd") && !again.contains("SECRET"));
    assert!(
        std::fs::read_to_string(d.path().join("settings.json"))
            .unwrap()
            .contains("SECRET")
    );

    for bad in [
        json!({"ollama_url": "ftp://x"}),
        json!({"history_char_budget": 0}),
    ] {
        assert_eq!(
            call(&app, "PATCH", "/api/settings", Some(bad))
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let v = json_of(
        call(
            &app,
            "PATCH",
            "/api/settings",
            Some(json!({"openrouter_api_key": ""})),
        )
        .await,
    )
    .await;
    assert_eq!(v["openrouter_api_key"], Value::Null);
}

#[tokio::test]
async fn providers_include_http_kinds_and_models_errors_are_502() {
    let (app, _d) = app();
    // point Ollama at a closed port
    call(
        &app,
        "PATCH",
        "/api/settings",
        Some(json!({"ollama_url": "http://127.0.0.1:1"})),
    )
    .await;
    let v = json_of(call(&app, "GET", "/api/providers", None).await).await;
    let kinds: Vec<_> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["claude", "antigravity", "ollama", "openrouter"]);
    let r = call(&app, "GET", "/api/providers/ollama/models", None).await;
    assert_eq!(r.status(), StatusCode::BAD_GATEWAY);
    let r = call(&app, "GET", "/api/providers/claude/models", None).await;
    assert_eq!(json_of(r).await, json!([]));
}

#[tokio::test]
async fn policy_over_http_create_and_attach() {
    let (app, _d) = app();
    for (provider, status) in [
        ("openrouter", 403),
        ("claude", 403),
        ("antigravity", 403),
        ("ollama", 200),
    ] {
        let r = call(
            &app,
            "POST",
            "/api/threads",
            Some(json!({"provider": provider, "space": "private", "model": "m"})),
        )
        .await;
        assert_eq!(r.status().as_u16(), status, "{provider}");
    }
    let r = call(
        &app,
        "POST",
        "/api/threads",
        Some(json!({"provider": "ollama", "space": "public"})),
    )
    .await;
    assert_eq!(r.status(), StatusCode::BAD_REQUEST, "model required");
    let t = json_of(
        call(
            &app,
            "POST",
            "/api/threads",
            Some(json!({"provider": "ollama", "space": "public", "model": "llama3"})),
        )
        .await,
    )
    .await;
    assert_eq!(t["model"], "llama3");
    let id = t["id"].as_str().unwrap();

    upload(&app, "private", "", &[("p.txt", b"private")]).await;
    upload(&app, "public", "", &[("bin.dat", &[0xff, 0, 1])]).await;
    let post = |att: Value| {
        let app = app.clone();
        let id = id.to_string();
        async move {
            call(
                &app,
                "POST",
                &format!("/api/threads/{id}/messages"),
                Some(json!({"content": "hi", "attachments": att})),
            )
            .await
            .status()
        }
    };
    assert_eq!(
        post(json!([{"space": "private", "path": "p.txt"}])).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(json!([{"space": "public", "path": "bin.dat"}])).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        post(json!([{"space": "public", "path": "missing.txt"}])).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post(json!([{"space": "public", "path": "../x"}])).await,
        StatusCode::BAD_REQUEST
    );
    let msgs = json_of(call(&app, "GET", &format!("/api/threads/{id}/messages"), None).await).await;
    assert_eq!(msgs, json!([]));
}
