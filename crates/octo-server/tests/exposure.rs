use axum::body::Body;
use axum::http::{Request, StatusCode};
use octo_core::{Config, Hub};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

async fn status(app: &axum::Router, path: &str) -> StatusCode {
    app.clone()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn api_router_has_no_relay() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::new(Some(tmp.path().to_path_buf()));
    let app = octo_server::app(Arc::new(Hub::new(cfg, "test-node").unwrap()), None);
    assert_eq!(status(&app, "/relay").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_healthz_is_open_even_with_token() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = Config::new(Some(tmp.path().to_path_buf()));
    let app = octo_server::app_with_token(
        Arc::new(Hub::new(cfg, "test-node").unwrap()),
        None,
        Some("t".into()),
    );
    assert_eq!(status(&app, "/healthz").await, StatusCode::OK);
    assert_eq!(status(&app, "/api/threads").await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn relay_router_serves_only_relay_and_healthz() {
    let relay = octo_net::relay::Relay::new(Default::default()).unwrap();
    let app = octo_server::relay_app(relay);
    assert_eq!(status(&app, "/healthz").await, StatusCode::OK);
    assert_eq!(status(&app, "/api/threads").await, StatusCode::NOT_FOUND);
    assert_eq!(status(&app, "/api/settings").await, StatusCode::NOT_FOUND);
    assert_eq!(status(&app, "/").await, StatusCode::NOT_FOUND);
    // /relay exists (plain GET without upgrade is rejected, but not 404)
    assert_ne!(status(&app, "/relay").await, StatusCode::NOT_FOUND);
}

#[test]
fn non_loopback_without_token_is_refused() {
    let wide: SocketAddr = "0.0.0.0:7878".parse().unwrap();
    let lo: SocketAddr = "127.0.0.1:7878".parse().unwrap();
    let lo6: SocketAddr = "[::1]:7878".parse().unwrap();
    let err = octo_server::check_exposure(wide, None).unwrap_err();
    assert!(err.contains("--token"), "{err}");
    assert!(octo_server::check_exposure(wide, Some("t")).is_ok());
    assert!(octo_server::check_exposure(lo, None).is_ok());
    assert!(octo_server::check_exposure(lo6, None).is_ok());
}
