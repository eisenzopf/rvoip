//! Client connection management
//!
//! This module handles client connection establishment, management, and disconnection.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::api::common::error::MediaTransportError;
use crate::api::common::frame::MediaFrame;
use crate::api::server::security::{ServerSecurityContext, ClientSecurityContext};
use crate::api::server::config::ServerConfig;
use crate::api::server::transport::ClientInfo;
use crate::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
// payload registry moved to media-core

/// Client connection in the server
pub struct ClientConnection {
    /// Client ID
    pub(crate) id: String,
    /// Remote address
    pub(crate) address: SocketAddr,
    /// RTP session for this client
    pub(crate) session: Arc<Mutex<RtpSession>>,
    /// Security context for this client
    pub(crate) security: Option<Arc<dyn ClientSecurityContext + Send + Sync>>,
    /// Task handle for packet forwarding
    pub(crate) task: Option<JoinHandle<()>>,
    /// Is connected
    pub(crate) connected: bool,
    /// Creation time
    pub(crate) created_at: SystemTime,
    /// Last activity time
    pub(crate) last_activity: Arc<Mutex<SystemTime>>,
}

/// Handle an incoming client connection
pub async fn handle_client(
    addr: SocketAddr,
    config: &ServerConfig,
    security_context: &Arc<RwLock<Option<Arc<dyn ServerSecurityContext + Send + Sync>>>>,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    frame_sender: &broadcast::Sender<(String, MediaFrame)>,
    client_connected_callbacks: &Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
) -> Result<String, crate::api::common::error::MediaTransportError> {
    info!("Handling new client from {}", addr);
    
    let client_id = format!("client-{}", Uuid::new_v4());
    debug!("Assigned client ID: {}", client_id);
    
    // Create RTP session config for this client
    let session_config = RtpSessionConfig {
        local_addr: config.local_address,
        remote_addr: Some(addr),
        ssrc: Some(rand::random()),
        payload_type: config.default_payload_type,
        clock_rate: config.clock_rate,
        jitter_buffer_size: Some(config.jitter_buffer_size as usize),
        max_packet_age_ms: Some(config.jitter_max_packet_age_ms),
        enable_jitter_buffer: config.enable_jitter_buffer,
    };
    
    // Create RTP session
    let rtp_session = RtpSession::new(session_config).await
        .map_err(|e| MediaTransportError::Transport(format!("Failed to create client RTP session: {}", e)))?;
        
    let rtp_session = Arc::new(Mutex::new(rtp_session));
    
    // Create security context if needed
    let security_ctx = if config.security_config.security_mode.is_enabled() {
        // Check if we have a server security context
        let server_ctx_option = security_context.read().await;
        
        if let Some(server_ctx) = server_ctx_option.as_ref() {
            // Create client context from server context - still using the trait method
            let client_ctx: Arc<dyn ClientSecurityContext + Send + Sync> = server_ctx.create_client_context(addr).await
                .map_err(|e| MediaTransportError::Security(format!("Failed to create client security context: {}", e)))?;
            
            // Get socket from the session
            let session = rtp_session.lock().await;
            let socket_arc = session.get_socket_handle().await
                .map_err(|e| MediaTransportError::Transport(format!("Failed to get socket handle: {}", e)))?;
            drop(session);
            
            // Create a proper SocketHandle from the UdpSocket
            let socket_handle = crate::api::server::security::SocketHandle {
                socket: socket_arc,
                remote_addr: Some(addr),
            };
            
            // Set socket on the client context
            client_ctx.set_socket(socket_handle).await
                .map_err(|e| MediaTransportError::Security(format!("Failed to set socket on client security context: {}", e)))?;
            
            Some(client_ctx)
        } else {
            return Err(MediaTransportError::Security("Server security context not initialized".to_string()));
        }
    } else {
        None
    };
    
    // Start a task to forward frames from this client
    let frame_sender_clone = frame_sender.clone(); // Clone directly from self
    let client_id_clone = client_id.clone();
    let session_clone = rtp_session.clone();
    
    let forward_task = tokio::spawn(async move {
        let session = session_clone.lock().await;
        let mut event_rx = session.subscribe();
        drop(session);
        
        while let Ok(event) = event_rx.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    // Determine frame type from payload type
                    let frame_type = crate::api::common::frame::MediaFrameType::Audio; // Default to Audio, media-core handles frame type
                    
                    // Convert to MediaFrame
                    let frame = MediaFrame {
                        frame_type,
                        data: packet.payload.to_vec(),
                        timestamp: packet.header.timestamp,
                        sequence: packet.header.sequence_number,
                        marker: packet.header.marker,
                        payload_type: packet.header.payload_type,
                        ssrc: packet.header.ssrc,
                        csrcs: packet.header.csrc.clone(),
                    };
                    
                    // Forward to server via broadcast channel
                    // We can ignore send errors since they just mean no receivers are listening
                    if let Err(e) = frame_sender_clone.send((client_id_clone.clone(), frame)) {
                        // Only log this as a warning, not an error - it's expected if no subscribers
                        debug!("No receivers for frame from client {}: {}", client_id_clone, e);
                    }
                },
                RtpSessionEvent::NewStreamDetected { ssrc } => {
                    debug!("New stream with SSRC={:08x} detected for client {}", ssrc, client_id_clone);
                },
                _ => {} // Handle other events if needed
            }
        }
    });
    
    // Create client connection
    let client = ClientConnection {
        id: client_id.clone(),
        address: addr,
        session: rtp_session,
        security: security_ctx,
        task: Some(forward_task),
        connected: true,
        created_at: SystemTime::now(),
        last_activity: Arc::new(Mutex::new(SystemTime::now())),
    };
    
    // Add to clients
    let mut clients_guard = clients.write().await;
    clients_guard.insert(client_id.clone(), client);
    drop(clients_guard);
    
    // Create client info
    let client_info = ClientInfo {
        id: client_id.clone(),
        address: addr,
        secure: false, // Will be updated once handshake is complete
        security_info: None,
        connected: true,
    };
    
    // Notify callbacks
    let callbacks_guard = client_connected_callbacks.read().await;
    for callback in &*callbacks_guard {
        callback(client_info.clone());
    }
    
    Ok(client_id)
}

