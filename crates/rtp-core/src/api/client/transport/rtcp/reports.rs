//! RTCP reports functionality
//!
//! This module handles RTCP (RTP Control Protocol) reports, including
//! sender reports, receiver reports, and statistics gathering.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, warn, info};

use crate::api::common::error::MediaTransportError;
use crate::api::client::transport::RtcpStats;
use crate::session::RtpSession;

/// Send an RTCP Receiver Report
///
/// This function sends a Receiver Report RTCP packet to the remote peer. This can be
/// useful to force an immediate quality report instead of waiting for the
/// automatic interval-based reports.
pub async fn send_rtcp_receiver_report(
    session: &Arc<Mutex<RtpSession>>,
    is_connected: bool,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_rtcp_receiver_report functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    // Get the session and send the receiver report
    let mut session = session.lock().await;
    session.send_receiver_report().await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP receiver report: {}", e)))
}

/// Send an RTCP Sender Report
///
/// This function sends a Sender Report RTCP packet to the remote peer. This can be
/// useful to force an immediate quality report instead of waiting for the
/// automatic interval-based reports.
pub async fn send_rtcp_sender_report(
    session: &Arc<Mutex<RtpSession>>,
    is_connected: bool,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted send_rtcp_sender_report functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    // Get the session and send the sender report
    let mut session = session.lock().await;
    session.send_sender_report().await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP sender report: {}", e)))
}

/// Get detailed RTCP statistics
///
/// This function returns detailed quality metrics gathered from RTCP reports
/// including jitter, packet loss, and round-trip time.
pub async fn get_rtcp_stats(
    session: &Arc<Mutex<RtpSession>>,
    is_connected: bool,
) -> Result<RtcpStats, MediaTransportError> {
    // Placeholder for the extracted get_rtcp_stats functionality
    // Check if connected
    if !is_connected {
        return Err(MediaTransportError::NotConnected);
    }
    
    let session = session.lock().await;
    let rtp_stats = session.get_stats();
    
    // Get stream stats if available
    let mut stream_stats = None;
    let ssrcs = session.get_all_ssrcs().await;
    if !ssrcs.is_empty() {
        // Just use the first SSRC for now
        stream_stats = session.get_stream(ssrcs[0]).await;
    }
    
    // Create RTCP stats from the available information
    let mut rtcp_stats = RtcpStats::default();
    
    // Set basic stats
    rtcp_stats.jitter_ms = rtp_stats.jitter_ms;
    if rtp_stats.packets_received > 0 {
        rtcp_stats.packet_loss_percent = (rtp_stats.packets_lost as f64 / rtp_stats.packets_received as f64) * 100.0;
    }
    
    // If we have stream stats, use them to enhance the RTCP stats
    if let Some(stream) = stream_stats {
        rtcp_stats.cumulative_packets_lost = stream.packets_lost as u32;
        // Note: RTT is not available directly, would need to be calculated from RTCP reports
    }
    
    Ok(rtcp_stats)
}

/// Set the RTCP report interval
///
/// This function sets how frequently RTCP reports are sent. The default is usually
/// 5% of the session bandwidth, but this can be adjusted for more or less
/// frequent reporting.
pub async fn set_rtcp_interval(
    session: &Arc<Mutex<RtpSession>>,
    interval: Duration,
) -> Result<(), MediaTransportError> {
    // Placeholder for the extracted set_rtcp_interval functionality
    let mut session = session.lock().await;
    
    // The bandwidth calculation follows from RFC 3550 where RTCP bandwidth is typically 
    // 5% of session bandwidth. If we want a specific interval, we need to set the
    // bandwidth accordingly: bandwidth = packet_size * 8 / interval_fraction
    // where interval_fraction is 0.05 for 5%
    
    // Assuming average RTCP packet is around 100 bytes, calculate bandwidth
    let bytes_per_second = 100.0 / interval.as_secs_f64();
    let bits_per_second = bytes_per_second * 8.0 / 0.05; // 5% of bandwidth for RTCP
    
    // Set bandwidth on the session
    session.set_bandwidth(bits_per_second as u32);
    
    Ok(())
} 