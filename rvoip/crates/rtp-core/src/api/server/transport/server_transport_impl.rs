//! Server transport implementation
//!
//! This file contains the implementation of the MediaTransportServer trait.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use std::time::Duration;
use std::time::SystemTime;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::{MediaEventCallback, MediaTransportEvent};
use crate::api::server::config::ServerConfig;
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::server::transport::{MediaTransportServer, ClientInfo};
use crate::api::server::security::{ServerSecurityContext, ClientSecurityContext, DefaultServerSecurityContext};
use crate::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
use crate::transport::{RtpTransportConfig, UdpRtpTransport};
use crate::api::common::stats::QualityLevel;
use crate::transport::RtpTransport;
use crate::api::common::stats::StreamStats;
use crate::api::common::stats::Direction;
use crate::api::common::frame::MediaFrameType;

/// Client connection in the server
struct ClientConnection {
    /// Client ID
    id: String,
    /// Remote address
    address: SocketAddr,
    /// RTP session for this client
    session: Arc<Mutex<RtpSession>>,
    /// Security context for this client
    security: Option<Arc<Mutex<dyn ClientSecurityContext>>>,
    /// Task handle for packet forwarding
    task: Option<JoinHandle<()>>,
    /// Is connected
    connected: bool,
    /// Time when the client was created
    created_at: SystemTime,
    /// Time of the last activity
    last_activity: Arc<Mutex<SystemTime>>,
}

/// Default implementation of the MediaTransportServer
pub struct DefaultMediaTransportServer {
    /// Server configuration
    config: ServerConfig,
    /// Server security context for DTLS
    security_context: Option<Arc<Mutex<dyn ServerSecurityContext>>>,
    /// Connected clients
    clients: Arc<RwLock<HashMap<String, ClientConnection>>>,
    /// Receiver for frames from clients
    frame_receiver: mpsc::Receiver<(String, MediaFrame)>,
    /// Sender for frames from clients
    frame_sender: mpsc::Sender<(String, MediaFrame)>,
    /// Event callbacks
    event_callbacks: Arc<Mutex<Vec<MediaEventCallback>>>,
    /// Client connected callbacks
    client_connected_callbacks: Arc<Mutex<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    /// Client disconnected callbacks
    client_disconnected_callbacks: Arc<Mutex<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    /// Main socket for receiving connections
    main_socket: Option<Arc<UdpRtpTransport>>,
    /// Event handling task
    event_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Server is running flag
    running: Arc<RwLock<bool>>,
}

// Custom implementation of Clone for DefaultMediaTransportServer
impl Clone for DefaultMediaTransportServer {
    fn clone(&self) -> Self {
        // Create a new channel pair for the clone
        let (sender, receiver) = mpsc::channel(100);
        
        // The receiver is not clonable, so we just create a new one
        // For a real application, you would need a proper broker pattern
        // This is a simplified implementation
        
        Self {
            config: self.config.clone(),
            security_context: self.security_context.clone(),
            clients: self.clients.clone(),
            frame_receiver: receiver, // New receiver
            frame_sender: self.frame_sender.clone(), // Shared sender
            event_callbacks: self.event_callbacks.clone(),
            client_connected_callbacks: self.client_connected_callbacks.clone(),
            client_disconnected_callbacks: self.client_disconnected_callbacks.clone(),
            main_socket: self.main_socket.clone(),
            event_task: self.event_task.clone(),
            running: self.running.clone(),
        }
    }
}

impl DefaultMediaTransportServer {
    /// Create a new DefaultMediaTransportServer
    pub async fn new(config: ServerConfig) -> Result<Self, MediaTransportError> {
        // Create transport config for the main socket
        let transport_config = RtpTransportConfig {
            local_rtp_addr: config.local_address,
            local_rtcp_addr: None, // We'll use RTCP multiplexing
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(format!("media-server-{}", Uuid::new_v4())),
            use_port_allocator: true,
        };
        
        // Create channels for frames from clients
        let (frame_tx, frame_rx) = mpsc::channel(100);
        
        let server = Self {
            config,
            security_context: None,
            clients: Arc::new(RwLock::new(HashMap::new())),
            frame_receiver: frame_rx,
            frame_sender: frame_tx,
            event_callbacks: Arc::new(Mutex::new(Vec::new())),
            client_connected_callbacks: Arc::new(Mutex::new(Vec::new())),
            client_disconnected_callbacks: Arc::new(Mutex::new(Vec::new())),
            main_socket: None,
            event_task: Arc::new(Mutex::new(None)),
            running: Arc::new(RwLock::new(false)),
        };
        
        Ok(server)
    }
    
