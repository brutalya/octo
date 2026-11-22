//! API types of the node / pairing endpoints (the logic lives in `octo-server`).
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Trust a node grants a member; `full` members may use its private space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TrustLevel {
    Full,
    Public,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NodeInfo {
    pub node_id: String,
    pub name: String,
    pub is_self: bool,
    pub online: bool,
    /// Trust this node grants the member; `null` for the node itself.
    pub trust: Option<TrustLevel>,
    /// Whether this node lets the member run CLI agents here; `null` for the node itself.
    pub agents: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingInvite {
    pub code: String,
    /// Unix seconds.
    #[ts(type = "number")]
    pub expires_at: u64,
}

/// This node's link to its relay. `active`: a link is kept open (only when there are members).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelayStatus {
    pub configured: bool,
    pub active: bool,
    pub online: bool,
    pub error: Option<String>,
}

/// Result of a one-off connection attempt to a relay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RelayCheck {
    pub ok: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingPending {
    pub sas: String,
    pub peer_name: String,
}
