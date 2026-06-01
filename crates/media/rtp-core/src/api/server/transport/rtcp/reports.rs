//! RTCP reports functionality
//!
//! This module handles RTCP sender and receiver reports.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::warn;

use crate::api::client::transport::RtcpStats;
use crate::api::common::error::MediaTransportError;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::session::RtpSession;

/// Send RTCP receiver report to all clients
pub async fn send_rtcp_receiver_report(
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<(), MediaTransportError> {
    let connected_ids: Vec<String> = clients
        .iter()
        .filter(|e| e.value().connected)
        .map(|e| e.key().clone())
        .collect();

    if clients.is_empty() {
        return Err(MediaTransportError::NoClients);
    }

    for client_id in connected_ids {
        if let Err(e) = send_rtcp_receiver_report_to_client(&client_id, clients).await {
            warn!(
                "Failed to send RTCP receiver report to client {}: {}",
                client_id, e
            );
        }
    }

    Ok(())
}

/// Send RTCP sender report to all clients
pub async fn send_rtcp_sender_report(
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<(), MediaTransportError> {
    let connected_ids: Vec<String> = clients
        .iter()
        .filter(|e| e.value().connected)
        .map(|e| e.key().clone())
        .collect();

    if clients.is_empty() {
        return Err(MediaTransportError::NoClients);
    }

    for client_id in connected_ids {
        if let Err(e) = send_rtcp_sender_report_to_client(&client_id, clients).await {
            warn!(
                "Failed to send RTCP sender report to client {}: {}",
                client_id, e
            );
        }
    }

    Ok(())
}

/// Send RTCP receiver report to a specific client
pub async fn send_rtcp_receiver_report_to_client(
    client_id: &str,
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<(), MediaTransportError> {
    let session_arc = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        client.session.clone()
    };

    let session = session_arc.lock().await;
    session.send_receiver_report().await.map_err(|e| {
        MediaTransportError::RtcpError(format!("Failed to send RTCP receiver report: {}", e))
    })
}

/// Send RTCP sender report to a specific client
pub async fn send_rtcp_sender_report_to_client(
    client_id: &str,
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<(), MediaTransportError> {
    let session_arc = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        client.session.clone()
    };

    let session = session_arc.lock().await;
    session.send_sender_report().await.map_err(|e| {
        MediaTransportError::RtcpError(format!("Failed to send RTCP sender report: {}", e))
    })
}

/// Get RTCP statistics for all clients
pub async fn get_rtcp_stats(
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<RtcpStats, MediaTransportError> {
    let connected_ids: Vec<String> = clients
        .iter()
        .filter(|e| e.value().connected)
        .map(|e| e.key().clone())
        .collect();

    if clients.is_empty() {
        return Err(MediaTransportError::NoClients);
    }

    let mut aggregate_stats = RtcpStats::default();
    let mut client_count = 0;

    for client_id in &connected_ids {
        match get_client_rtcp_stats(client_id, clients).await {
            Ok(stats) => {
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
            }
            Err(e) => {
                warn!("Failed to get RTCP stats for client {}: {}", client_id, e);
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
    clients: &Arc<DashMap<String, ClientConnection>>,
) -> Result<RtcpStats, MediaTransportError> {
    let session_arc = {
        let client = clients
            .get(client_id)
            .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
        if !client.connected {
            return Err(MediaTransportError::ClientNotConnected(
                client_id.to_string(),
            ));
        }
        client.session.clone()
    };

    let session = session_arc.lock().await;
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
        rtcp_stats.packet_loss_percent =
            (rtp_stats.packets_lost as f64 / rtp_stats.packets_received as f64) * 100.0;
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
    clients: &Arc<DashMap<String, ClientConnection>>,
    interval: Duration,
) -> Result<(), MediaTransportError> {
    // Snapshot connected sessions; setting bandwidth then happens
    // outside the DashMap iter guard.
    let sessions: Vec<Arc<Mutex<RtpSession>>> = clients
        .iter()
        .filter(|e| e.value().connected)
        .map(|e| e.value().session.clone())
        .collect();

    // The bandwidth calculation follows from RFC 3550 where RTCP
    // bandwidth is typically 5% of session bandwidth. Assuming an
    // average RTCP packet around 100 bytes:
    let bytes_per_second = 100.0 / interval.as_secs_f64();
    let bits_per_second = bytes_per_second * 8.0 / 0.05;

    for session_arc in sessions {
        let mut session = session_arc.lock().await;
        session.set_bandwidth(bits_per_second as u32);
    }

    Ok(())
}
