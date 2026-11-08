//! Octo network layer: node identity, membership, pairing, relay, E2EE sessions, stream mux.
pub mod error;
pub mod identity;
pub mod membership;
pub mod mux;
pub mod pairing;
pub mod relay;
pub mod relay_client;
pub mod session;

pub use error::{NetError, Result};

/// Noise parameters of the `XX` pattern (relay link and pairing).
pub const XX_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";

/// Largest Noise message (and buffer) in bytes.
pub(crate) const NOISE_MAX: usize = 65_535;
