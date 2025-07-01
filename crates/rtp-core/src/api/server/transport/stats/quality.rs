//! Quality estimation functionality
//!
//! This module handles the estimation of media quality levels.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};

use crate::api::common::error::MediaTransportError;
use crate::api::common::stats::{MediaStats, QualityLevel, StreamStats};
use crate::api::server::transport::core::connection::ClientConnection;

/// Estimate quality level based on media statistics
pub fn estimate_quality_level(media_stats: &MediaStats) -> QualityLevel {
    // Simple estimation based on jitter, packet loss, and available bandwidth
    
    // Calculate an average quality level based on all streams
    if media_stats.streams.is_empty() {
        return QualityLevel::Unknown;
    }
    
    let mut quality_sum = 0;
    let mut stream_count = 0;
    
    for stream in media_stats.streams.values() {
        // Convert the numeric indicators to a quality level
        let quality = calculate_stream_quality(stream);
        
        // Convert quality level to numeric value for averaging
        let quality_value = match quality {
            QualityLevel::Excellent => 5,
            QualityLevel::Good => 4,
            QualityLevel::Fair => 3,
            QualityLevel::Poor => 2,
            QualityLevel::Bad => 1,
            QualityLevel::Unknown => 0,
        };
        
        if quality_value > 0 {
            quality_sum += quality_value;
            stream_count += 1;
        }
    }
    
    // If no streams had enough data for quality estimation
    if stream_count == 0 {
        return QualityLevel::Unknown;
    }
    
    // Calculate average quality and map back to enum
    let avg_quality = quality_sum as f32 / stream_count as f32;
    
    match avg_quality {
        x if x >= 4.5 => QualityLevel::Excellent,
        x if x >= 3.5 => QualityLevel::Good,
        x if x >= 2.5 => QualityLevel::Fair,
        x if x >= 1.5 => QualityLevel::Poor,
        _ => QualityLevel::Bad,
    }
}

/// Calculate quality level for a single stream
fn calculate_stream_quality(stream: &StreamStats) -> QualityLevel {
    // Not enough data for quality estimation
    if stream.packet_count < 10 {
        return QualityLevel::Unknown;
    }
    
    // Packet loss percentage (0.0 to 1.0)
    let packet_loss = stream.fraction_lost;
    
    // Jitter in milliseconds
    let jitter_ms = stream.jitter_ms;
    
    // Round trip time in milliseconds (if available)
    let rtt_ms = stream.rtt_ms.unwrap_or(0.0);
    
    // Assign scores for each metric (0-100, higher is better)
    
    // Packet loss score (100 = no loss, 0 = 20% loss or more)
    let packet_loss_score = if packet_loss <= 0.001 { // Less than 0.1% loss
        100.0
    } else if packet_loss <= 0.005 { // Less than 0.5% loss
        95.0
    } else if packet_loss <= 0.01 { // Less than 1% loss
        90.0
    } else if packet_loss <= 0.03 { // Less than 3% loss
        70.0
    } else if packet_loss <= 0.05 { // Less than 5% loss
        50.0
    } else if packet_loss <= 0.10 { // Less than 10% loss
        30.0
    } else if packet_loss <= 0.20 { // Less than 20% loss
        10.0
    } else { // 20% or more loss
        0.0
    };
    
    // Jitter score (100 = very low jitter, 0 = high jitter)
    let jitter_score = if jitter_ms <= 1.0 {
        100.0
    } else if jitter_ms <= 5.0 {
        90.0
    } else if jitter_ms <= 10.0 {
        80.0
    } else if jitter_ms <= 20.0 {
        60.0
    } else if jitter_ms <= 30.0 {
        40.0
    } else if jitter_ms <= 50.0 {
        20.0
    } else {
        0.0
    };
    
    // RTT score (100 = very low RTT, 0 = high RTT)
    let rtt_score = if rtt_ms == 0.0 { // Unknown RTT
        50.0 // Neutral score
    } else if rtt_ms <= 20.0 {
        100.0
    } else if rtt_ms <= 50.0 {
        90.0
    } else if rtt_ms <= 100.0 {
        80.0
    } else if rtt_ms <= 150.0 {
        60.0
    } else if rtt_ms <= 200.0 {
        40.0
    } else if rtt_ms <= 300.0 {
        20.0
    } else {
        0.0
    };
    
    // Calculate overall quality score with weights:
    // - Packet loss is most important (weight 50%)
    // - Jitter is second (weight 30%)
    // - RTT is third (weight 20%)
    let quality_score = packet_loss_score * 0.5 + jitter_score * 0.3 + rtt_score * 0.2;
    
    // Map score to quality level
    match quality_score {
        x if x >= 90.0 => QualityLevel::Excellent,
        x if x >= 75.0 => QualityLevel::Good,
        x if x >= 60.0 => QualityLevel::Fair,
        x if x >= 40.0 => QualityLevel::Poor,
        _ => QualityLevel::Bad,
    }
}

