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
    let server = start_server(cfg, Some(octo_server::WebAssets::Dir(web)))
        .await
        .unwrap();

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
    // The node service runs here too (pairing, sync and the tunnel need it).
    assert_eq!(get(server.addr, "/api/nodes", Some(&tok)).await.0, 200);
    server.shutdown().await;
}

#[test]
fn tokens_are_unique_and_injected() {
    assert_ne!(octo_desktop::new_token(), octo_desktop::new_token());
    let js = octo_desktop::init_script("abc", "http://127.0.0.1:4321");
    assert!(js.contains("__OCTO_TOKEN__ = \"abc\""));
    assert!(js.contains("location.origin === \"http://127.0.0.1:4321\""));
}

#[test]
fn navigation_is_limited_to_the_embedded_server() {
    let addr: std::net::SocketAddr = "127.0.0.1:4321".parse().unwrap();
    let ok = |u: &str| octo_desktop::is_app_url(&u.parse().unwrap(), addr);
    assert!(ok("http://127.0.0.1:4321/"));
    assert!(ok("http://127.0.0.1:4321/some/route?x=1#y"));
    for bad in [
        "http://127.0.0.1:4322/",
        "https://127.0.0.1:4321/",
        "http://localhost:4321/",
        "http://127.0.0.1.evil.example:4321/",
        "http://evil.example/",
        "http://127.0.0.1@evil.example:4321/",
        "file:///etc/passwd",
        "javascript:alert(1)",
    ] {
        assert!(!ok(bad), "{bad}");
    }
}

/// A failed start names the step, so a phone's logcat says what was refused.
#[tokio::test]
async fn start_errors_name_the_failing_step() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("keys"), b"not a dir").unwrap();
    let Err(e) = start_server(Config::new(Some(data.clone())), None).await else {
        panic!("start must fail when keys/ is a file");
    };
    let msg = format!("{e:#}");
    assert!(msg.starts_with("node key"), "{msg}");

    std::fs::remove_file(data.join("keys")).unwrap();
    std::fs::create_dir_all(data.join("octo.db")).unwrap();
    let Err(e) = start_server(Config::new(Some(data)), None).await else {
        panic!("start must fail when octo.db is a directory");
    };
    assert!(format!("{e:#}").starts_with("open data"), "{e:#}");
}
