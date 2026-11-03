use crate::providers::ProviderKind;
use crate::storage::Space;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("private threads may only use local providers")]
    PrivateNonLocal,
    #[error("attachment must come from the thread's space")]
    AttachmentSpace,
}

pub fn check_provider(space: Space, provider: ProviderKind) -> Result<(), PolicyError> {
    if space == Space::Private && !provider.is_local() {
        return Err(PolicyError::PrivateNonLocal);
    }
    Ok(())
}

pub fn check_attachment(thread_space: Space, attachment_space: Space) -> Result<(), PolicyError> {
    if thread_space != attachment_space {
        return Err(PolicyError::AttachmentSpace);
    }
    Ok(())
}
