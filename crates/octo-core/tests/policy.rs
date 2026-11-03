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
