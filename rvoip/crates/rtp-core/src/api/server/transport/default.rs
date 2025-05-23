//! Default implementation for the server transport
//!
//! This file contains the implementation of the MediaTransportServer trait
//! through the DefaultMediaTransportServer struct.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use std::any::Any;

use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::{mpsc, Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::{MediaEventCallback, MediaTransportEvent};
use crate::api::common::stats::{MediaStats, QualityLevel, StreamStats, Direction};
use crate::api::common::config::{SecurityInfo, SecurityMode};
use crate::api::common::frame::MediaFrameType;
use crate::api::common::extension::ExtensionFormat;
use crate::api::server::transport::{MediaTransportServer, ClientInfo, HeaderExtension};
use crate::api::server::security::{ServerSecurityContext, ClientSecurityContext, DefaultServerSecurityContext};
use crate::api::server::config::ServerConfig;
use crate::api::client::transport::{RtcpStats, VoipMetrics};
use crate::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
use crate::transport::{RtpTransportConfig, UdpRtpTransport, RtpTransport, SecurityRtpTransport};
use crate::{CsrcManager, CsrcMapping, RtpSsrc, RtpCsrc, MAX_CSRC_COUNT};
use crate::srtp::{SrtpContext, SrtpCryptoSuite, SRTP_AES128_CM_SHA1_80};
use crate::srtp::crypto::SrtpCryptoKey;

use super::core::{ClientConnection, handle_client, handle_client_static};

/// Default implementation of the MediaTransportServer
pub struct DefaultMediaTransportServer {
    /// Session identifier
    id: String,
    
    /// Configuration
    config: ServerConfig,
    
    /// Whether the server is running
    running: Arc<RwLock<bool>>,
    
    /// Main socket for the server
    main_socket: Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    
    /// Socket
    socket: Arc<RwLock<Option<UdpSocket>>>,
    
    /// Main transport
    transport: Arc<RwLock<Option<Arc<dyn RtpTransport>>>>,
    
    /// Client sockets
    client_sockets: Arc<RwLock<HashMap<String, UdpSocket>>>,
    
    /// Client transports
    client_transports: Arc<RwLock<HashMap<String, Arc<dyn RtpTransport>>>>,
    
    /// Security context for DTLS/SRTP
    security_context: Arc<RwLock<Option<Arc<dyn ServerSecurityContext + Send + Sync>>>>,
    
    /// Client connections
    clients: Arc<RwLock<HashMap<String, ClientConnection>>>,
    
    /// Transport event callbacks
    event_callbacks: Arc<RwLock<Vec<MediaEventCallback>>>,
    
    /// Client connected callbacks
    client_connected_callbacks: Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    
    /// Client disconnected callbacks
    client_disconnected_callbacks: Arc<RwLock<Vec<Box<dyn Fn(ClientInfo) + Send + Sync>>>>,
    
    /// SSRC demultiplexing enabled flag
    ssrc_demultiplexing_enabled: Arc<RwLock<bool>>,
    
    /// CSRC management status
    csrc_management_enabled: Arc<RwLock<bool>>,
    
    /// CSRC manager
    csrc_manager: Arc<RwLock<CsrcManager>>,
    
    /// Header extension format
    header_extension_format: Arc<RwLock<ExtensionFormat>>,
    
    /// Header extensions enabled flag
    header_extensions_enabled: Arc<RwLock<bool>>,
    
    /// Header extension ID-to-URI mappings
    header_extension_mappings: Arc<RwLock<HashMap<u8, String>>>,
    
    /// Pending header extensions to be added to outgoing packets
    pending_extensions: Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    
    /// Header extensions received from clients
    received_extensions: Arc<RwLock<HashMap<String, Vec<HeaderExtension>>>>,
    
    /// Frame sender for broadcast
    frame_sender: broadcast::Sender<(String, MediaFrame)>,
    
    /// Event handling task
    event_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

// Custom implementation of Clone for DefaultMediaTransportServer
impl Clone for DefaultMediaTransportServer {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            config: self.config.clone(),
            running: self.running.clone(),
            main_socket: self.main_socket.clone(),
            socket: self.socket.clone(),
            transport: self.transport.clone(),
            client_sockets: self.client_sockets.clone(),
            client_transports: self.client_transports.clone(),
            security_context: self.security_context.clone(),
            clients: self.clients.clone(),
            event_callbacks: self.event_callbacks.clone(),
            client_connected_callbacks: self.client_connected_callbacks.clone(),
            client_disconnected_callbacks: self.client_disconnected_callbacks.clone(),
            ssrc_demultiplexing_enabled: self.ssrc_demultiplexing_enabled.clone(),
            csrc_management_enabled: self.csrc_management_enabled.clone(),
            csrc_manager: self.csrc_manager.clone(),
            header_extension_format: self.header_extension_format.clone(),
            header_extensions_enabled: self.header_extensions_enabled.clone(),
            header_extension_mappings: self.header_extension_mappings.clone(),
            pending_extensions: self.pending_extensions.clone(),
            received_extensions: self.received_extensions.clone(),
            frame_sender: self.frame_sender.clone(),
            event_task: self.event_task.clone(),
        }
    }
}

