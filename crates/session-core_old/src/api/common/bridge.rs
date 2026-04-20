//! Call bridging utilities

use crate::api::types::SessionId;
use crate::api::bridge::{BridgeId, BridgeInfo};
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use std::sync::Arc;

/// Create a bridge between two calls
pub async fn create_bridge(
    coordinator: &Arc<SessionCoordinator>,
    session1: &SessionId,
    session2: &SessionId,
) -> Result<BridgeId> {
    coordinator.bridge_sessions(session1, session2).await
}

/// Destroy a bridge
pub async fn destroy_bridge(
    coordinator: &Arc<SessionCoordinator>,
    bridge_id: &BridgeId,
) -> Result<()> {
    coordinator.destroy_bridge(bridge_id).await
}

/// Get bridge information
pub async fn get_bridge_info(
    coordinator: &Arc<SessionCoordinator>,
    bridge_id: &BridgeId,
) -> Result<Option<BridgeInfo>> {
    coordinator.get_bridge_info(bridge_id).await
}