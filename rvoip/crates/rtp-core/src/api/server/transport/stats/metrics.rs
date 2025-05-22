//! Server metrics functionality
//!
//! This module handles server-specific metrics and statistics.

use crate::api::common::error::MediaTransportError;
use crate::api::common::stats::{MediaStats, QualityLevel};

/// Get aggregate server statistics
pub async fn get_stats(
    // Parameters will be added during implementation
) -> Result<MediaStats, MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement get_stats")
}

/// Get statistics for a specific client
pub async fn get_client_stats(
    // Parameters will be added during implementation
) -> Result<MediaStats, MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement get_client_stats")
}

/// Get the frame type based on payload type
pub fn get_frame_type_from_payload_type(
    // Parameters will be added during implementation
) -> crate::api::common::frame::MediaFrameType {
    // To be implemented during refactoring
    todo!("Implement get_frame_type_from_payload_type")
} 