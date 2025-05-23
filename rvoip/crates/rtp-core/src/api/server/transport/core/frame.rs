//! Frame processing
//!
//! This module handles frame sending, receiving, and broadcasting functionality.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use bytes::Bytes;
use tokio::sync::{Mutex, RwLock, broadcast};
use tracing::{debug, warn};

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::frame::MediaFrameType;
use crate::api::server::transport::core::connection::ClientConnection;
use crate::packet::RtpPacket;
use crate::session::RtpSession;
use crate::{CsrcManager, RtpCsrc, MAX_CSRC_COUNT};
use crate::transport::{UdpRtpTransport, RtpTransport};

/// Send a media frame to a specific client
pub async fn send_frame_to(
    client_id: &str, 
    frame: MediaFrame, 
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    ssrc_demultiplexing_enabled: &Arc<RwLock<bool>>,
    csrc_management_enabled: &Arc<RwLock<bool>>,
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
) -> Result<(), MediaTransportError> {
    // Get client transport info
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    let addr = client.address;
    let session = client.session.clone();
    drop(clients_guard);
    
    let mut session_guard = session.lock().await;
    
    // Get SSRC to use
    let ssrc = if *ssrc_demultiplexing_enabled.read().await && frame.ssrc != 0 {
        frame.ssrc
    } else {
        session_guard.get_ssrc()
    };
    
    // Create RTP header
    let mut header = crate::packet::RtpHeader::new(
        frame.payload_type,
        frame.sequence,
        frame.timestamp,
        ssrc
    );
    
    // Set marker bit
    if frame.marker {
        header.marker = true;
    }
    
    // Add CSRCs if CSRC management is enabled
    if *csrc_management_enabled.read().await {
        // For simplicity, we'll just use all known SSRCs as active sources
        // In a real conference mixer, this would be based on audio activity
        let clients_guard = clients.read().await;
        
        // Get all SSRCs from all clients (simplified approach)
        let mut active_ssrcs = Vec::new();
        for client in clients_guard.values() {
            if client.connected {
                let client_session = client.session.lock().await;
                active_ssrcs.push(client_session.get_ssrc());
                // In a real implementation, we would also include all streams tracked by this client
            }
        }
        drop(clients_guard);
        
        if !active_ssrcs.is_empty() {
            // Get CSRC values from the manager
            let csrc_manager_guard = csrc_manager.read().await;
            let csrcs = csrc_manager_guard.get_active_csrcs(&active_ssrcs);
            
            // Take only up to MAX_CSRC_COUNT
            let csrcs = if csrcs.len() > MAX_CSRC_COUNT as usize {
                csrcs[0..MAX_CSRC_COUNT as usize].to_vec()
            } else {
                csrcs
            };
            
            // Add CSRCs to the header if we have any
            if !csrcs.is_empty() {
                debug!("Adding {} CSRCs to outgoing packet", csrcs.len());
                header.add_csrcs(&csrcs);
            }
        }
    }
    
    // Store frame data length before it's moved
    let data_len = frame.data.len(); 
    
    // Create RTP packet
    let packet = RtpPacket::new(
        header,
        Bytes::from(frame.data),
    );
    
    // Get main socket
    let socket_guard = main_socket.read().await;
    let socket = socket_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Server is not running".to_string()))?;
    
    // Send packet
    socket.send_rtp(&packet, addr).await
        .map_err(|e| MediaTransportError::SendError(format!("Failed to send RTP packet: {}", e)))?;
    
    // We don't have update_sent_stats method in RtpSession, so we'll just log
    debug!("Sent frame to client {}: PT={}, TS={}, SEQ={}, Size={} bytes", 
           client_id, frame.payload_type, frame.timestamp, frame.sequence, data_len);
    
    Ok(())
}

