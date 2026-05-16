//! Optional B2BUA convenience: wires the canonical incoming-INVITE →
//! originate-outbound → bridge pattern entirely through
//! [`UnifiedCoordinator`].
//!
//! Per CARVE_PLAN §5: validates that `server::*` stands on its own — a
//! SIP-only consumer can use rvoip-sip without `rvoip-core` involvement by
//! composing api/ calls.
//!
//! ```rust,no_run
//! use rvoip_sip::server::b2bua::SipB2bua;
//! use rvoip_sip::SessionId;
//!
//! # async fn example(
//! #     coordinator: std::sync::Arc<rvoip_sip::UnifiedCoordinator>,
//! #     incoming_session_id: SessionId,
//! # ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let b2bua = SipB2bua::new(coordinator);
//! let _bridge = b2bua
//!     .handle_inbound("sip:gw@example.com", &incoming_session_id, "sip:bob@upstream.example.net")
//!     .await?;
//! # Ok(())
//! # }
//! ```

use crate::api::unified::{BridgeError, BridgeHandle, UnifiedCoordinator};
use crate::server::bridge::sip_bridge;
use crate::SessionId;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum B2buaError {
    #[error("session error: {0}")]
    Session(#[from] crate::errors::SessionError),
    #[error("bridge error: {0}")]
    Bridge(BridgeError),
}

impl From<BridgeError> for B2buaError {
    fn from(err: BridgeError) -> Self {
        B2buaError::Bridge(err)
    }
}

#[derive(Clone)]
pub struct SipB2bua {
    coordinator: Arc<UnifiedCoordinator>,
}

impl SipB2bua {
    pub fn new(coordinator: Arc<UnifiedCoordinator>) -> Self {
        Self { coordinator }
    }

    /// Accept the inbound INVITE on `incoming`, originate an outbound leg to
    /// `target_uri` from `from_uri`, then bridge the two. Returns the
    /// resulting [`BridgeHandle`] (drop to tear down).
    pub async fn handle_inbound(
        &self,
        from_uri: &str,
        incoming: &SessionId,
        target_uri: &str,
    ) -> Result<BridgeHandle, B2buaError> {
        self.coordinator.accept_call(incoming).await?;
        let outbound = self
            .coordinator
            .invite(Some(from_uri.to_string()), target_uri.to_string())
            .send()
            .await?;
        let handle = sip_bridge(&self.coordinator, incoming, &outbound).await?;
        Ok(handle)
    }
}
