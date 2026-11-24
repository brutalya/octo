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

/// Where this node's pairing stands; lets Settings show a running invite again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PairingStage {
    Idle,
    /// A code is out (or being created) and nobody has used it yet.
    Inviting,
    /// Connecting with a code from another device.
    Joining,
    /// Both sides see the numbers; waiting for Confirm / Numbers differ.
    Confirm,
    /// Decision taken; the exchange is being completed.
    Finishing,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PairingState {
    pub stage: PairingStage,
    /// The code this node handed out, while it is inviting or confirming that pairing.
    pub invite: Option<PairingInvite>,
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

/// This device's network as Settings shows it. A device is in a network once it has a relay
/// (it created one, or joined with a code) or members.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NetworkView {
    pub joined: bool,
    /// This device's label for the network: the saved name, else the relay host.
    pub name: Option<String>,
    pub relay_host: Option<String>,
    /// Devices in the network, this one included.
    pub devices: u32,
}

/// What the caller may do on this node: run CLI agents (the node's switch for the caller) and
/// use the private space (full trust). Asked through the tunnel; locally both are true.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AccessView {
    pub agents: bool,
    pub private_files: bool,
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