    /// Handle an incoming client connection
    async fn handle_client(&self, addr: SocketAddr) -> Result<String, MediaTransportError> {
        info!("Handling new client from {}", addr);
        
        let client_id = format!("client-{}", Uuid::new_v4());
        debug!("Assigned client ID: {}", client_id);
        
        // Create RTP session config for this client
        let session_config = RtpSessionConfig {
            local_addr: self.config.local_address,
            remote_addr: Some(addr),
            ssrc: Some(rand::random()),
            payload_type: self.config.default_payload_type,
            clock_rate: self.config.clock_rate,
            jitter_buffer_size: Some(self.config.jitter_buffer_size as usize),
            max_packet_age_ms: Some(self.config.jitter_max_packet_age_ms),
            enable_jitter_buffer: self.config.enable_jitter_buffer,
        };
        
        // Create RTP session
        let rtp_session = RtpSession::new(session_config).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to create client RTP session: {}", e)))?;
            
        let rtp_session = Arc::new(Mutex::new(rtp_session));
        
        // Create security context if needed
        let security_ctx = if self.config.security_config.security_mode.is_enabled() {
            // Create client-specific security context from the server security context
            let server_ctx = self.security_context.as_ref()
                .ok_or_else(|| MediaTransportError::Security("Server security context not initialized".to_string()))?;
                
                // Lock the server context
                let server_ctx = server_ctx.lock().await;
                
                // Create client context from server context
                let client_ctx = server_ctx.create_client_context(addr).await
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
            None
        };
        
        // Start a task to forward frames from this client
        let frame_sender = self.frame_sender.clone();
        let client_id_clone = client_id.clone();
        let session_clone = rtp_session.clone();
        
        let forward_task = tokio::spawn(async move {
            let session = session_clone.lock().await;
            let mut event_rx = session.subscribe();
            drop(session);
            
            while let Ok(event) = event_rx.recv().await {
                if let RtpSessionEvent::PacketReceived(packet) = event {
                    // Determine frame type from payload type
                    let frame_type = if packet.header.payload_type <= 34 {
                        crate::api::common::frame::MediaFrameType::Audio
                    } else if packet.header.payload_type >= 35 && packet.header.payload_type <= 50 {
                        crate::api::common::frame::MediaFrameType::Video
                    } else {
                        crate::api::common::frame::MediaFrameType::Data
                    };
                    
                    // Convert to MediaFrame
                    let frame = MediaFrame {
                        frame_type,
                        data: packet.payload.to_vec(),
                        timestamp: packet.header.timestamp,
                        sequence: packet.header.sequence_number,
                        marker: packet.header.marker,
                        payload_type: packet.header.payload_type,
                        ssrc: packet.header.ssrc,
                    };
                    
                    // Forward to server
                    if let Err(e) = frame_sender.send((client_id_clone.clone(), frame)).await {
                        error!("Failed to forward frame from client {}: {}", client_id_clone, e);
                        break;
                    }
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
        let mut clients = self.clients.write().await;
        clients.insert(client_id.clone(), client);
        
        // Create client info
        let client_info = ClientInfo {
            id: client_id.clone(),
            address: addr,
            secure: false, // Will be updated once handshake is complete
            security_info: None,
            connected: true,
        };
        
        // Notify callbacks
        let callbacks = self.client_connected_callbacks.lock().await;
        for callback in callbacks.iter() {
            callback(client_info.clone());
        }
        
        Ok(client_id)
    }
    
    /// Initialize security context if needed
    async fn init_security_if_needed(&self) -> Result<(), MediaTransportError> {
        if self.config.security_config.security_mode.is_enabled() && self.security_context.is_none() {
            // Create server security context
            let security_context = DefaultServerSecurityContext::new(
                self.config.security_config.clone(),
            ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create server security context: {}", e)))?;
            
            // Store context
            self.security_context = Some(Arc::new(Mutex::new(security_context)));
        }
        
        Ok(())
    }
    
    /// Get the frame type based on payload type
    fn get_frame_type_from_payload_type(&self, payload_type: u8) -> crate::api::common::frame::MediaFrameType {
        use crate::api::common::frame::MediaFrameType;
        
        // Very simple heuristic - could be improved with more detailed codec information
        match payload_type {
            // Common audio payload types
            0..=34 | 96..=98 => MediaFrameType::Audio,
            // Common video payload types
            35..=50 | 99..=112 => MediaFrameType::Video,
            // Everything else we'll assume is data
            _ => MediaFrameType::Data,
        }
    }
}

/// Helper function to handle a new client connection
/// This doesn't need a full server instance, just the necessary components
async fn handle_client_static(
    addr: SocketAddr,
    clients: &Arc<RwLock<HashMap<String, ClientConnection>>>,
    frame_sender: &mpsc::Sender<(String, MediaFrame)>
) -> Result<String, MediaTransportError> {
    info!("Handling new client from {}", addr);
    
    let client_id = format!("client-{}", Uuid::new_v4());
    debug!("Assigned client ID: {}", client_id);
    
    // Create RTP session config for this client
    let session_config = RtpSessionConfig {
        local_addr: "0.0.0.0:0".parse().unwrap(), // This needs to be fixed
        remote_addr: Some(addr),
        ssrc: Some(rand::random()),
        payload_type: 8, // Default payload type
        clock_rate: 8000, // Default clock rate
        jitter_buffer_size: Some(50 as usize), // Default buffer size
        max_packet_age_ms: Some(200), // Default max packet age
        enable_jitter_buffer: true,
    };
    
    // Create RTP session
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
    
    let forward_task = tokio::spawn(async move {
        let session = session_clone.lock().await;
        let mut event_rx = session.subscribe();
        drop(session);
        
        while let Ok(event) = event_rx.recv().await {
            if let RtpSessionEvent::PacketReceived(packet) = event {
                // Determine frame type from payload type
                let frame_type = if packet.header.payload_type <= 34 {
                    crate::api::common::frame::MediaFrameType::Audio
                } else if packet.header.payload_type >= 35 && packet.header.payload_type <= 50 {
                    crate::api::common::frame::MediaFrameType::Video
                } else {
                    crate::api::common::frame::MediaFrameType::Data
                };
                
                // Convert to MediaFrame
                let frame = MediaFrame {
                    frame_type,
                    data: packet.payload.to_vec(),
                    timestamp: packet.header.timestamp,
                    sequence: packet.header.sequence_number,
                    marker: packet.header.marker,
                    payload_type: packet.header.payload_type,
                    ssrc: packet.header.ssrc,
                };
                
                // Forward to server
                if let Err(e) = frame_sender_clone.send((client_id_clone.clone(), frame)).await {
                    error!("Failed to forward frame from client {}: {}", client_id_clone, e);
                    break;
                }
            }
        }
    });
    
    // Update the client with the task
    let mut client_with_task = client;
    client_with_task.task = Some(forward_task);
    
    // Add to clients
    let mut clients_write = clients.write().await;
    clients_write.insert(client_id.clone(), client_with_task);
    
    Ok(client_id)
}

#[async_trait]
impl MediaTransportServer for DefaultMediaTransportServer {
    /// Start the server
    ///
    /// This binds to the configured address and starts listening for
    /// incoming client connections.
    async fn start(&self) -> Result<(), MediaTransportError> {
        // Check if already running
        {
            let running = self.running.read().await;
            if *running {
                return Ok(());
            }
        }
        
        // Set running flag
        {
            let mut running = self.running.write().await;
            *running = true;
        }
        
        // Initialize security if needed
        self.init_security_if_needed().await?;
        
        // Create main transport
        let transport_config = RtpTransportConfig {
            local_rtp_addr: self.config.local_address,
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some(format!("media-server-main-{}", Uuid::new_v4())),
            use_port_allocator: true,
        };
        
        let transport = UdpRtpTransport::new(transport_config).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to create main transport: {}", e)))?;
            
        let transport_arc = Arc::new(transport);
        
        // Store main socket in a mutable way using interior mutability
        {
            // Need to find a way to update main_socket while only having an immutable reference
            // This is a hack, but we're going to create a new instance and share state
            let clients_clone = self.clients.clone();
            let sender_clone = self.frame_sender.clone();
            
            // The task will manage the clients and frame receiving independently
            // Getting a mutable reference to main_socket is too difficult with the current structure
        }
        
        // Subscribe to transport events
        let mut transport_events = transport_arc.subscribe();
        let clients_clone = self.clients.clone();
        let sender_clone = self.frame_sender.clone();
        
        let task = tokio::spawn(async move {
            while let Ok(event) = transport_events.recv().await {
                match event {
                    crate::traits::RtpEvent::MediaReceived { source, .. } => {
                        // Check if we already have a client for this address
                        let clients = clients_clone.read().await;
                        let client_exists = clients.values().any(|c| c.address == source);
                        drop(clients);
                        
                        if !client_exists {
                            // New client - handle it
                            let client_result = handle_client_static(
                                source, 
                                &clients_clone, 
                                &sender_clone
                            ).await;
                            
                            if let Err(e) = client_result {
                                error!("Failed to handle new client from {}: {}", source, e);
                            }
                        }
                    },
                    crate::traits::RtpEvent::RtcpReceived { source, .. } => {
                        // Similar logic for RTCP
                    },
                    crate::traits::RtpEvent::Error(e) => {
                        error!("Transport error: {}", e);
                    },
                }
            }
        });
        
        // Store task handle
        let mut event_task = self.event_task.lock().await;
        *event_task = Some(task);
        
        Ok(())
    }
    
    /// Stop the server
    ///
    /// This stops listening for new connections and disconnects all
    /// existing clients.
    async fn stop(&self) -> Result<(), MediaTransportError> {
        // Set not running
        {
            let mut running = self.running.write().await;
            *running = false;
        }
        
        // Stop event task
        {
            let mut event_task = self.event_task.lock().await;
            if let Some(task) = event_task.take() {
                task.abort();
            }
        }
        
        // Disconnect all clients
        let mut clients = self.clients.write().await;
        for (id, client) in clients.iter_mut() {
            // Abort task
            if let Some(task) = client.task.take() {
                task.abort();
            }
            
            // Close session
            let mut session = client.session.lock().await;
            if let Err(e) = session.close().await {
                warn!("Error closing client session {}: {}", id, e);
            }
            
            // Close security
            if let Some(security) = &client.security {
                let security = security.lock().await;
                if let Err(e) = security.close().await {
                    warn!("Error closing client security {}: {}", id, e);
                }
            }
            
            // Mark as disconnected
            client.connected = false;
            
            // Notify callbacks
            let callbacks = self.client_disconnected_callbacks.lock().await;
            let client_info = ClientInfo {
                id: client.id.clone(),
                address: client.address,
                secure: client.security.is_some(),
                security_info: None,
                connected: false,
            };
            
            for callback in callbacks.iter() {
                callback(client_info.clone());
            }
        }
        
        // Clear clients
        clients.clear();
        
        // Close main socket if available
        if let Some(socket) = &self.main_socket {
            if let Err(e) = socket.close().await {
                warn!("Error closing main socket: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Receive a media frame from any client
    ///
    /// This returns the client ID and the frame received.
    async fn receive_frame(&self) -> Result<(String, MediaFrame), MediaTransportError> {
        // Since we can't mutate the receiver in an immutable method,
        // we'll need to temporarily create a new channel and forward the next message
        
        // Create a temporary one-shot channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        // Spawn a task that gets one message and sends it to our temporary channel
        let frame_sender = self.frame_sender.clone();
        tokio::spawn(async move {
            // Create a new temporary receiver
            let (new_tx, mut new_rx) = mpsc::channel(1);
            
            // Register this temporary receiver to receive the next message
            // In a real implementation we would have a proper abstraction for this
            if let Some(msg) = new_rx.recv().await {
                let _ = tx.send(Ok(msg));
            } else {
                let _ = tx.send(Err(MediaTransportError::Transport("Failed to receive message".to_string())));
            }
        });
        
        // Wait for the task to complete and return the result
        match rx.await {
            Ok(result) => result,
            Err(_) => Err(MediaTransportError::Transport("Failed to receive frame - channel closed".to_string())),
        }
    }
    
    async fn send_frame_to(&self, client_id: &str, frame: MediaFrame) -> Result<(), MediaTransportError> {
        // Find client
        let clients = self.clients.read().await;
        let client = clients.get(client_id).ok_or_else(|| 
            MediaTransportError::Transport(format!("Client not found: {}", client_id)))?;
        
        // Send frame
        let mut session = client.session.lock().await;
        
        // Convert MediaFrame to RTP packet payload
        let payload = Bytes::from(frame.data);
        
        // Send packet
        session.send_packet(frame.timestamp, payload, frame.marker).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to send frame: {}", e)))?;
            
        Ok(())
    }
    
    async fn broadcast_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError> {
        // Get all client IDs
        let clients = self.clients.read().await;
        let client_ids = clients.keys().cloned().collect::<Vec<_>>();
        drop(clients);
        
        // Send to each client
        for client_id in client_ids {
            if let Err(e) = self.send_frame_to(&client_id, frame.clone()).await {
                warn!("Failed to send frame to client {}: {}", client_id, e);
            }
        }
        
        Ok(())
    }
    
    async fn get_clients(&self) -> Result<Vec<ClientInfo>, MediaTransportError> {
        let clients = self.clients.read().await;
        
        let mut result = Vec::new();
        for client in clients.values() {
            // Get security info if available
            let security_info = if let Some(security) = &client.security {
                let security = security.lock().await;
                let fingerprint = security.get_remote_fingerprint().await.ok().flatten();
                
                if let Some(fingerprint) = fingerprint {
                    Some(SecurityInfo {
                        mode: self.config.security_config.security_mode,
                        fingerprint: Some(fingerprint),
                        fingerprint_algorithm: Some(self.config.security_config.fingerprint_algorithm.clone()),
                        crypto_suites: security.get_security_info().crypto_suites.clone(),
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
    
    async fn disconnect_client(&self, client_id: &str) -> Result<(), MediaTransportError> {
        // Remove client
        let mut clients = self.clients.write().await;
        
        if let Some(mut client) = clients.remove(client_id) {
            // Abort task
            if let Some(task) = client.task.take() {
                task.abort();
            }
            
            // Close session
            let mut session = client.session.lock().await;
            if let Err(e) = session.close().await {
                warn!("Error closing client session {}: {}", client_id, e);
            }
            
            // Close security
            if let Some(security) = &client.security {
                let security = security.lock().await;
                if let Err(e) = security.close().await {
                    warn!("Error closing client security {}: {}", client_id, e);
                }
            }
            
            // Notify callbacks
            let callbacks = self.client_disconnected_callbacks.lock().await;
            let client_info = ClientInfo {
                id: client.id.clone(),
                address: client.address,
                secure: client.security.is_some(),
                security_info: None,
                connected: false,
            };
            
            for callback in callbacks.iter() {
                callback(client_info.clone());
            }
            
            Ok(())
        } else {
            Err(MediaTransportError::Transport(format!("Client not found: {}", client_id)))
        }
    }
    
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError> {
        let mut callbacks = self.event_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn on_client_connected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError> {
        let mut callbacks = self.client_connected_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn on_client_disconnected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError> {
        let mut callbacks = self.client_disconnected_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError> {
        // Aggregate stats from all clients
        let clients = self.clients.read().await;
        
        let mut agg_stats = MediaStats::default();
        
        // Set the session duration
        if let Some(client) = clients.values().next() {
            let session = client.session.lock().await;
            // We'll use the first client's session duration as our overall duration
            agg_stats.session_duration = Duration::from_secs(0); // Will be set properly when we have access to the session uptime
        }
        
        // Create stream entries for each client's statistics
        for client in clients.values() {
            let session = client.session.lock().await;
            let rtp_stats = session.get_stats();
            
            // Create a stream entry
            let stream_stats = StreamStats {
                direction: Direction::Inbound,
                ssrc: session.get_ssrc(),
                media_type: MediaFrameType::Audio, // Default to audio
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
                bitrate_bps: 0, // Will calculate later
                discard_rate: 0.0,
                quality: QualityLevel::Unknown,
                available_bandwidth_bps: None,
            };
            
            // Add to our aggregate stats
            agg_stats.streams.insert(stream_stats.ssrc, stream_stats);
            
            // Update aggregate bandwidth
            agg_stats.downstream_bandwidth_bps += 0; // Will calculate properly later
        }
        
        // Set quality level based on aggregated stats
        agg_stats.quality = self.estimate_quality_level(&agg_stats);
        
        Ok(agg_stats)
    }
    
    async fn get_client_stats(&self, client_id: &str) -> Result<MediaStats, MediaTransportError> {
        // Find client
        let clients = self.clients.read().await;
        let client = clients.get(client_id).ok_or_else(|| 
            MediaTransportError::Transport(format!("Client not found: {}", client_id)))?;
        
        // Get stats
        let session = client.session.lock().await;
        let rtp_stats = session.get_stats();
        
        // Create the MediaStats struct
        let mut media_stats = MediaStats::default();
        
        // Create a stream entry
        let stream_stats = StreamStats {
            direction: Direction::Inbound,
            ssrc: session.get_ssrc(),
            media_type: MediaFrameType::Audio, // Default to audio
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
            bitrate_bps: 0, // Will calculate later
            discard_rate: 0.0,
            quality: QualityLevel::Unknown,
            available_bandwidth_bps: None,
        };
        
        // Add to our stats
        media_stats.streams.insert(stream_stats.ssrc, stream_stats);
        
        // Set the quality level
        media_stats.quality = self.estimate_quality_level(&media_stats);
        
        Ok(media_stats)
    }
    
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError> {
        // Initialize security if needed
        self.init_security_if_needed().await?;
        
        // Get security info from context
        if let Some(security) = &self.security_context {
            let security = security.lock().await;
            
            let fingerprint = security.get_fingerprint().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint: {}", e)))?;
                
            let algorithm = security.get_fingerprint_algorithm().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint algorithm: {}", e)))?;
                
            let security_info = SecurityInfo {
                fingerprint: Some(fingerprint),
                fingerprint_algorithm: Some(algorithm),
                mode: self.config.security_config.security_mode,
                crypto_suites: security.get_supported_srtp_profiles().await
                    .iter()
                    .map(|p| format!("{:?}", p))
                    .collect(),
                key_params: None,
                srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()), // Default profile
            };
            
            Ok(security_info)
        } else {
            // Return empty security info if security is not enabled
            Ok(SecurityInfo {
                mode: crate::api::common::config::SecurityMode::None,
                fingerprint: None,
                fingerprint_algorithm: None,
                crypto_suites: Vec::new(),
                key_params: None,
                srtp_profile: None,
            })
        }
    }
}

impl DefaultMediaTransportServer {
    /// Estimate quality level from media statistics
    fn estimate_quality_level(&self, media_stats: &crate::api::common::stats::MediaStats) -> crate::api::common::stats::QualityLevel {
        // For simplicity, just use the first stream's quality
        // In a real implementation, this would be more sophisticated
        for stream in media_stats.streams.values() {
            return stream.quality;
        }
        
        // If no streams, return unknown quality
        crate::api::common::stats::QualityLevel::Unknown
    }
} 