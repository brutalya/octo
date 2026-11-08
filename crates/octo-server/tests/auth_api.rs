use axum::body::Body;
use axum::http::{Request, StatusCode};
use octo_core::{Config, Hub};
use std::sync::Arc;
use tower::ServiceExt;

fn router(token: Option<&str>) -> (axum::Router, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::new(Some(tmp.path().to_path_buf()));
    let web = tmp.path().join("web");
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>octo</html>").unwrap();
    let app = octo_server::app_with_token(
        Arc::new(Hub::new(cfg, "test-node").unwrap()),
        Some(web),
        token.map(str::to_string),
    );
    (app, tmp)
}

async fn status(app: &axum::Router, uri: &str, auth: Option<&str>) -> StatusCode {
    let mut b = Request::builder().uri(uri);
    if let Some(a) = auth {
        b = b.header("authorization", a);
    }
    app.clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn api_requires_token_when_configured() {
    let (app, _d) = router(Some("s3cret"));
    assert_eq!(
        status(&app, "/api/providers", None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&app, "/api/providers", Some("Bearer wrong")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&app, "/api/providers", Some("s3cret")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&app, "/api/providers", Some("Bearer s3cret")).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn static_files_stay_open_with_token() {
    let (app, _d) = router(Some("s3cret"));
    assert_eq!(status(&app, "/", None).await, StatusCode::OK);
}

#[tokio::test]
async fn no_token_means_open() {
    let (app, _d) = router(None);
    assert_eq!(status(&app, "/api/providers", None).await, StatusCode::OK);
}
