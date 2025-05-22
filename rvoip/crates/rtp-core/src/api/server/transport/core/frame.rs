//! Frame processing
//!
//! This module handles frame sending, receiving, and broadcasting functionality.

use std::net::SocketAddr;
use std::sync::Arc;
use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::session::RtpSession;
use tokio::sync::Mutex;

/// Send a media frame to a specific client
pub async fn send_frame_to(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_frame_to")
}

/// Broadcast a media frame to all connected clients
pub async fn broadcast_frame(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement broadcast_frame")
}

/// Receive a media frame from any client
pub async fn receive_frame(
    // Parameters will be added during implementation
) -> Result<(String, MediaFrame), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement receive_frame")
} 