//! Event handling
//!
//! This module handles event subscription and callback management.

use crate::api::common::events::{MediaEventCallback, MediaTransportEvent};
use crate::api::common::error::MediaTransportError;
use crate::api::server::transport::ClientInfo;

/// Register a callback for media transport events
pub async fn on_event(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement on_event")
}

/// Register a callback for client connected events
pub async fn on_client_connected(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement on_client_connected")
}

/// Register a callback for client disconnected events
pub async fn on_client_disconnected(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement on_client_disconnected")
} 