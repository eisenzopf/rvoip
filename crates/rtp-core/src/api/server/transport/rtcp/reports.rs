//! RTCP reports functionality
//!
//! This module handles RTCP sender and receiver reports.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn};

use crate::api::common::error::MediaTransportError;
use crate::api::client::transport::RtcpStats;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::session::RtpSession;
use crate::transport::UdpRtpTransport;

/// Send RTCP receiver report to all clients
pub async fn send_rtcp_receiver_report(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(), MediaTransportError> {
    // Get all clients
    let clients_guard = clients.read().await;
    
    // Return error if no clients
    if clients_guard.is_empty() {
        return Err(MediaTransportError::NoClients);
    }
    
    // Send RTCP receiver report to all clients
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            if let Err(e) = send_rtcp_receiver_report_to_client(
                client_id,
                clients
            ).await {
                warn!("Failed to send RTCP receiver report to client {}: {}", client_id, e);
                // Continue with other clients even if one fails
            }
        }
    }
    
    Ok(())
}

/// Send RTCP sender report to all clients
pub async fn send_rtcp_sender_report(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(), MediaTransportError> {
    // Get all clients
    let clients_guard = clients.read().await;
    
    // Return error if no clients
    if clients_guard.is_empty() {
        return Err(MediaTransportError::NoClients);
    }
    
    // Send RTCP sender report to all clients
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            if let Err(e) = send_rtcp_sender_report_to_client(
                client_id,
                clients
            ).await {
                warn!("Failed to send RTCP sender report to client {}: {}", client_id, e);
                // Continue with other clients even if one fails
            }
        }
    }
    
    Ok(())
}

/// Send RTCP receiver report to a specific client
pub async fn send_rtcp_receiver_report_to_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(), MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Send RTCP receiver report
    let mut session = client.session.lock().await;
    session.send_receiver_report().await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP receiver report: {}", e)))
}

/// Send RTCP sender report to a specific client
pub async fn send_rtcp_sender_report_to_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(), MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Send RTCP sender report
    let mut session = client.session.lock().await;
    session.send_sender_report().await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP sender report: {}", e)))
}

/// Get RTCP statistics for all clients
pub async fn get_rtcp_stats(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<RtcpStats, MediaTransportError> {
    // Get all clients
    let clients_guard = clients.read().await;
    
    // Return error if no clients
    if clients_guard.is_empty() {
        return Err(MediaTransportError::NoClients);
    }
    
    // Aggregate RTCP stats from all clients
    let mut aggregate_stats = RtcpStats::default();
    let mut client_count = 0;
    
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            match get_client_rtcp_stats(
                client_id,
                clients
            ).await {
                Ok(stats) => {
                    // Aggregate stats (simple averaging for now)
                    aggregate_stats.jitter_ms += stats.jitter_ms;
                    aggregate_stats.packet_loss_percent += stats.packet_loss_percent;
                    if let Some(rtt) = stats.round_trip_time_ms {
                        if let Some(existing_rtt) = aggregate_stats.round_trip_time_ms {
                            aggregate_stats.round_trip_time_ms = Some(existing_rtt + rtt);
                        } else {
                            aggregate_stats.round_trip_time_ms = Some(rtt);
                        }
                    }
                    aggregate_stats.rtcp_packets_sent += stats.rtcp_packets_sent;
                    aggregate_stats.rtcp_packets_received += stats.rtcp_packets_received;
                    aggregate_stats.cumulative_packets_lost += stats.cumulative_packets_lost;
                    
                    client_count += 1;
                },
                Err(e) => {
                    warn!("Failed to get RTCP stats for client {}: {}", client_id, e);
                    // Continue with other clients even if one fails
                }
            }
        }
    }
    
    // Calculate averages if we have clients
    if client_count > 0 {
        aggregate_stats.jitter_ms /= client_count as f64;
        aggregate_stats.packet_loss_percent /= client_count as f64;
        if let Some(rtt) = aggregate_stats.round_trip_time_ms {
            aggregate_stats.round_trip_time_ms = Some(rtt / client_count as f64);
        }
    }
    
    Ok(aggregate_stats)
}

/// Get RTCP statistics for a specific client
pub async fn get_client_rtcp_stats(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<RtcpStats, MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get session stats
    let session = client.session.lock().await;
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

/// Set RTCP interval for all clients
pub async fn set_rtcp_interval(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    interval: Duration,
) -> Result<(), MediaTransportError> {
    // Get all clients
    let clients_guard = clients.read().await;
    
    // Set RTCP interval for all clients
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            let mut session = client.session.lock().await;
            
            // The bandwidth calculation follows from RFC 3550 where RTCP bandwidth is typically 
            // 5% of session bandwidth. If we want a specific interval, we need to set the
            // bandwidth accordingly: bandwidth = packet_size * 8 / interval_fraction
            // where interval_fraction is 0.05 for 5%
            
            // Assuming average RTCP packet is around 100 bytes, calculate bandwidth
            let bytes_per_second = 100.0 / interval.as_secs_f64();
            let bits_per_second = bytes_per_second * 8.0 / 0.05; // 5% of bandwidth for RTCP
            
            // Set bandwidth on the session
            session.set_bandwidth(bits_per_second as u32);
        }
    }
    
    Ok(())
} 