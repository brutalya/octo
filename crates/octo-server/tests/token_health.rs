#![cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn token_file_is_created_0600_and_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("sub").join("token");
    let t1 = octo_server::load_or_create_token(&p).unwrap();
    assert!(t1.len() >= 43 && t1.chars().all(|c| c.is_ascii_alphanumeric()));
    let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let t2 = octo_server::load_or_create_token(&p).unwrap();
    assert_eq!(t1, t2);
}

#[test]
fn token_file_trims_and_rejects_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("token");
    std::fs::write(&p, "abc123\n").unwrap();
    assert_eq!(octo_server::load_or_create_token(&p).unwrap(), "abc123");
    std::fs::write(&p, "\n").unwrap();
    assert!(octo_server::load_or_create_token(&p).is_err());
}

#[tokio::test]
async fn healthcheck_ok_against_relay_and_fails_when_down() {
    let relay = octo_net::relay::Relay::new(Default::default()).unwrap();
    let app = octo_server::relay_app(relay);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let a = addr.to_string();
    tokio::task::spawn_blocking(move || octo_server::healthcheck(&a))
        .await
        .unwrap()
        .unwrap();
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    };
    assert!(octo_server::healthcheck(&dead).is_err());
}

#[test]
fn blank_tokens_are_rejected() {
    for t in ["", " ", "\n\t"] {
        assert!(octo_server::check_token(t).is_err(), "{t:?}");
    }
    assert!(octo_server::check_token("x").is_ok());
}

#[test]
fn concurrent_token_creation_agrees_on_one_complete_token() {
    let tmp = tempfile::tempdir().unwrap();
    let p = std::sync::Arc::new(tmp.path().join("token"));
    let hs: Vec<_> = (0..8)
        .map(|_| {
            let p = p.clone();
            std::thread::spawn(move || octo_server::load_or_create_token(&p).unwrap())
        })
        .collect();
    let tokens: Vec<String> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(
        tokens.iter().all(|t| t == &tokens[0] && t.len() == 48),
        "{tokens:?}"
    );
    assert_eq!(std::fs::read_to_string(&*p).unwrap(), tokens[0]);
    // No temp files left behind.
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn healthcheck_works_on_a_private_relay() {
    let relay =
        octo_net::relay::Relay::with_secret(Default::default(), Some("s3cret".into())).unwrap();
    let app = octo_server::relay_app(relay);
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let a = l.local_addr().unwrap().to_string();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    tokio::task::spawn_blocking(move || octo_server::healthcheck(&a))
        .await
        .unwrap()
        .unwrap();
}