/// Static helper function to handle a new client connection
pub async fn handle_client_static(
    addr: SocketAddr,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    frame_sender: &broadcast::Sender<(String, MediaFrame)>
) -> Result<String, crate::api::common::error::MediaTransportError> {
    info!("Handling new client from {}", addr);
    
    let client_id = format!("client-{}", Uuid::new_v4());
    debug!("Assigned client ID: {}", client_id);
    
    // Create RTP session config for this client - bind to 0.0.0.0:0 to let OS choose a port
    let session_config = RtpSessionConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(),
        remote_addr: Some(addr),
        ssrc: Some(rand::random()),
        payload_type: 8, // Default payload type
        clock_rate: 8000, // Default clock rate
        jitter_buffer_size: Some(50 as usize), // Default buffer size
        max_packet_age_ms: Some(200), // Default max packet age
        enable_jitter_buffer: true,
    };
    
    // Create RTP session
    debug!("Creating RTP session for client {}", client_id);
    let rtp_session = RtpSession::new(session_config).await
        .map_err(|e| MediaTransportError::Transport(format!("Failed to create client RTP session: {}", e)))?;
        
    let rtp_session = Arc::new(Mutex::new(rtp_session));
    
    // Create client connection without security for now (will be added later)
    let client = ClientConnection {
        id: client_id.clone(),
        address: addr,
        session: rtp_session,
        security: None,
        task: None,
        connected: true,
        created_at: SystemTime::now(),
        last_activity: Arc::new(Mutex::new(SystemTime::now())),
    };
    
    // Start a task to forward frames from this client
    let frame_sender_clone = frame_sender.clone();
    let client_id_clone = client_id.clone();
    let session_clone = client.session.clone();
    
    debug!("Starting packet forwarding task for client {}", client_id);
    let forward_task = tokio::spawn(async move {
        let session = session_clone.lock().await;
        
        // Get session details for debugging
        debug!("Session details - SSRC: {}, Target: {}", 
               session.get_ssrc(), addr);
        
        let mut event_rx = session.subscribe();
        drop(session);
        
        debug!("Starting packet receive loop for client {}", client_id_clone);
        let mut packets_received = 0;
        
        while let Ok(event) = event_rx.recv().await {
            match event {
                RtpSessionEvent::PacketReceived(packet) => {
                    packets_received += 1;
                    
                    // Determine frame type from payload type
                    let frame_type = crate::api::common::frame::MediaFrameType::Audio; // Default to Audio, media-core handles frame type
                    
                    // Log packet details
                    debug!("Client {}: Received packet #{} - PT: {}, Seq: {}, TS: {}, Size: {} bytes",
                          client_id_clone, packets_received, 
                          packet.header.payload_type, packet.header.sequence_number,
                          packet.header.timestamp, packet.payload.len());
                    
                    // Convert to MediaFrame
                    let frame = MediaFrame {
                        frame_type,
                        data: packet.payload.to_vec(),
                        timestamp: packet.header.timestamp,
                        sequence: packet.header.sequence_number,
                        marker: packet.header.marker,
                        payload_type: packet.header.payload_type,
                        ssrc: packet.header.ssrc,
                        csrcs: packet.header.csrc.clone(),
                    };
                    
                    // Forward to server via broadcast channel
                    match frame_sender_clone.send((client_id_clone.clone(), frame)) {
                        Ok(receiver_count) => {
                            debug!("Broadcast packet to {} receivers - Client: {}, Seq: {}", 
                                  receiver_count, client_id_clone, packet.header.sequence_number);
                        },
                        Err(e) => {
                            // This is expected if no subscribers are listening
                            debug!("No receivers for frame from client {}: {}", client_id_clone, e);
                        }
                    }
                },
                other_event => {
                    debug!("Client {}: Received non-packet event: {:?}", client_id_clone, other_event);
                }
            }
        }
        
        debug!("Packet forwarding task ended for client {}", client_id_clone);
    });
    
    // Update the client with the task
    let mut client_with_task = client;
    client_with_task.task = Some(forward_task);
    
    // Add to clients
    debug!("Adding client {} to clients map", client_id);
    let mut clients_write = clients.write().await;
    clients_write.insert(client_id.clone(), client_with_task);
    
    info!("Successfully added client {}", client_id);
    Ok(client_id)
}

