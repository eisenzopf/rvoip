//! Server metrics functionality
//!
//! This module handles server-specific media metrics.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::debug;

use crate::api::common::error::MediaTransportError;
use crate::api::common::stats::{MediaStats, QualityLevel};
use crate::api::server::transport::core::connection::ClientConnection;

/// Server Metrics Structure
#[derive(Debug, Default, Clone)]
pub struct ServerMetrics {
    /// Number of active client connections
    pub active_clients: usize,
    
    /// Total bytes received across all clients
    pub total_bytes_received: u64,
    
    /// Total bytes sent across all clients
    pub total_bytes_sent: u64,
    
    /// Total packets received across all clients
    pub total_packets_received: u64,
    
    /// Total packets sent across all clients
    pub total_packets_sent: u64,
    
    /// Average jitter across all clients (ms)
    pub average_jitter_ms: f32,
    
    /// Average packet loss across all clients (percentage)
    pub average_packet_loss: f32,
    
    /// Total aggregate downstream bandwidth (bps)
    pub total_downstream_bandwidth_bps: u64,
    
    /// Total aggregate upstream bandwidth (bps)
    pub total_upstream_bandwidth_bps: u64,
    
    /// Average round trip time (ms)
    pub average_rtt_ms: Option<f32>,
    
    /// Overall quality level
    pub overall_quality: QualityLevel,
    
    /// Total RTCP packets received
    pub total_rtcp_packets_received: u64,
    
    /// Total RTCP packets sent
    pub total_rtcp_packets_sent: u64,
    
    /// Time since server started
    pub uptime: Duration,
}

/// Calculate Mean Opinion Score (MOS) based on R-factor
/// 
/// This implements the ITU-T G.107 E-model for calculating MOS from R-factor.
pub fn calculate_mos_from_rfactor(r_factor: f32) -> f32 {
    if r_factor < 0.0 {
        return 1.0;
    } else if r_factor > 100.0 {
        return 4.5;
    }
    
    // MOS calculation according to ITU-T G.107
    if r_factor < 0.0 {
        1.0
    } else if r_factor < 6.52 {
        1.0
    } else if r_factor < 100.0 {
        1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor)
    } else {
        4.5
    }
}

/// Calculate R-factor from network metrics
pub fn calculate_rfactor(
    packet_loss_percent: f32,
    jitter_ms: f32,
    rtt_ms: f32,
) -> f32 {
    // Base R-factor for G.711 is 93.2
    let r0 = 93.2;
    
    // Impairment due to packet loss (according to simplified E-model)
    let is = if packet_loss_percent <= 0.0 {
        0.0
    } else {
        // Non-linear effect of packet loss
        2.0 + 14.0 * (1.0 - (1.0 - packet_loss_percent / 100.0).powf(30.0))
    };
    
    // Impairment due to jitter (simplified model)
    let ij = if jitter_ms < 1.0 {
        0.0
    } else {
        0.8 + 0.5 * jitter_ms.sqrt()
    };
    
    // Impairment due to delay (simplified model)
    let id = if rtt_ms < 100.0 {
        0.0
    } else if rtt_ms < 300.0 {
        (rtt_ms - 100.0) / 20.0
    } else {
        10.0 + (rtt_ms - 300.0) / 10.0
    };
    
    // Total R-factor (capped between 0 and 100)
    let r = r0 - is - id - ij;
    
    r.max(0.0).min(100.0)
}

/// Get server metrics
pub async fn get_server_metrics(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    media_stats: &MediaStats,
    server_start_time: Duration,
) -> Result<ServerMetrics, MediaTransportError> {
    let clients_guard = clients.read().await;
    
    let mut metrics = ServerMetrics::default();
    
    // Set active clients count
    metrics.active_clients = clients_guard.values().filter(|c| c.connected).count();
    
    // Calculate aggregates and averages
    let mut total_jitter = 0.0;
    let mut total_packet_loss = 0.0;
    let mut total_rtt = 0.0;
    let mut rtt_count = 0;
    
    // Aggregate values from all streams in the media stats
    for stream in media_stats.streams.values() {
        // Sum up bytes and packets
        metrics.total_bytes_received += stream.byte_count;
        metrics.total_packets_received += stream.packet_count;
        
        // Jitter average
        total_jitter += stream.jitter_ms;
        
        // Packet loss average
        total_packet_loss += stream.fraction_lost;
        
        // RTT average (if available)
        if let Some(rtt) = stream.rtt_ms {
            total_rtt += rtt;
            rtt_count += 1;
        }
    }
    
    // Calculate averages
    let stream_count = media_stats.streams.len();
    if stream_count > 0 {
        metrics.average_jitter_ms = total_jitter / stream_count as f32;
        metrics.average_packet_loss = total_packet_loss / stream_count as f32 * 100.0; // Convert to percentage
    }
    
    if rtt_count > 0 {
        metrics.average_rtt_ms = Some(total_rtt / rtt_count as f32);
    }
    
    // Set bandwidth metrics
    metrics.total_downstream_bandwidth_bps = media_stats.downstream_bandwidth_bps as u64;
    metrics.total_upstream_bandwidth_bps = media_stats.upstream_bandwidth_bps as u64;
    
    // Set quality level
    metrics.overall_quality = media_stats.quality;
    
    // Set RTCP metrics (would be populated from some RTCP stats source)
    // For now we'll leave them as default (0)
    
    // Set server uptime
    metrics.uptime = server_start_time;
    
    Ok(metrics)
}

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