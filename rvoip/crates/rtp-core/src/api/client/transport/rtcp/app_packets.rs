//! RTCP application packets functionality
//!
//! This module handles RTCP application-defined packets, including
//! APP packets, BYE packets, and XR (Extended Report) packets.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, warn, info};
use bytes;

use crate::api::common::error::MediaTransportError;
use crate::api::client::transport::VoipMetrics;
use crate::session::RtpSession;
use crate::transport::RtpTransport;
use crate::transport::UdpRtpTransport;

/// Send an RTCP Application-Defined (APP) packet
///
/// This function sends an RTCP APP packet with the specified name and application data.
/// APP packets are used for application-specific purposes and allow
/// custom data to be exchanged between endpoints.
pub async fn send_rtcp_app(
    session: &Arc<Mutex<RtpSession>>,
    transport: &Arc<Mutex<Option<Arc<UdpRtpTransport>>>>,
    remote_address: std::net::SocketAddr,
    is_connected: bool,
    name: &str,
    data: Vec<u8>,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_rtcp_app functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    // Validate name (must be exactly 4 ASCII characters)
    if name.len() != 4 || !name.chars().all(|c| c.is_ascii()) {
        return Err(MediaTransportError::ConfigError(
            "APP name must be exactly 4 ASCII characters".to_string()
        ));
    }
    
    // Get session for SSRC
    let session = session.lock().await;
    let ssrc = session.get_ssrc();
    
    // Create APP packet
    let mut app_packet = crate::RtcpApplicationDefined::new_with_name(ssrc, name)
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to create APP packet: {}", e)))?;
    
    // Set data - clone it before using
    let data_clone = data.clone();
    app_packet.set_data(bytes::Bytes::from(data));
    
    // Create RTCP packet
    let rtcp_packet = crate::RtcpPacket::ApplicationDefined(app_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize APP packet: {}", e)))?;
    
    // Get transport
    let transport_guard = transport.lock().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::NotConnected)?;
    
    // Send to remote address
    transport.send_rtcp_bytes(&rtcp_data, remote_address).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send APP packet: {}", e)))?;
    
    debug!("Sent RTCP APP packet: name={}, data_len={}", name, data_clone.len());
    
    Ok(())
}

/// Send an RTCP Goodbye (BYE) packet
///
/// This function sends an RTCP BYE packet with an optional reason for leaving.
/// BYE packets are used to indicate that a source is no longer active.
pub async fn send_rtcp_bye(
    session: &Arc<Mutex<RtpSession>>,
    transport: &Arc<Mutex<Option<Arc<UdpRtpTransport>>>>,
    remote_address: std::net::SocketAddr,
    is_connected: bool,
    reason: Option<String>,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_rtcp_bye functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    // Get session
    let session = session.lock().await;
    
    // Create BYE packet with our SSRC - clone reason before moving
    let reason_clone = reason.clone();
    let bye_packet = crate::RtcpGoodbye {
        sources: vec![session.get_ssrc()],
        reason,
    };
    
    // Create RTCP packet
    let rtcp_packet = crate::RtcpPacket::Goodbye(bye_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize BYE packet: {}", e)))?;
    
    // Get transport
    let transport_guard = transport.lock().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::NotConnected)?;
    
    // Send to remote address
    transport.send_rtcp_bytes(&rtcp_data, remote_address).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send BYE packet: {}", e)))?;
    
    debug!("Sent RTCP BYE packet: reason={:?}", reason_clone);
    
    Ok(())
}

/// Send an RTCP Extended Report (XR) packet with VoIP metrics
///
/// This function sends an RTCP XR packet with VoIP metrics for the specified SSRC.
/// XR packets are used to report extended statistics beyond what is
/// available in standard Sender/Receiver Reports.
pub async fn send_rtcp_xr_voip_metrics(
    session: &Arc<Mutex<RtpSession>>,
    transport: &Arc<Mutex<Option<Arc<UdpRtpTransport>>>>,
    remote_address: std::net::SocketAddr,
    is_connected: bool,
    metrics: VoipMetrics,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_rtcp_xr_voip_metrics functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    // Get session for SSRC
    let session = session.lock().await;
    let ssrc = session.get_ssrc();
    
    // Create XR packet
    let mut xr_packet = crate::RtcpExtendedReport::new(ssrc);
    
    // Convert our metrics to VoipMetricsBlock
    let voip_metrics_block = crate::VoipMetricsBlock {
        ssrc: metrics.ssrc,
        loss_rate: metrics.loss_rate,
        discard_rate: metrics.discard_rate,
        burst_density: metrics.burst_density,
        gap_density: metrics.gap_density,
        burst_duration: metrics.burst_duration,
        gap_duration: metrics.gap_duration,
        round_trip_delay: metrics.round_trip_delay,
        end_system_delay: metrics.end_system_delay,
        signal_level: metrics.signal_level as u8, // Convert i8 to u8
        noise_level: metrics.noise_level as u8,   // Convert i8 to u8
        rerl: metrics.rerl,
        r_factor: metrics.r_factor,
        ext_r_factor: 0, // Not used in our API
        mos_lq: metrics.mos_lq,
        mos_cq: metrics.mos_cq,
        rx_config: 0, // Default configuration
        jb_nominal: metrics.jb_nominal,
        jb_maximum: metrics.jb_maximum,
        jb_abs_max: metrics.jb_abs_max,
        gmin: 16, // Default value for minimum gap threshold
    };
    
    // Add the VoIP metrics block to the XR packet
    xr_packet.add_block(crate::RtcpXrBlock::VoipMetrics(voip_metrics_block));
    
    // Create RTCP packet
    let rtcp_packet = crate::RtcpPacket::ExtendedReport(xr_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize XR packet: {}", e)))?;
    
    // Get transport
    let transport_guard = transport.lock().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::NotConnected)?;
    
    // Send to remote address
    transport.send_rtcp_bytes(&rtcp_data, remote_address).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send XR packet: {}", e)))?;
    
    debug!("Sent RTCP XR VoIP metrics: loss_rate={}%, r_factor={}", 
           metrics.loss_rate, metrics.r_factor);
    
    Ok(())
} 