//! RTCP application packets functionality
//!
//! This module handles RTCP application-defined packets, BYE packets, and XR packets.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, warn};
use bytes::Bytes;

use crate::api::common::error::MediaTransportError;
use crate::api::client::transport::VoipMetrics;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::packet::rtcp::{RtcpPacket, RtcpApplicationDefined, RtcpGoodbye, RtcpExtendedReport, RtcpXrBlock, VoipMetricsBlock};
use crate::transport::RtpTransport;

/// Send RTCP APP packet to all clients
pub async fn send_rtcp_app(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    name: &str,
    data: Vec<u8>,
) -> Result<(), MediaTransportError> {
    // Get all connected clients
    let clients_guard = clients.read().await;
    
    if clients_guard.is_empty() {
        debug!("No clients to send RTCP APP packet to");
        return Ok(());
    }
    
    // Send to each client
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            if let Err(e) = send_rtcp_app_to_client(
                client_id,
                clients,
                main_socket,
                name,
                data.clone()
            ).await {
                warn!("Failed to send RTCP APP packet to client {}: {}", client_id, e);
            }
        }
    }
    
    Ok(())
}

/// Send RTCP APP packet to a specific client
pub async fn send_rtcp_app_to_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    name: &str,
    data: Vec<u8>,
) -> Result<(), MediaTransportError> {
    // Validate name (must be exactly 4 ASCII characters)
    if name.len() != 4 || !name.is_ascii() {
        return Err(MediaTransportError::ConfigError(
            "APP name must be exactly 4 ASCII characters".to_string()
        ));
    }
    
    // Get client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get client address
    let client_addr = client.address;
    
    // Get the SSRC to use
    let ssrc = client.session.lock().await.get_ssrc();
    
    // Create APP packet
    let mut app_packet = RtcpApplicationDefined::new_with_name(ssrc, name)
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to create APP packet: {}", e)))?;
    
    // Set data
    app_packet.set_data(Bytes::from(data.clone()));
    
    // Create RTCP packet
    let rtcp_packet = RtcpPacket::ApplicationDefined(app_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize APP packet: {}", e)))?;
    
    // Get transport
    let transport_guard = main_socket.read().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Transport not initialized".to_string()))?;
    
    // Send to client
    transport.send_rtcp_bytes(&rtcp_data, client_addr).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send APP packet: {}", e)))?;
    
    debug!("Sent RTCP APP packet to client {}: name={}, data_len={}", 
           client_id, name, data.len());
    
    Ok(())
}

/// Send RTCP BYE packet to all clients
pub async fn send_rtcp_bye(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    reason: Option<String>,
) -> Result<(), MediaTransportError> {
    // Get all connected clients
    let clients_guard = clients.read().await;
    
    if clients_guard.is_empty() {
        debug!("No clients to send RTCP BYE packet to");
        return Ok(());
    }
    
    // Send to each client
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            if let Err(e) = send_rtcp_bye_to_client(
                client_id,
                clients,
                main_socket,
                reason.clone()
            ).await {
                warn!("Failed to send RTCP BYE packet to client {}: {}", client_id, e);
            }
        }
    }
    
    Ok(())
}

/// Send RTCP BYE packet to a specific client
pub async fn send_rtcp_bye_to_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    reason: Option<String>,
) -> Result<(), MediaTransportError> {
    // Get client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get client address
    let client_addr = client.address;
    
    // Get the SSRC to use
    let ssrc = client.session.lock().await.get_ssrc();
    
    // Create BYE packet
    let reason_clone = reason.clone();
    let bye_packet = RtcpGoodbye {
        sources: vec![ssrc],
        reason,
    };
    
    // Create RTCP packet
    let rtcp_packet = RtcpPacket::Goodbye(bye_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize BYE packet: {}", e)))?;
    
    // Get transport
    let transport_guard = main_socket.read().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Transport not initialized".to_string()))?;
    
    // Send to client
    transport.send_rtcp_bytes(&rtcp_data, client_addr).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send BYE packet: {}", e)))?;
    
    debug!("Sent RTCP BYE packet to client {}: reason={:?}", client_id, reason_clone);
    
    Ok(())
}

/// Send RTCP XR VoIP metrics packet to all clients
pub async fn send_rtcp_xr_voip_metrics(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    metrics: VoipMetrics,
) -> Result<(), MediaTransportError> {
    // Get all connected clients
    let clients_guard = clients.read().await;
    
    if clients_guard.is_empty() {
        debug!("No clients to send RTCP XR packet to");
        return Ok(());
    }
    
    // Send to each client
    for (client_id, client) in clients_guard.iter() {
        if client.connected {
            if let Err(e) = send_rtcp_xr_voip_metrics_to_client(
                client_id,
                clients,
                main_socket,
                metrics.clone()
            ).await {
                warn!("Failed to send RTCP XR packet to client {}: {}", client_id, e);
            }
        }
    }
    
    Ok(())
}

/// Send RTCP XR VoIP metrics packet to a specific client
pub async fn send_rtcp_xr_voip_metrics_to_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    metrics: VoipMetrics,
) -> Result<(), MediaTransportError> {
    // Get client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Get client address
    let client_addr = client.address;
    
    // Get the SSRC to use
    let ssrc = client.session.lock().await.get_ssrc();
    
    // Create XR packet
    let mut xr_packet = RtcpExtendedReport::new(ssrc);
    
    // Convert our metrics to VoipMetricsBlock
    let voip_metrics_block = VoipMetricsBlock {
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
    xr_packet.add_block(RtcpXrBlock::VoipMetrics(voip_metrics_block));
    
    // Create RTCP packet
    let rtcp_packet = RtcpPacket::ExtendedReport(xr_packet);
    
    // Serialize
    let rtcp_data = rtcp_packet.serialize()
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize XR packet: {}", e)))?;
    
    // Get transport
    let transport_guard = main_socket.read().await;
    let transport = transport_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Transport not initialized".to_string()))?;
    
    // Send to client
    transport.send_rtcp_bytes(&rtcp_data, client_addr).await
        .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send XR packet: {}", e)))?;
    
    debug!("Sent RTCP XR VoIP metrics to client {}: loss_rate={}%, r_factor={}", 
           client_id, metrics.loss_rate, metrics.r_factor);
    
    Ok(())
} 