/// Get client transport information (address and session)
pub async fn get_client_transport_info(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
) -> Result<(SocketAddr, Arc<Mutex<RtpSession>>), MediaTransportError> {
    // Get the client
    let clients_guard = clients.read().await;
    let client = clients_guard.get(client_id)
        .ok_or_else(|| MediaTransportError::ClientNotFound(client_id.to_string()))?;
    
    // Check if client is connected
    if !client.connected {
        return Err(MediaTransportError::ClientNotConnected(client_id.to_string()));
    }
    
    // Return client address and session
    Ok((client.address, client.session.clone()))
}

/// Disconnect a client
pub async fn disconnect_client(
    client_id: &str,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    client_disconnected_callbacks: &Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
) -> Result<(), MediaTransportError> {
    // Remove client
    let mut clients_guard = clients.write().await;
    
    if let Some(mut client) = clients_guard.remove(client_id) {
        // Abort task
        if let Some(task) = client.task.take() {
            task.abort();
        }
        
        // Close session
        let mut session = client.session.lock().await;
        if let Err(e) = session.close().await {
            warn!("Error closing client session {}: {}", client_id, e);
        }
        
        // Close security context if it exists
        if let Some(security_ctx) = &client.security {
            if let Err(e) = security_ctx.close().await {
                warn!("Error closing client security {}: {}", client_id, e);
            }
        }
        
        // Notify callbacks
        let callbacks_guard = client_disconnected_callbacks.read().await;
        let client_info = ClientInfo {
            id: client.id.clone(),
            address: client.address,
            secure: client.security.is_some(),
            security_info: None,
            connected: false,
        };
        
        for callback in &*callbacks_guard {
            callback(client_info.clone());
        }
        
        Ok(())
    } else {
        Err(MediaTransportError::Transport(format!("Client not found: {}", client_id)))
    }
}

/// Get client information
pub async fn get_clients_info(
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    config: &ServerConfig,
) -> Result<Vec<ClientInfo>, MediaTransportError> {
    let clients_guard = clients.read().await;
    
    let mut result = Vec::new();
    for client in clients_guard.values() {
        // Get security info if available
        let security_info = if let Some(security_ctx) = &client.security {
            // We can directly access methods on the ClientSecurityContext trait
            let fingerprint = security_ctx.get_remote_fingerprint().await.ok().flatten();
            
            if let Some(fingerprint) = fingerprint {
                Some(crate::api::common::config::SecurityInfo {
                    mode: config.security_config.security_mode,
                    fingerprint: Some(fingerprint),
                    fingerprint_algorithm: Some(config.security_config.fingerprint_algorithm.clone()),
                    crypto_suites: security_ctx.get_security_info().crypto_suites.clone(),
                    key_params: None,
                    srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()), // Default profile
                })
            } else {
                None
            }
        } else {
            None
        };
        
        result.push(ClientInfo {
            id: client.id.clone(),
            address: client.address,
            secure: client.security.is_some(),
            security_info,
            connected: client.connected,
        });
    }
    
    Ok(result)
} 