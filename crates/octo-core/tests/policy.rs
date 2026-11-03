use octo_core::policy::{PolicyError, check_provider};
use octo_core::provider::ProviderKind;
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
