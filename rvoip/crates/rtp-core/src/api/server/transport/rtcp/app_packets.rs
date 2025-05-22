//! RTCP application packets functionality
//!
//! This module handles RTCP application-defined packets, BYE packets, and XR packets.

use crate::api::common::error::MediaTransportError;
use crate::api::client::transport::VoipMetrics;

/// Send RTCP APP packet to all clients
pub async fn send_rtcp_app(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_app")
}

/// Send RTCP APP packet to a specific client
pub async fn send_rtcp_app_to_client(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_app_to_client")
}

/// Send RTCP BYE packet to all clients
pub async fn send_rtcp_bye(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_bye")
}

/// Send RTCP BYE packet to a specific client
pub async fn send_rtcp_bye_to_client(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_bye_to_client")
}

/// Send RTCP XR VoIP metrics packet to all clients
pub async fn send_rtcp_xr_voip_metrics(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_xr_voip_metrics")
}

/// Send RTCP XR VoIP metrics packet to a specific client
pub async fn send_rtcp_xr_voip_metrics_to_client(
    // Parameters will be added during implementation
) -> Result<(), MediaTransportError> {
    // To be implemented during refactoring
    todo!("Implement send_rtcp_xr_voip_metrics_to_client")
} 