/// Broadcast a media frame to all connected clients
pub async fn broadcast_frame(
    frame: MediaFrame, 
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    csrc_management_enabled: &Arc<RwLock<bool>>, 
    csrc_manager: &Arc<RwLock<CsrcManager>>,
    main_socket: &Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
) -> Result<(), MediaTransportError> {
    // Create a base header with frame info
    let mut base_header = crate::packet::RtpHeader::new(
        frame.payload_type,
        frame.sequence,
        frame.timestamp,
        frame.ssrc
    );
    
    // Set marker bit
    if frame.marker {
        base_header.marker = true;
    }
    
    // Add CSRCs if CSRC management is enabled
    if *csrc_management_enabled.read().await {
        // Get list of connected clients
        let clients_guard = clients.read().await;
        
        // Get all SSRCs from all clients (simplified approach)
        let mut active_ssrcs = Vec::new();
        for client in clients_guard.values() {
            if client.connected {
                let client_session = client.session.lock().await;
                active_ssrcs.push(client_session.get_ssrc());
                // In a real implementation, we would also include all streams tracked by this client
            }
        }
        drop(clients_guard);
        
        if !active_ssrcs.is_empty() {
            // Get CSRC values from the manager
            let csrc_manager_guard = csrc_manager.read().await;
            let csrcs = csrc_manager_guard.get_active_csrcs(&active_ssrcs);
            
            // Take only up to MAX_CSRC_COUNT
            let csrcs = if csrcs.len() > MAX_CSRC_COUNT as usize {
                csrcs[0..MAX_CSRC_COUNT as usize].to_vec()
            } else {
                csrcs
            };
            
            // Add CSRCs to the header if we have any
            if !csrcs.is_empty() {
                debug!("Adding {} CSRCs to outgoing broadcast packet", csrcs.len());
                base_header.add_csrcs(&csrcs);
            }
        }
    }
    
    // Create RTP packet once with shared data
    let shared_data = Arc::new(Bytes::from(frame.data));
    
    // Get main socket
    let socket_guard = main_socket.read().await;
    let socket = socket_guard.as_ref()
        .ok_or_else(|| MediaTransportError::Transport("Server is not running".to_string()))?;
    
    // Get list of connected clients
    let clients_guard = clients.read().await;
    
    // Send to each client (in parallel)
    let mut send_tasks = Vec::new();
    
    for (client_id, client) in clients_guard.iter() {
        if !client.connected {
            continue;
        }
        
        // Clone header for each client
        let header = base_header.clone();
        
        // Clone data reference
        let data = shared_data.clone();
        
        // Get client address
        let addr = client.address;
        
        // Create RTP packet
        let packet = crate::packet::RtpPacket::new(
            header,
            Bytes::clone(&data),
        );
        
        // Clone socket reference
        let socket_clone = socket.clone();
        
        // Update stats in client session
        let client_id_clone = client_id.clone();
        let payload_type = frame.payload_type;
        let data_len = data.len();
        
        // Spawn task to send packet and update stats
        let task = tokio::spawn(async move {
            // Send packet
            if let Err(e) = socket_clone.send_rtp(&packet, addr).await {
                warn!("Failed to send broadcast frame to client {}: {}", client_id_clone, e);
                return;
            }
            
            // We don't have update_sent_stats method in RtpSession, so we'll just log
            debug!("Sent broadcast frame to client {}: PT={}, Size={} bytes", 
                   client_id_clone, payload_type, data_len);
        });
        
        send_tasks.push(task);
    }
    drop(clients_guard);
    
    // Wait for all sends to complete
    for task in send_tasks {
        let _ = task.await;
    }
    
    Ok(())
}

/// Receive a media frame from any client
pub async fn receive_frame(
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
) -> Result<(String, MediaFrame), MediaTransportError> {
    // Create a new receiver from the broadcast channel
    let mut receiver = frame_sender.subscribe();
    
    // Wait for a frame with a shorter timeout (500ms instead of 2s)
    match tokio::time::timeout(std::time::Duration::from_millis(500), receiver.recv()).await {
        Ok(Ok(frame)) => {
            // Successfully received frame
            Ok(frame)
        },
        Ok(Err(e)) => {
            // Error receiving from the broadcast channel
            Err(MediaTransportError::Transport(format!("Broadcast channel error: {}", e)))
        },
        Err(_) => {
            // Timeout occurred
            Err(MediaTransportError::Timeout("No frame received within timeout period".to_string()))
        }
    }
} 