impl DefaultMediaTransportServer {
    /// Create a new DefaultMediaTransportServer
    pub async fn new(
        config: ServerConfig,
    ) -> Result<Self, MediaTransportError> {
        // Extract configuration values
        let csrc_management_enabled = config.csrc_management_enabled;
        let header_extensions_enabled = config.header_extensions_enabled;
        let header_extension_format = config.header_extension_format;
        let ssrc_demultiplexing_enabled = false; // Disabled by default
        
        // Create broadcast channel for frames with buffer size 16
        let (sender, _) = broadcast::channel(16);
        
        Ok(Self {
            id: format!("media-server-main-{}", Uuid::new_v4()),
            config,
            running: Arc::new(RwLock::new(false)),
            main_socket: Arc::new(RwLock::new(None)),
            socket: Arc::new(RwLock::new(None)),
            transport: Arc::new(RwLock::new(None)),
            client_sockets: Arc::new(RwLock::new(HashMap::new())),
            client_transports: Arc::new(RwLock::new(HashMap::new())),
            security_context: Arc::new(RwLock::new(None)),
            clients: Arc::new(RwLock::new(HashMap::new())),
            event_callbacks: Arc::new(RwLock::new(Vec::new())),
            client_connected_callbacks: Arc::new(RwLock::new(Vec::new())),
            client_disconnected_callbacks: Arc::new(RwLock::new(Vec::new())),
            ssrc_demultiplexing_enabled: Arc::new(RwLock::new(false)),
            csrc_management_enabled: Arc::new(RwLock::new(csrc_management_enabled)),
            csrc_manager: Arc::new(RwLock::new(CsrcManager::new())),
            header_extension_format: Arc::new(RwLock::new(header_extension_format)),
            header_extensions_enabled: Arc::new(RwLock::new(header_extensions_enabled)),
            header_extension_mappings: Arc::new(RwLock::new(HashMap::new())),
            pending_extensions: Arc::new(RwLock::new(HashMap::new())),
            received_extensions: Arc::new(RwLock::new(HashMap::new())),
            frame_sender: sender,
            event_task: Arc::new(Mutex::new(None)),
        })
    }

    /// Get the frame type based on payload type
    pub fn get_frame_type_from_payload_type(&self, payload_type: u8) -> crate::api::common::frame::MediaFrameType {
        super::util::get_frame_type_from_payload_type(payload_type)
    }

    /// Create a new instance of ServerMetrics
    pub async fn get_server_metrics(&self, server_start_time: std::time::Duration) -> Result<super::stats::ServerMetrics, MediaTransportError> {
        // Get current media stats
        let media_stats = self.get_stats().await?;
        
        // Create server metrics
        super::stats::get_server_metrics(
            &self.clients,
            &media_stats,
            server_start_time
        ).await
    }
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
        super::security::init_security_if_needed(
            &self.config, 
            &self.security_context
        ).await?;
        
        // Create main transport
        let transport_config = RtpTransportConfig {
            local_rtp_addr: self.config.local_address,
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: self.config.rtcp_mux,
            session_id: Some(format!("media-server-main-{}", Uuid::new_v4())),
            use_port_allocator: false, // Changed from true to false to use exact specified port
        };
        
        debug!("Creating main transport with config: {:?}", transport_config);
        
        // Create base UDP transport
        let udp_transport = UdpRtpTransport::new(transport_config).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to create UDP transport: {}", e)))?;
        
