//! Server transport implementation
//!
//! This file contains the implementation of the MediaTransportServer trait.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;
use std::time::Duration;
use std::time::SystemTime;
use std::any::Any;

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
    /// Security context for this client - using Any to avoid casting issues
    security: Option<Arc<Mutex<Box<dyn std::any::Any + Send + Sync>>>>,
    /// Task handle for packet forwarding
    task: Option<JoinHandle<()>>,
    /// Is connected
    connected: bool,
    /// Creation time
    created_at: SystemTime,
    /// Last activity time
    last_activity: Arc<Mutex<SystemTime>>,
}

/// Default implementation of the MediaTransportServer
pub struct DefaultMediaTransportServer {
    /// Server configuration
    config: ServerConfig,
    /// Server security context for DTLS - using Any to avoid casting issues
    security_context: Arc<RwLock<Option<Arc<Mutex<Box<dyn std::any::Any + Send + Sync>>>>>>,
    /// Connected clients
    clients: Arc<RwLock<HashMap<String, ClientConnection>>>,
    /// Sender for frames from clients (broadcast channel)
    frame_sender: broadcast::Sender<(String, MediaFrame)>,
    /// Event callbacks
    event_callbacks: Arc<Mutex<Vec<MediaEventCallback>>>,
    /// Client connected callbacks
    client_connected_callbacks: Arc<Mutex<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    /// Client disconnected callbacks
    client_disconnected_callbacks: Arc<Mutex<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    /// Main socket for receiving connections (wrapped in RwLock for thread-safe interior mutability)
    main_socket: Arc<RwLock<Option<Arc<UdpRtpTransport>>>>,
    /// Event handling task
    event_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    /// Server is running flag
    running: Arc<RwLock<bool>>,
}

// Custom implementation of Clone for DefaultMediaTransportServer
impl Clone for DefaultMediaTransportServer {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            security_context: self.security_context.clone(),
            clients: self.clients.clone(),
            frame_sender: self.frame_sender.clone(),
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
        // Create broadcast channel for frames from clients with capacity for 100 frames
        let (tx, _rx) = broadcast::channel(100);
        
        Ok(Self {
            config,
            security_context: Arc::new(RwLock::new(None)),
            clients: Arc::new(RwLock::new(HashMap::new())),
            frame_sender: tx,
            event_callbacks: Arc::new(Mutex::new(Vec::new())),
            client_connected_callbacks: Arc::new(Mutex::new(Vec::new())),
            client_disconnected_callbacks: Arc::new(Mutex::new(Vec::new())),
            main_socket: Arc::new(RwLock::new(None)),
            event_task: Arc::new(Mutex::new(None)),
            running: Arc::new(RwLock::new(false)),
        })
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
            // Check if we have a server security context
            let server_ctx_option = self.security_context.read().await;
            
            if let Some(server_ctx) = server_ctx_option.as_ref() {
                // Lock the server context
                let server_ctx = server_ctx.lock().await;
                
                // Downcast to ServerSecurityContext trait
                let server_ctx = server_ctx.downcast_ref::<Arc<dyn crate::api::server::security::ServerSecurityContext>>()
                    .ok_or_else(|| MediaTransportError::Security("Failed to downcast server security context".to_string()))?;
                
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
                return Err(MediaTransportError::Security("Server security context not initialized".to_string()));
            }
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
                    
