//! `CrossBridgeHandle` — the cross-transport sibling of the SIP-fast-path
//! `BridgeHandle` re-exported in `super`.
//!
//! Owns the abort handles for the two frame-pump tasks that copy media
//! between the bridged Connections. `Drop` aborts both pumps so an
//! `unbridge_connections` call (or the Orchestrator going away) tears
//! the bridge down promptly.

use chrono::{DateTime, Utc};
use tokio::task::AbortHandle;

use crate::ids::{BridgeId, ConnectionId};

pub struct CrossBridgeHandle {
    pub id: BridgeId,
    pub a: ConnectionId,
    pub b: ConnectionId,
    pub created_at: DateTime<Utc>,
    a_to_b: AbortHandle,
    b_to_a: AbortHandle,
}

impl CrossBridgeHandle {
    pub fn new(
        id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        a_to_b: AbortHandle,
        b_to_a: AbortHandle,
    ) -> Self {
        Self {
            id,
            a,
            b,
            created_at: Utc::now(),
            a_to_b,
            b_to_a,
        }
    }
}

impl Drop for CrossBridgeHandle {
    fn drop(&mut self) {
        self.a_to_b.abort();
        self.b_to_a.abort();
    }
}
