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
//! - **Backward Compatibility**: Support legacy session coordination patterns
//! - **Event Filtering**: Send only relevant events to avoid session-core overload

use tokio::sync::mpsc;
use crate::events::SessionCoordinationEvent;
use crate::errors::DialogResult;
use super::core::DialogManager;

/// Trait for session coordination operations
pub trait SessionCoordinator {
    /// Set up session coordination channel
    fn configure_session_coordination(
        &self,
        sender: mpsc::Sender<SessionCoordinationEvent>,
    ) -> impl std::future::Future<Output = ()> + Send;
    
    /// Send event to session-core
    fn notify_session_layer(
        &self,
        event: SessionCoordinationEvent,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for event sending operations
pub trait EventSender {
    /// Send session coordination event
    fn send_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

// Stub implementations
impl SessionCoordinator for DialogManager {
    async fn configure_session_coordination(&self, sender: mpsc::Sender<SessionCoordinationEvent>) {
        self.set_session_coordinator(sender).await;
    }
    
    async fn notify_session_layer(&self, event: SessionCoordinationEvent) -> DialogResult<()> {
        // Use the emit_session_coordination_event method which properly uses GlobalEventCoordinator
        self.emit_session_coordination_event(event).await;
        Ok(())
    }
}

impl EventSender for DialogManager {
    async fn send_coordination_event(&self, event: SessionCoordinationEvent) -> DialogResult<()> {
        self.notify_session_layer(event).await
    }
} 