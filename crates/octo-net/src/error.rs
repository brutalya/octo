#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("noise: {0}")]
    Noise(#[from] snow::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("key file must be exactly 32 bytes")]
    BadKeyFile,
    #[error("malformed message")]
    Malformed,
    #[error("unsupported protocol version {0}")]
    Version(u8),
    #[error("peer is not a member of this network")]
    NotMember,
    #[error("no established session with peer")]
    NoSession,
    #[error("message too large")]
    TooLarge,
    #[error("not connected to relay")]
    Offline,
    #[error("replayed or stale handshake")]
    Replay,
    #[error("relay: {0}")]
    Relay(String),
    #[error("pairing: {0}")]
    Pairing(String),
    #[error("stream reset")]
    Reset,
    #[error("closed")]
    Closed,
    #[error("timed out")]
    Timeout,
}

pub type Result<T> = std::result::Result<T, NetError>;
