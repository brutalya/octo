//! API types of the node / pairing endpoints (the logic lives in `octo-server`).
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeInfo {
    pub node_id: String,
    pub name: String,
    pub is_self: bool,
    pub online: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingInvite {
    pub code: String,
    /// Unix seconds.
    #[ts(type = "number")]
    pub expires_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingPending {
    pub sas: String,
    pub peer_name: String,
}