        // Create SecurityRtpTransport wrapper
        let transport = SecurityRtpTransport::new(Arc::new(udp_transport), true).await
            .map_err(|e| MediaTransportError::Transport(format!("Failed to create SecurityRtpTransport: {}", e)))?;
        
        // Note: We don't start the UDP receiver here since SecurityRtpTransport handles packet reception
        
        let transport_arc: Arc<dyn RtpTransport> = Arc::new(transport);
        
        // Wire SRTP context if security is enabled
        let security_guard = self.security_context.read().await;
        if let Some(security_ctx) = security_guard.as_ref() {
            // For now, we'll extract the SRTP key directly from the config
            // In a full implementation, we'd access it through security_ctx.get_config()
            if self.config.security_config.security_mode == SecurityMode::Srtp {
                if let Some(srtp_key) = &self.config.security_config.srtp_key {
                    debug!("Configuring server SRTP context with pre-shared keys");
                    debug!("Creating server SRTP context from {} byte key", srtp_key.len());
                    
                    // Extract key and salt (first 16 bytes = key, next 14 bytes = salt)
                    if srtp_key.len() >= 30 {
                        let key = srtp_key[0..16].to_vec();
                        let salt = srtp_key[16..30].to_vec();
                        
                        debug!("Creating server SrtpCryptoKey with key: {} bytes, salt: {} bytes", key.len(), salt.len());
                        let crypto_key = SrtpCryptoKey::new(key, salt);
                        
                        match SrtpContext::new(SRTP_AES128_CM_SHA1_80, crypto_key) {
                            Ok(srtp_context) => {
                                debug!("Successfully created server SRTP context");
                                
                                // Set SRTP context on security transport
                                if let Some(sec_transport) = transport_arc.as_any()
                                    .downcast_ref::<SecurityRtpTransport>() {
                                    sec_transport.set_srtp_context(srtp_context).await;
                                    info!("SRTP context successfully configured on server transport");
                                } else {
                                    warn!("Failed to downcast to SecurityRtpTransport for server SRTP context setting");
                                }
                            },
                            Err(e) => {
                                error!("Failed to create server SRTP context: {}", e);
                                return Err(MediaTransportError::Security(format!("Server SRTP context creation failed: {}", e)));
                            }
                        }
                    } else {
                        error!("Server SRTP key too short: {} bytes (expected at least 30)", srtp_key.len());
                        return Err(MediaTransportError::Security("Server SRTP key too short".to_string()));
                    }
                } else {
                    warn!("SRTP mode enabled on server but no key provided");
                }
            }
        }
        drop(security_guard);
        
        // Store the socket in a thread-safe way using RwLock
        {
            let mut main_socket = self.main_socket.write().await;
            *main_socket = Some(transport_arc.clone());
        }
        
        // Subscribe to transport events
        let mut transport_events = transport_arc.subscribe();
        let clients_clone = self.clients.clone();
        let sender_clone = self.frame_sender.clone();
        let ssrc_demux_enabled_clone = self.ssrc_demultiplexing_enabled.clone();
        
