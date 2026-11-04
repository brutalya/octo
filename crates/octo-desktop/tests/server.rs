use octo_core::Config;
use octo_desktop::start_server;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn get(addr: std::net::SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = String::new();
    s.read_to_string(&mut buf).await.unwrap();
    let status = buf.split(' ').nth(1).unwrap().parse().unwrap();
    (status, buf)
}

#[tokio::test]
async fn embedded_server_serves_ui_and_guards_api() {
    let tmp = tempfile::tempdir().unwrap();
    let web = tmp.path().join("web");
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), "<html>octo-ui</html>").unwrap();
    let cfg = Config::new(Some(tmp.path().join("data")));
    let server = start_server(cfg, Some(web)).await.unwrap();

    assert_ne!(server.addr.port(), 0);
    assert!(server.addr.ip().is_loopback());
    assert_eq!(server.token.len(), 64);

    let (st, body) = get(server.addr, "/", None).await;
    assert_eq!(st, 200);
    assert!(body.contains("octo-ui"));
    assert_eq!(get(server.addr, "/api/providers", None).await.0, 401);
    assert_eq!(
        get(server.addr, "/api/providers", Some("nope")).await.0,
        401
    );
    let tok = server.token.clone();
    assert_eq!(get(server.addr, "/api/providers", Some(&tok)).await.0, 200);
    server.shutdown().await;
}

#[test]
fn tokens_are_unique_and_injected() {
    assert_ne!(octo_desktop::new_token(), octo_desktop::new_token());
    assert!(octo_desktop::init_script("abc").contains("__OCTO_TOKEN__ = \"abc\""));
}
