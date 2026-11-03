use crate::provider::ProviderKind;
use crate::storage::Space;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("private threads may only use local providers")]
    PrivateNonLocal,
}

pub fn check_provider(space: Space, provider: ProviderKind) -> Result<(), PolicyError> {
    if space == Space::Private && !provider.is_local() {
        return Err(PolicyError::PrivateNonLocal);
    }
    Ok(())
}
