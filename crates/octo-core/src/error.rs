use crate::files::FilesError;
use crate::policy::PolicyError;
use crate::settings::SettingsError;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("not found")]
    NotFound,
    #[error("a run is already in progress for this thread")]
    Busy,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("internal error: {0}")]
    Internal(&'static str),
    #[error("{0}")]
    UnsupportedMedia(String),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Files(#[from] FilesError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("storage: {0}")]
    Storage(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
