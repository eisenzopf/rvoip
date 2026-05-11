//! SIP↔SIP same-codec fast-path bridge strategy.
//!
//! Per CARVE_PLAN §3 (BridgeManager row): when both legs are SIP and codecs
//! match, the cross-transport `Orchestrator` dispatches to this fast path
//! instead of running the generic frame-pump (INTERFACE_DESIGN §10.2). The
//! fast path simply calls `api::UnifiedCoordinator::bridge(a, b)`, which
//! already does the right thing — RTP-direct relay through `media-core`.
//!
//! This module wraps no media-core types; it returns the
//! `media-core::BridgeHandle` that `bridge()` already returns. Bridge
//! registry tracking is the caller's responsibility (typically
//! `rvoip-core::BridgeManager`).

use crate::api::unified::{BridgeError, BridgeHandle, UnifiedCoordinator};
use crate::SessionId;
use std::sync::Arc;

/// Run the SIP-fast-path bridge between two existing SIP sessions.
///
/// Returns the [`BridgeHandle`] (media-core type, re-exported through
/// `rvoip-sip`'s api). Drop the handle to tear the bridge down.
pub async fn sip_bridge(
    coordinator: &UnifiedCoordinator,
    session_a: &SessionId,
    session_b: &SessionId,
) -> Result<BridgeHandle, BridgeError> {
    coordinator.bridge(session_a, session_b).await
}

/// Strategy object the cross-transport `Orchestrator` (in step 9+) registers
/// for SIP↔SIP bridge dispatch. Provided as a struct (not just a function)
/// so future configuration knobs (e.g. per-tenant bridge limits, codec
/// preference policies) can attach without changing the call signature.
#[derive(Clone)]
pub struct SipBridgeStrategy {
    coordinator: Arc<UnifiedCoordinator>,
}

impl SipBridgeStrategy {
    pub fn new(coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self { coordinator }
    }

    pub async fn bridge(
        &self,
        a: &SessionId,
        b: &SessionId,
    ) -> Result<BridgeHandle, BridgeError> {
        sip_bridge(&self.coordinator, a, b).await
    }
}