        let task = tokio::spawn(async move {
            debug!("Started transport event task");
            while let Ok(event) = transport_events.recv().await {
                match event {
                    crate::traits::RtpEvent::MediaReceived { source, payload, payload_type, timestamp, marker } => {
                        // Debug output to help diagnose issues
                        debug!("RtpEvent::MediaReceived from {} - payload size={}", source, payload.len());
                        
                        // Smart detection: if the payload is small and looks like text/media data rather than RTP packet,
                        // it's likely already processed by SecurityRtpTransport
                        let is_processed_payload = payload.len() < 100 && 
                            (payload.len() < 12 || // Too small for RTP header
                             (payload.len() >= 1 && payload[0] != 0x80)); // First byte doesn't look like RTP version 2
                        
                        let (frame, ssrc) = if is_processed_payload {
                            // This is already processed payload from SecurityRtpTransport - don't re-parse
                            debug!("Detected processed payload from SecurityRtpTransport - using provided RTP info");
                            
                            // Use the RTP header information already provided in the event
                            let frame = crate::api::common::frame::MediaFrame {
                                frame_type: if payload_type <= 34 {
                                    crate::api::common::frame::MediaFrameType::Audio
                                } else if payload_type >= 35 && payload_type <= 50 {
                                    crate::api::common::frame::MediaFrameType::Video
                                } else {
                                    crate::api::common::frame::MediaFrameType::Data
                                },
                                data: payload.to_vec(),
                                timestamp,
                                sequence: 0, // SecurityRtpTransport doesn't provide sequence in event
                                marker,
                                payload_type,
                                ssrc: 0, // Will be determined from client mapping
                                csrcs: Vec::new(),
                            };
                            
                            // Log payload data if it's text (helps with debugging)
                            if let Ok(text) = std::str::from_utf8(&payload) {
                                debug!("Processed payload (text): {}", text);
                            }
                            
                            // For processed payloads, we'll use the source address to determine SSRC
                            let temp_ssrc = rand::random::<u32>();
                            (frame, temp_ssrc)
                        } else {
                            // This looks like a raw RTP packet - parse it normally
                            debug!("Attempting to parse raw RTP packet from payload...");
                            let packet_result = crate::packet::RtpPacket::parse(&payload);
                            
                            match packet_result {
                                Ok(packet) => {
                                    // Successfully parsed the packet, use its header information
                                    debug!("Successfully parsed RTP packet with SSRC={:08x}, seq={}, ts={}, PT={}", 
                                           packet.header.ssrc, packet.header.sequence_number, 
                                           packet.header.timestamp, packet.header.payload_type);
                                    
                                    let frame = crate::api::common::frame::MediaFrame {
                                        frame_type: if packet.header.payload_type <= 34 {
                                            crate::api::common::frame::MediaFrameType::Audio
                                        } else if packet.header.payload_type >= 35 && packet.header.payload_type <= 50 {
                                            crate::api::common::frame::MediaFrameType::Video
                                        } else {
                                            crate::api::common::frame::MediaFrameType::Data
                                        },
                                        data: packet.payload.to_vec(),
                                        timestamp: packet.header.timestamp,
                                        sequence: packet.header.sequence_number,
                                        marker: packet.header.marker,
                                        payload_type: packet.header.payload_type,
                                        ssrc: packet.header.ssrc,
                                        csrcs: packet.header.csrc.clone(),
                                    };
                                    
                                    // Log payload data if it's text (helps with debugging)
                                    if let Ok(text) = std::str::from_utf8(&packet.payload) {
                                        debug!("Raw packet payload (text): {}", text);
                                    }
                                    
                                    (frame, packet.header.ssrc)
                                },
                                Err(e) => {
                                    // If we couldn't parse the packet, create a MediaFrame with the available info
                                    debug!("Couldn't parse RTP packet, using available info: {}", e);
                                    
                                    // Attempt to dump the first few bytes for debugging
                                    if payload.len() >= 12 {
                                        let mut header_bytes = [0u8; 12];
                                        header_bytes.copy_from_slice(&payload[0..12]);
                                        debug!("First 12 bytes of payload: {:?}", header_bytes);
                                    }
                                    
                                    // Generate a random sequence number and SSRC as fallback
                                    let sequence_number = rand::random::<u16>();
                                    let ssrc = rand::random::<u32>();
                                    
                                    let frame = crate::api::common::frame::MediaFrame {
                                        frame_type: if payload_type <= 34 {
                                            crate::api::common::frame::MediaFrameType::Audio
                                        } else if payload_type >= 35 && payload_type <= 50 {
                                            crate::api::common::frame::MediaFrameType::Video
                                        } else {
                                            crate::api::common::frame::MediaFrameType::Data
                                        },
                                        data: payload.to_vec(),
                                        timestamp,
                                        sequence: sequence_number,
                                        marker,
                                        payload_type,
                                        ssrc,
                                        csrcs: Vec::new(), // No CSRCs in fallback case
                                    };
                                    (frame, ssrc)
                                }
                            }
                        };
                        
                        // Check if SSRC demultiplexing is enabled
                        let ssrc_demux_enabled = *ssrc_demux_enabled_clone.read().await;
                        debug!("SSRC demultiplexing enabled: {}, handling packet with SSRC={:08x}", 
                               ssrc_demux_enabled, ssrc);
                        
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
                        
                        // Forward directly to broadcast channel
                        // This ensures frames are available immediately via the receive_frame method
                        match sender_clone.send((client_id.clone(), frame)) {
                            Ok(receivers) => {
                                debug!("Directly forwarded frame to {} receivers for client {} (SSRC={:08x})", 
                                        receivers, client_id, ssrc);
                            },
                            Err(e) => {
                                debug!("No receivers for direct frame forwarding: {}", e);
                            }
                        }
                        
                        // Handle client creation if new
                        if !client_exists {
                            // New client - handle it
                            debug!("New client detected at {}, handling...", source);
                            let client_result = super::core::handle_client_static(
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
            
            // Close security context if present
            if let Some(security_ctx) = &client.security {
                debug!("Closing security for client {}", id);
                if let Err(e) = security_ctx.close().await {
                    warn!("Error closing client security {}: {}", id, e);
                }
            }
            
            // Mark as disconnected
            client.connected = false;
            
            // Notify callbacks
            debug!("Notifying disconnection callbacks for client {}", id);
            let callbacks_guard = self.client_disconnected_callbacks.read().await;
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
            drop(callbacks_guard);
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
    async fn receive_frame(&self) -> Result<(String, MediaFrame), MediaTransportError> {
        super::core::receive_frame(&self.frame_sender).await
    }
    
    /// Get the local address currently bound to
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
    async fn send_frame_to(&self, client_id: &str, frame: MediaFrame) -> Result<(), MediaTransportError> {
        super::core::send_frame_to(
            client_id, 
            frame, 
            &self.clients,
            &self.ssrc_demultiplexing_enabled,
            &self.csrc_management_enabled,
            &self.csrc_manager,
            &self.main_socket,
        ).await
    }
    
    /// Broadcast a media frame to all connected clients
    async fn broadcast_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError> {
        super::core::broadcast_frame(
            frame, 
            &self.clients,
            &self.csrc_management_enabled,
            &self.csrc_manager,
            &self.main_socket,
        ).await
    }
    
    /// Get a list of connected clients
    async fn get_clients(&self) -> Result<Vec<ClientInfo>, MediaTransportError> {
        super::core::get_clients_info(
            &self.clients,
            &self.config,
        ).await
    }
    
    /// Disconnect a specific client
    async fn disconnect_client(&self, client_id: &str) -> Result<(), MediaTransportError> {
        super::core::disconnect_client(
            client_id,
            &self.clients,
            &self.client_disconnected_callbacks,
        ).await
    }
    
    /// Register a callback for transport events
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError> {
        super::core::on_event(
            callback,
            &self.event_callbacks,
        ).await
    }
    
    /// Register a callback for client connection events
    async fn on_client_connected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError> {
        super::core::on_client_connected(
            callback,
            &self.client_connected_callbacks,
        ).await
    }
    
    /// Register a callback for client disconnection events
    async fn on_client_disconnected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError> {
        super::core::on_client_disconnected(
            callback,
            &self.client_disconnected_callbacks,
        ).await
    }
    
    /// Get statistics for all clients
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError> {
        super::stats::get_stats(
            &self.clients
        ).await
    }
    
    /// Get statistics for a specific client
    async fn get_client_stats(&self, client_id: &str) -> Result<MediaStats, MediaTransportError> {
        super::stats::get_client_stats(
            client_id,
            &self.clients
        ).await
    }
    
    /// Get security information for SDP exchange
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError> {
        super::security::get_security_info(
            &self.config,
            &self.security_context,
        ).await
    }
    
    async fn send_rtcp_receiver_report(&self) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_receiver_report(
            &self.clients
        ).await
    }
    
    async fn send_rtcp_sender_report(&self) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_sender_report(
            &self.clients
        ).await
    }
    
    async fn send_rtcp_receiver_report_to_client(&self, client_id: &str) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_receiver_report_to_client(
            client_id,
            &self.clients
        ).await
    }
    
    async fn send_rtcp_sender_report_to_client(&self, client_id: &str) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_sender_report_to_client(
            client_id,
            &self.clients
        ).await
    }
    
    async fn get_rtcp_stats(&self) -> Result<RtcpStats, MediaTransportError> {
        super::rtcp::get_rtcp_stats(
            &self.clients
        ).await
    }
    
    async fn get_client_rtcp_stats(&self, client_id: &str) -> Result<RtcpStats, MediaTransportError> {
        super::rtcp::get_client_rtcp_stats(
            client_id,
            &self.clients
        ).await
    }
    
    async fn set_rtcp_interval(&self, interval: Duration) -> Result<(), MediaTransportError> {
        super::rtcp::set_rtcp_interval(
            &self.clients,
            interval
        ).await
    }
    
    async fn send_rtcp_app(&self, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_app(
            &self.clients,
            &self.main_socket,
            name,
            data
        ).await
    }
    
    async fn send_rtcp_app_to_client(&self, client_id: &str, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_app_to_client(
            client_id,
            &self.clients,
            &self.main_socket,
            name,
            data
        ).await
    }
    
    async fn send_rtcp_bye(&self, reason: Option<String>) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_bye(
            &self.clients,
            &self.main_socket,
            reason
        ).await
    }
    
    async fn send_rtcp_bye_to_client(&self, client_id: &str, reason: Option<String>) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_bye_to_client(
            client_id,
            &self.clients,
            &self.main_socket,
            reason
        ).await
    }
    
    async fn send_rtcp_xr_voip_metrics(&self, metrics: VoipMetrics) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_xr_voip_metrics(
            &self.clients,
            &self.main_socket,
            metrics
        ).await
    }
    
