use octo_core::policy::{PolicyError, check_provider};
use octo_core::providers::ProviderKind;
use octo_core::storage::Space;

#[test]
fn private_rejects_non_local() {
    assert_eq!(
        check_provider(Space::Private, ProviderKind::Claude),
        Err(PolicyError::PrivateNonLocal)
    );
    assert_eq!(
        check_provider(Space::Private, ProviderKind::Antigravity),
        Err(PolicyError::PrivateNonLocal)
    );
}

#[test]
fn public_allows_all() {
    assert!(check_provider(Space::Public, ProviderKind::Claude).is_ok());
    assert!(check_provider(Space::Public, ProviderKind::Antigravity).is_ok());
}

#[test]
fn ollama_is_the_only_local_provider() {
    assert!(ProviderKind::Ollama.is_local());
    assert!(!ProviderKind::OpenRouter.is_local());
    assert!(!ProviderKind::Claude.is_local());
    assert!(check_provider(Space::Private, ProviderKind::Ollama).is_ok());
    assert_eq!(
        check_provider(Space::Private, ProviderKind::OpenRouter),
        Err(PolicyError::PrivateNonLocal)
    );
    assert!(check_provider(Space::Public, ProviderKind::OpenRouter).is_ok());
    assert!(check_provider(Space::Public, ProviderKind::Ollama).is_ok());
}

#[test]
fn attachment_space_must_equal_thread_space() {
    use octo_core::policy::check_attachment;
    for (t, a, ok) in [
        (Space::Private, Space::Private, true),
        (Space::Public, Space::Public, true),
        (Space::Public, Space::Private, false),
        (Space::Private, Space::Public, false),
    ] {
        let r = check_attachment(t, a);
        assert_eq!(r.is_ok(), ok, "{t:?} {a:?}");
        if !ok {
            assert_eq!(r, Err(PolicyError::AttachmentSpace));
        }
    }
}

#[test]
fn ollama_is_local_only_for_loopback_or_private_hosts() {
    use octo_core::policy::{check_thread, is_local_url};
    for u in [
        "http://127.0.0.1:11434",
        "http://127.9.9.9:1",
        "http://localhost:11434",
        "http://LOCALHOST:1",
        "http://[::1]:11434",
        "http://10.1.2.3:1",
        "http://172.16.0.1:1",
        "http://172.31.255.255:1",
        "http://192.168.1.5:1",
        "http://169.254.1.1:1",
        "http://[fd00::1]:1",
        "http://[fe80::1]:1",
        "https://192.168.0.2",
    ] {
        assert!(is_local_url(u), "{u}");
        assert!(check_thread(Space::Private, ProviderKind::Ollama, u).is_ok());
    }
    for u in [
        "http://172.32.0.1:1",
        "http://172.15.0.1:1",
        "http://8.8.8.8:1",
        "http://example.com:11434",
        "http://localhost.evil.com:1",
        "http://127.0.0.1.evil.com:1",
        "http://[2001:db8::1]:1",
        "http://box:11434",
        "ftp://127.0.0.1",
        "nope",
        "",
    ] {
        assert!(!is_local_url(u), "{u}");
        assert_eq!(
            check_thread(Space::Private, ProviderKind::Ollama, u),
            Err(PolicyError::PrivateNonLocal),
            "{u}"
        );
        assert!(check_thread(Space::Public, ProviderKind::Ollama, u).is_ok());
    }
}
