//! Session Coordination for Dialog Management
//!
//! This module handles coordination between dialog-core and session-core layers,
//! maintaining proper architectural separation while enabling high-level session
//! management operations.
//!
//! ## Architecture Pattern
//!
//! ```text
//! session-core (High-level call/session logic)
//!      ↓ SessionCoordinationEvent
//! dialog-core (SIP protocol operations)  ← THIS MODULE
//!      ↓ TransactionKey
//! transaction-core (SIP reliability)
//! ```
//!
//! ## Key Responsibilities
//!
//! - **Event Emission**: Send session coordination events to session-core
//! - **Event Translation**: Convert dialog events to session events
//! - **Layer Boundary**: Maintain clean separation between protocol and session logic
//! - **Event Filtering**: Send only relevant events to avoid session-core overload

use super::core::DialogManager;
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;

/// Trait for session coordination operations
pub trait SessionCoordinator {
    /// Send event to session-core
    fn notify_session_layer(
        &self,
        event: SessionCoordinationEvent,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

// Implementation using GlobalEventCoordinator
impl SessionCoordinator for DialogManager {
    async fn notify_session_layer(&self, event: SessionCoordinationEvent) -> DialogResult<()> {
        if let Some(hub) = self.event_hub.read().await.clone() {
            let acknowledged = hub
                .publish_session_coordination_authoritative(event)
                .await
                .map_err(|_| {
                    DialogError::routing_error(
                        "authoritative session dispatch was rejected by session-core",
                    )
                })?;
            return acknowledged.then_some(()).ok_or_else(|| {
                DialogError::routing_error(
                    "authoritative session dispatch had no session-core handler",
                )
            });
        }

        // Focused manager tests inject transaction events directly without
        // constructing the cross-crate event hub. Keep their private channel
        // as a test-only causal sink; production never regains a legacy
        // fallback when the authoritative route is absent or rejects delivery.
        #[cfg(test)]
        if let Some(sender) = self.session_coordinator.read().await.clone() {
            return sender.send(event).await.map_err(|_| {
                DialogError::routing_error("test session coordination route was closed")
            });
        }

        Err(DialogError::routing_error(
            "authoritative session dispatch requires an event hub",
        ))
    }
}