                    // Forward to server via broadcast channel
                    // We can ignore send errors since they just mean no receivers are listening
                    if let Err(e) = frame_sender.send((client_id_clone.clone(), frame)) {
                        // Only log this as a warning, not an error - it's expected if no subscribers
                        debug!("No receivers for frame from client {}: {}", client_id_clone, e);
                    }
                }
            }
        });
        
        // Create client connection
        let client = ClientConnection {
            id: client_id.clone(),
            address: addr,
            session: rtp_session,
            security: security_ctx.map(|ctx| Arc::new(Mutex::new(Box::new(ctx) as Box<dyn std::any::Any + Send + Sync>))),
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
        if self.config.security_config.security_mode.is_enabled() {
            // Check if we already have a security context
            let security_exists = {
                let context = self.security_context.read().await;
                context.is_some()
            };

            if !security_exists {
                // We need to create a new security context
                // Create server security context
                let security_context = DefaultServerSecurityContext::new(
                    self.config.security_config.clone(),
                ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create server security context: {}", e)))?;
                
                // Store context
                let mut context_write = self.security_context.write().await;
                *context_write = Some(Arc::new(Mutex::new(Box::new(security_context) as Box<dyn std::any::Any + Send + Sync>)));
            }
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
    frame_sender: &broadcast::Sender<(String, MediaFrame)>
) -> Result<String, MediaTransportError> {
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
                    let frame_type = if packet.header.payload_type <= 34 {
                        crate::api::common::frame::MediaFrameType::Audio
                    } else if packet.header.payload_type >= 35 && packet.header.payload_type <= 50 {
                        crate::api::common::frame::MediaFrameType::Video
                    } else {
                        crate::api::common::frame::MediaFrameType::Data
                    };
                    
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
            use_port_allocator: false, // Changed from true to false to use exact specified port
        };
        
        debug!("Creating main transport with config: {:?}", transport_config);
        
        let transport = UdpRtpTransport::new(transport_config).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to create main transport: {}", e)))?;
        
        // Log the actual bound address
        let actual_addr = transport.local_rtp_addr()
            .map_err(|e| MediaTransportError::Transport(format!("Failed to get local RTP address: {}", e)))?;
            
        info!("Server actually bound to: {}", actual_addr);
            
        let transport_arc = Arc::new(transport);
        
        // Store the socket in a thread-safe way using RwLock
        {
            let mut main_socket = self.main_socket.write().await;
            *main_socket = Some(transport_arc.clone());
        }
        
        // Subscribe to transport events
        let mut transport_events = transport_arc.subscribe();
        let clients_clone = self.clients.clone();
        let sender_clone = self.frame_sender.clone();
        
        let task = tokio::spawn(async move {
            debug!("Started transport event task");
            while let Ok(event) = transport_events.recv().await {
                match event {
                    crate::traits::RtpEvent::MediaReceived { source, payload, payload_type, timestamp, marker } => {
                        // Debug output to help diagnose issues
                        debug!("RtpEvent::MediaReceived from {}", source);
                        
                        // Direct handling of received frames:
                        // 1. Convert to MediaFrame
                        let frame_type = if payload_type <= 34 {
                            crate::api::common::frame::MediaFrameType::Audio
                        } else if payload_type >= 35 && payload_type <= 50 {
                            crate::api::common::frame::MediaFrameType::Video
                        } else {
                            crate::api::common::frame::MediaFrameType::Data
                        };
                        
                        // Generate a random sequence number since it's not provided in the event
                        // In a production scenario, we would track these per source
                        let sequence_number = rand::random::<u16>();
                        
                        // Use a random SSRC if not available from the event
                        // In a production scenario, we would track these per source
                        let ssrc = rand::random::<u32>();
                        
                        let frame = crate::api::common::frame::MediaFrame {
                            frame_type,
                            data: payload.to_vec(),
                            timestamp,
                            sequence: sequence_number,
                            marker,
                            payload_type,
                            ssrc,
                        };
                        
                        // Check if we already have a client for this address
                        let clients = clients_clone.read().await;
                        let client_exists = clients.values().any(|c| c.address == source);
                        let client_id = if client_exists {
                            // Find the client ID for this address
                            clients.values()
                                .find(|c| c.address == source)
                                .map(|c| c.id.clone())
                                .unwrap_or_else(|| format!("unknown-{}", source))
                        } else {
                            // No client ID yet
                            format!("pending-{}", source)
                        };
                        drop(clients);
                        
                        // 2. Forward directly to broadcast channel
                        // This ensures frames are available immediately via the receive_frame method
                        match sender_clone.send((client_id.clone(), frame)) {
                            Ok(receivers) => {
                                debug!("Directly forwarded frame to {} receivers for client {} (seq={})", 
                                        receivers, client_id, sequence_number);
                            },
                            Err(e) => {
                                debug!("No receivers for direct frame forwarding: {}", e);
                            }
                        }
                        
                        // 3. Handle client creation if new
                        if !client_exists {
                            // New client - handle it
                            debug!("New client detected at {}, handling...", source);
                            let client_result = handle_client_static(
                                source, 
                                &clients_clone, 
                                &sender_clone
                            ).await;
                            
                            match client_result {
                                Ok(client_id) => debug!("Successfully handled new client {} from {}", client_id, source),
                                Err(e) => error!("Failed to handle new client from {}: {}", source, e),
                            }
                        } else {
                            debug!("Existing client from {}, no new client creation needed", source);
                        }
                    },
                    crate::traits::RtpEvent::RtcpReceived { source, .. } => {
                        debug!("RtpEvent::RtcpReceived from {}", source);
                    },
                    crate::traits::RtpEvent::Error(e) => {
                        error!("Transport error: {}", e);
                    },
                }
            }
            debug!("Transport event task ended");
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
        // Check if we're running
        {
            let running = self.running.read().await;
            if !*running {
                debug!("Server already stopped, no action needed");
                return Ok(());
            }
        }

        // Set not running
        {
            let mut running = self.running.write().await;
            *running = false;
        }
        
        debug!("Stopping server - aborting event task");
        
        // Stop event task first to prevent new client connections
        {
            let mut event_task = self.event_task.lock().await;
            if let Some(task) = event_task.take() {
                debug!("Aborting main event task");
                task.abort();
                // Try to wait for the task to finish (with timeout)
                match tokio::time::timeout(Duration::from_millis(100), task).await {
                    Ok(_) => debug!("Event task terminated gracefully"),
                    Err(_) => debug!("Event task termination timed out"),
                }
            } else {
                debug!("No event task to abort");
            }
        }
        
        debug!("Disconnecting all clients");
        
        // Disconnect all clients
        let mut clients = self.clients.write().await;
        debug!("Disconnecting {} clients", clients.len());
        
        for (id, client) in clients.iter_mut() {
            debug!("Disconnecting client {}", id);
            
            // Abort task
            if let Some(task) = client.task.take() {
                debug!("Aborting client task for {}", id);
                task.abort();
                // Try to wait for the task to finish (with timeout)
                match tokio::time::timeout(Duration::from_millis(100), task).await {
                    Ok(_) => debug!("Client task for {} terminated gracefully", id),
                    Err(_) => debug!("Client task termination for {} timed out", id),
                }
            }
            
            // Close session
            debug!("Closing session for client {}", id);
            let mut session = client.session.lock().await;
            if let Err(e) = session.close().await {
                warn!("Error closing client session {}: {}", id, e);
            }
            drop(session);
            
            // Close security
            if let Some(security) = &client.security {
                debug!("Closing security for client {}", id);
                let security = security.lock().await;
                // Downcast to ClientSecurityContext trait
                if let Some(security_ctx) = security.downcast_ref::<Arc<dyn crate::api::server::security::ClientSecurityContext>>() {
                    if let Err(e) = security_ctx.close().await {
                        warn!("Error closing client security {}: {}", id, e);
                    }
                } else {
                    warn!("Failed to downcast client security context for {}", id);
                }
            }
            
            // Mark as disconnected
            client.connected = false;
            
            // Notify callbacks
            debug!("Notifying disconnection callbacks for client {}", id);
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
        debug!("Clearing client list");
        clients.clear();
        drop(clients);
        
        // Close main socket if available
        debug!("Closing main socket");
        let mut main_socket = self.main_socket.write().await;
        if let Some(socket) = main_socket.take() {
            debug!("Closing main transport socket");
            if let Err(e) = socket.close().await {
                warn!("Error closing main socket: {}", e);
            }
        } else {
            debug!("No main socket to close");
        }
        
        // Ensure we release broadcast channel resources
        debug!("Ensuring broadcast channel resources are released");
        // Create a temporary receiver and then immediately drop it to avoid any lingering receivers
        {
            let _temp_receiver = self.frame_sender.subscribe();
            // Immediately drop the receiver
        }
        
        debug!("Server stopped successfully");
        Ok(())
    }
    
    /// Receive a media frame from any client
    ///
    /// This returns the client ID and the frame received.
    /// If no frame is available within timeout duration, returns a timeout error.
    async fn receive_frame(&self) -> Result<(String, MediaFrame), MediaTransportError> {
        // Create a new receiver from the broadcast channel
        let mut receiver = self.frame_sender.subscribe();
        
        // Wait for a frame with a shorter timeout (500ms instead of 2s)
        match tokio::time::timeout(Duration::from_millis(500), receiver.recv()).await {
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
    
    /// Get the local address currently bound to
    /// 
    /// This returns the actual bound address of the transport, which may be different
    /// from the configured address if dynamic port allocation is used.
    async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError> {
        let main_socket = self.main_socket.read().await;
        if let Some(socket) = main_socket.as_ref() {
            match socket.local_rtp_addr() {
                Ok(addr) => Ok(addr),
                Err(e) => Err(MediaTransportError::Transport(format!("Failed to get local address: {}", e))),
            }
        } else {
            Err(MediaTransportError::Transport("No socket bound yet. Start server first.".to_string()))
        }
    }
    
    /// Send a media frame to a specific client
    ///
    /// If the client is not connected, this will return an error.
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
                
                // Downcast to ClientSecurityContext trait
                if let Some(security_ctx) = security.downcast_ref::<Arc<dyn crate::api::server::security::ClientSecurityContext>>() {
                    let fingerprint = security_ctx.get_remote_fingerprint().await.ok().flatten();
                    
                    if let Some(fingerprint) = fingerprint {
                        Some(SecurityInfo {
                            mode: self.config.security_config.security_mode,
                            fingerprint: Some(fingerprint),
                            fingerprint_algorithm: Some(self.config.security_config.fingerprint_algorithm.clone()),
                            crypto_suites: security_ctx.get_security_info().crypto_suites.clone(),
                            key_params: None,
                            srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()), // Default profile
                        })
                    } else {
                        None
                    }
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
                // Downcast to ClientSecurityContext trait
                if let Some(security_ctx) = security.downcast_ref::<Arc<dyn crate::api::server::security::ClientSecurityContext>>() {
                    if let Err(e) = security_ctx.close().await {
                        warn!("Error closing client security {}: {}", client_id, e);
                    }
                } else {
                    warn!("Failed to downcast client security context for {}", client_id);
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
        
        // Get security context
        let security_context = self.security_context.read().await;
        
        if let Some(security_ctx) = security_context.as_ref() {
            // Lock the security context
            let security = security_ctx.lock().await;
            
            // Downcast to ServerSecurityContext trait
            let security_ctx = security.downcast_ref::<Arc<dyn crate::api::server::security::ServerSecurityContext>>()
                .ok_or_else(|| MediaTransportError::Security("Failed to downcast server security context".to_string()))?;
                
            // Get the fingerprint and algorithm
            let fingerprint = security_ctx.get_fingerprint().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint: {}", e)))?;
                
            let algorithm = security_ctx.get_fingerprint_algorithm().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to get fingerprint algorithm: {}", e)))?;
                
            // Get supported SRTP profiles
            let profiles = security_ctx.get_supported_srtp_profiles().await;
            
            // Create crypto suites list from profiles
            let crypto_suites = profiles.iter()
                .map(|p| match p {
                    crate::api::common::config::SrtpProfile::AesCm128HmacSha1_80 => "AES_CM_128_HMAC_SHA1_80",
                    crate::api::common::config::SrtpProfile::AesCm128HmacSha1_32 => "AES_CM_128_HMAC_SHA1_32",
                    crate::api::common::config::SrtpProfile::AesGcm128 => "AEAD_AES_128_GCM",
                    crate::api::common::config::SrtpProfile::AesGcm256 => "AEAD_AES_256_GCM",
                })
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
                
            // Create security info
            Ok(SecurityInfo {
                mode: self.config.security_config.security_mode,
                fingerprint: Some(fingerprint),
                fingerprint_algorithm: Some(algorithm),
                crypto_suites,
                key_params: None,
                srtp_profile: Some("AES_CM_128_HMAC_SHA1_80".to_string()), // Default profile
            })
        } else {
            Err(MediaTransportError::Security("Security context not initialized".to_string()))
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