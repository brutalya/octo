use crate::policy::PolicyError;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("not found")]
    NotFound,
    #[error("a run is already in progress for this thread")]
    Busy,
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error("storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