    async fn send_rtcp_xr_voip_metrics_to_client(&self, client_id: &str, metrics: VoipMetrics) -> Result<(), MediaTransportError> {
        super::rtcp::send_rtcp_xr_voip_metrics_to_client(
            client_id,
            &self.clients,
            &self.main_socket,
            metrics
        ).await
    }
    
    async fn is_csrc_management_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(*self.csrc_management_enabled.read().await)
    }
    
    async fn enable_csrc_management(&self) -> Result<bool, MediaTransportError> {
        // Check if already enabled
        if *self.csrc_management_enabled.read().await {
            return Ok(true);
        }
        
        // Set enabled flag
        *self.csrc_management_enabled.write().await = true;
        
        debug!("Enabled CSRC management on server");
        Ok(true)
    }
    
    async fn add_csrc_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaTransportError> {
        super::media::add_csrc_mapping(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            mapping
        ).await
    }
    
    async fn add_simple_csrc_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Result<(), MediaTransportError> {
        super::media::add_simple_csrc_mapping(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
            csrc
        ).await
    }
    
    async fn remove_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        super::media::remove_csrc_mapping_by_ssrc(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc
        ).await
    }
    
    async fn get_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        super::media::get_csrc_mapping_by_ssrc(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc
        ).await
    }
    
    async fn get_all_csrc_mappings(&self) -> Result<Vec<CsrcMapping>, MediaTransportError> {
        super::media::get_all_csrc_mappings(
            &self.csrc_management_enabled,
            &self.csrc_manager
        ).await
    }
    
    async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Result<Vec<RtpCsrc>, MediaTransportError> {
        super::media::get_active_csrcs(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            active_ssrcs
        ).await
    }
    
    async fn is_header_extensions_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(*self.header_extensions_enabled.read().await)
    }
    
    async fn enable_header_extensions(&self, format: ExtensionFormat) -> Result<bool, MediaTransportError> {
        // Check if already enabled
        if *self.header_extensions_enabled.read().await {
            return Ok(true);
        }
        
        // Set format and enabled flag
        *self.header_extension_format.write().await = format;
        *self.header_extensions_enabled.write().await = true;
        
        debug!("Enabled header extensions with format: {:?}", format);
        Ok(true)
    }
    
    async fn configure_header_extension(&self, id: u8, uri: String) -> Result<(), MediaTransportError> {
        super::media::configure_header_extension(
            &self.header_extensions_enabled,
            &self.header_extension_format,
            &self.header_extension_mappings,
            id, 
            uri
        ).await
    }
    
    async fn configure_header_extensions(&self, mappings: HashMap<u8, String>) -> Result<(), MediaTransportError> {
        super::media::configure_header_extensions(
            &self.header_extensions_enabled,
            &self.header_extension_format,
            &self.header_extension_mappings,
            mappings
        ).await
    }
    
    async fn add_header_extension_for_client(&self, client_id: &str, extension: HeaderExtension) -> Result<(), MediaTransportError> {
        super::media::add_header_extension_for_client(
            &self.header_extensions_enabled,
            &self.clients,
            &self.pending_extensions,
            client_id,
            extension
        ).await
    }
    
    async fn add_header_extension_for_all_clients(&self, extension: HeaderExtension) -> Result<(), MediaTransportError> {
        super::media::add_header_extension_for_all_clients(
            &self.header_extensions_enabled,
            &self.clients,
            &self.pending_extensions,
            extension
        ).await
    }
    
    async fn add_audio_level_extension_for_client(&self, client_id: &str, voice_activity: bool, level: u8) -> Result<(), MediaTransportError> {
        super::media::add_audio_level_extension_for_client(
            &self.header_extensions_enabled,
            &self.header_extension_mappings,
            &self.clients,
            &self.pending_extensions,
            client_id,
            voice_activity,
            level
        ).await
    }
    
    async fn add_audio_level_extension_for_all_clients(&self, voice_activity: bool, level: u8) -> Result<(), MediaTransportError> {
        super::media::add_audio_level_extension_for_all_clients(
            &self.header_extensions_enabled,
            &self.header_extension_mappings,
            &self.clients,
            &self.pending_extensions,
            voice_activity,
            level
        ).await
    }
    
    async fn add_video_orientation_extension_for_client(&self, client_id: &str, camera_front_facing: bool, camera_flipped: bool, rotation: u16) -> Result<(), MediaTransportError> {
        todo!("Implement add_video_orientation_extension_for_client")
    }
    
    async fn add_video_orientation_extension_for_all_clients(&self, camera_front_facing: bool, camera_flipped: bool, rotation: u16) -> Result<(), MediaTransportError> {
        todo!("Implement add_video_orientation_extension_for_all_clients")
    }
    
    async fn add_transport_cc_extension_for_client(&self, client_id: &str, sequence_number: u16) -> Result<(), MediaTransportError> {
        todo!("Implement add_transport_cc_extension_for_client")
    }
    
    async fn add_transport_cc_extension_for_all_clients(&self, sequence_number: u16) -> Result<(), MediaTransportError> {
        todo!("Implement add_transport_cc_extension_for_all_clients")
    }
    
    async fn get_received_header_extensions(&self, client_id: &str) -> Result<Vec<HeaderExtension>, MediaTransportError> {
        super::media::get_received_header_extensions(
            &self.header_extensions_enabled,
            &self.received_extensions,
            client_id
        ).await
    }
    
    async fn get_received_audio_level(&self, client_id: &str) -> Result<Option<(bool, u8)>, MediaTransportError> {
        super::media::get_received_audio_level(
            &self.header_extensions_enabled,
            &self.header_extension_mappings,
            &self.received_extensions,
            client_id
        ).await
    }
    
    async fn get_received_video_orientation(&self, client_id: &str) -> Result<Option<(bool, bool, u16)>, MediaTransportError> {
        todo!("Implement get_received_video_orientation")
    }
    
    async fn get_received_transport_cc(&self, client_id: &str) -> Result<Option<u16>, MediaTransportError> {
        todo!("Implement get_received_transport_cc")
    }

    async fn is_ssrc_demultiplexing_enabled(&self) -> Result<bool, MediaTransportError> {
        super::ssrc::is_ssrc_demultiplexing_enabled(
            &self.ssrc_demultiplexing_enabled
        ).await
    }

    async fn enable_ssrc_demultiplexing(&self) -> Result<bool, MediaTransportError> {
        super::ssrc::enable_ssrc_demultiplexing(
            &self.ssrc_demultiplexing_enabled
        ).await
    }

    async fn register_client_ssrc(&self, client_id: &str, ssrc: u32) -> Result<bool, MediaTransportError> {
        super::ssrc::register_client_ssrc(
            client_id,
            ssrc,
            &self.ssrc_demultiplexing_enabled,
            &self.clients
        ).await
    }

    async fn get_client_ssrcs(&self, client_id: &str) -> Result<Vec<u32>, MediaTransportError> {
        super::ssrc::get_client_ssrcs(
            client_id,
            &self.clients
        ).await
    }

    async fn update_csrc_cname(&self, original_ssrc: RtpSsrc, cname: String) -> Result<bool, MediaTransportError> {
        super::media::update_csrc_cname(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
            cname
        ).await
    }

    async fn update_csrc_display_name(&self, original_ssrc: RtpSsrc, name: String) -> Result<bool, MediaTransportError> {
        super::media::update_csrc_display_name(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
            name
        ).await
    }
}

 