/// Get statistics for all clients
pub async fn get_stats(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<MediaStats, MediaTransportError> {
    // Aggregate stats from all clients
    let clients_guard = clients.read().await;
    
    let mut agg_stats = MediaStats::default();
    
    // Set the session duration
    if let Some(client) = clients_guard.values().next() {
        // Simply use the system time as session duration - better than nothing
        agg_stats.session_duration = std::time::Duration::from_secs(0);
    }
    
    // Create stream entries for each client's statistics
    for client in clients_guard.values() {
        if !client.connected {
            continue;
        }
        
        let session = client.session.lock().await;
        let rtp_stats = session.get_stats();
        
        // Create a stream entry
        let mut stream_stats = StreamStats {
            direction: crate::api::common::stats::Direction::Inbound,
            ssrc: session.get_ssrc(),
            media_type: crate::api::common::frame::MediaFrameType::Audio, // Default to audio
            packet_count: rtp_stats.packets_received,
            byte_count: rtp_stats.bytes_received,
            packets_lost: rtp_stats.packets_lost,
            fraction_lost: if rtp_stats.packets_received > 0 {
                rtp_stats.packets_lost as f32 / rtp_stats.packets_received as f32
            } else {
                0.0
            },
            jitter_ms: rtp_stats.jitter_ms as f32,
            rtt_ms: None, // Not available yet
            mos: None, // Not calculated yet
            remote_addr: client.address,
            bitrate_bps: 0, // Will calculate based on received data
            discard_rate: 0.0,
            quality: QualityLevel::Unknown,
            available_bandwidth_bps: None,
        };
        
        // Calculate stream quality
        stream_stats.quality = calculate_stream_quality(&stream_stats);
        
        // Add to our aggregate stats
        agg_stats.streams.insert(stream_stats.ssrc, stream_stats);
        
        // Update aggregate bandwidth - we don't have estimated_bitrate_bps, so calculate it based on received data
        // This is a very simplistic estimation
        let now = std::time::SystemTime::now();
        let session_duration_secs = client.created_at.elapsed().unwrap_or_default().as_secs_f64();
        if session_duration_secs > 0.0 {
            let bitrate = (rtp_stats.bytes_received as f64 * 8.0 / session_duration_secs) as u32;
            agg_stats.downstream_bandwidth_bps += bitrate;
        }
    }
    
    // Set quality level based on aggregated stats
    agg_stats.quality = estimate_quality_level(&agg_stats);
    
    Ok(agg_stats)
}

/// Get statistics for a specific client
pub async fn get_client_stats(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<MediaStats, MediaTransportError> {
    // Find client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get stats
    let session = client.session.lock().await;
    let rtp_stats = session.get_stats();
    
    // Create the MediaStats struct
    let mut media_stats = MediaStats::default();
    
    // Set session duration - use elapsed time since client connection
    media_stats.session_duration = client.created_at.elapsed().unwrap_or_default();
    
    // Calculate estimated bitrate based on received data and session duration
    let session_duration_secs = media_stats.session_duration.as_secs_f64();
    let bitrate = if session_duration_secs > 0.0 {
        (rtp_stats.bytes_received as f64 * 8.0 / session_duration_secs) as u32
    } else {
        0
    };
    
    // Create a stream entry
    let mut stream_stats = StreamStats {
        direction: crate::api::common::stats::Direction::Inbound,
        ssrc: session.get_ssrc(),
        media_type: crate::api::common::frame::MediaFrameType::Audio, // Default to audio
        packet_count: rtp_stats.packets_received,
        byte_count: rtp_stats.bytes_received,
        packets_lost: rtp_stats.packets_lost,
        fraction_lost: if rtp_stats.packets_received > 0 {
            rtp_stats.packets_lost as f32 / rtp_stats.packets_received as f32
        } else {
            0.0
        },
        jitter_ms: rtp_stats.jitter_ms as f32,
        rtt_ms: None, // Not available yet
        mos: None, // Not calculated yet
        remote_addr: client.address,
        bitrate_bps: bitrate,
        discard_rate: 0.0,
        quality: QualityLevel::Unknown,
        available_bandwidth_bps: None,
    };
    
    // Calculate stream quality
    stream_stats.quality = calculate_stream_quality(&stream_stats);
    
    // Add to our stats
    media_stats.streams.insert(stream_stats.ssrc, stream_stats.clone());
    
    // Set the downstream bandwidth
    media_stats.downstream_bandwidth_bps = bitrate;
    
    // Set the quality level
    media_stats.quality = calculate_stream_quality(&stream_stats);
    
    Ok(media_stats)
} 