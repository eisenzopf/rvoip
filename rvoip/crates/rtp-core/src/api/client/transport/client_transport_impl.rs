//! Client transport implementation
//!
//! This module implements the client-side transport API.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time;
use tracing::{debug, error, info, warn};
use uuid;
use bytes;
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use rand::Rng;

use crate::api::common::frame::MediaFrame;
use crate::api::common::frame::MediaFrameType;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::{MediaTransportEvent, MediaEventCallback};
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::common::stats::{StreamStats, Direction, QualityLevel};
use crate::api::client::config::ClientConfig;
use crate::api::client::transport::MediaTransportClient;
use crate::api::client::transport::RtcpStats;
use crate::api::client::transport::VoipMetrics;
use crate::api::client::security::{ClientSecurityContext, DefaultClientSecurityContext};
use crate::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::api::server::security::SocketHandle;
use crate::packet::rtcp::{RtcpPacket, RtcpApplicationDefined, RtcpGoodbye, RtcpExtendedReport, RtcpXrBlock, VoipMetricsBlock};
use crate::{CsrcManager, CsrcMapping, RtpSsrc, RtpCsrc, MAX_CSRC_COUNT};
use bytes::Bytes;
use crate::api::common::extension::ExtensionFormat;
use crate::api::server::transport::HeaderExtension;

/// Default implementation of the client-side media transport
pub struct DefaultMediaTransportClient {
    /// Client configuration
    config: ClientConfig,
    
    /// RTP session for media transport
    session: Arc<Mutex<RtpSession>>,
    
    /// Security context for DTLS/SRTP
    security: Option<Arc<dyn ClientSecurityContext>>,
    
    /// Main RTP/RTCP transport socket
    transport: Arc<Mutex<Option<Arc<UdpRtpTransport>>>>,
    
    /// Connected flag
    connected: Arc<AtomicBool>,
    
    /// Frame sender for passing received frames to the application
    frame_sender: mpsc::Sender<MediaFrame>,
    
    /// Frame receiver for the application to receive frames
    frame_receiver: Arc<Mutex<mpsc::Receiver<MediaFrame>>>,
    
    /// Event callbacks
    event_callbacks: Arc<Mutex<Vec<MediaEventCallback>>>,
    
    /// Connect callbacks
    connect_callbacks: Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
    
    /// Disconnect callbacks
    disconnect_callbacks: Arc<Mutex<Vec<Box<dyn Fn() + Send + Sync>>>>,
    
    /// Media synchronization context
    media_sync: Arc<RwLock<Option<crate::sync::MediaSync>>>,
    
    /// Media sync enabled flag (can be enabled even if config.media_sync_enabled is None)
    media_sync_enabled: Arc<AtomicBool>,
    
    /// SSRC demultiplexing enabled flag
    ssrc_demultiplexing_enabled: Arc<AtomicBool>,
    
    /// Sequence number tracking per SSRC
    sequence_numbers: Arc<Mutex<HashMap<u32, u16>>>,
    
    /// CSRC management enabled flag
    csrc_management_enabled: Arc<AtomicBool>,
    
    /// CSRC manager for handling contributing source IDs
    csrc_manager: Arc<Mutex<CsrcManager>>,
}

impl DefaultMediaTransportClient {
    /// Create a new DefaultMediaTransportClient
    pub async fn new(config: ClientConfig) -> Result<Self, MediaTransportError> {
        // Create channel for frames
        let (frame_sender, frame_receiver) = mpsc::channel(100);
        
        // Create session config from client config
        let session_config = RtpSessionConfig {
            // Basic RTP configuration
            ssrc: Some(config.ssrc.unwrap_or_else(rand::random)),
            clock_rate: config.clock_rate,
            payload_type: config.default_payload_type,
            local_addr: "0.0.0.0:0".parse().unwrap(), // Bind to any address/port
            remote_addr: Some(config.remote_address),
            
            // Jitter buffer configuration
            jitter_buffer_size: Some(config.jitter_buffer_size as usize),
            max_packet_age_ms: Some(config.jitter_max_packet_age_ms),
            enable_jitter_buffer: config.enable_jitter_buffer,
        };
        
        // Create RTP session
        let session = RtpSession::new(session_config).await
            .map_err(|e| MediaTransportError::InitializationError(format!("Failed to create RTP session: {}", e)))?;
            
        // Create security context if enabled
        let security_context = if config.security_config.security_mode.is_enabled() {
            let security_ctx = DefaultClientSecurityContext::new(
                config.security_config.clone(),
            ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create security context: {}", e)))?;
            
            // Store security context with explicit cast to trait object
            Some(security_ctx as Arc<dyn ClientSecurityContext>)
        } else {
            None
        };
        
        // Initialize media sync if enabled in config
        let media_sync_enabled = config.media_sync_enabled.unwrap_or(false);
        let media_sync = if media_sync_enabled {
            // Create media sync context
            Some(crate::sync::MediaSync::new())
        } else {
            None
        };
        
        // Initialize SSRC demultiplexing if enabled in config
        let ssrc_demultiplexing_enabled = config.ssrc_demultiplexing_enabled.unwrap_or(false);
        
        // Initialize CSRC management from config
        let csrc_management_enabled = config.csrc_management_enabled; // This is already a bool
        
        Ok(Self {
            config,
            session: Arc::new(Mutex::new(session)),
            security: security_context,
            transport: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
            frame_sender,
            frame_receiver: Arc::new(Mutex::new(frame_receiver)),
            event_callbacks: Arc::new(Mutex::new(Vec::new())),
            connect_callbacks: Arc::new(Mutex::new(Vec::new())),
            disconnect_callbacks: Arc::new(Mutex::new(Vec::new())),
            media_sync: Arc::new(RwLock::new(media_sync)),
            media_sync_enabled: Arc::new(AtomicBool::new(media_sync_enabled)),
            ssrc_demultiplexing_enabled: Arc::new(AtomicBool::new(ssrc_demultiplexing_enabled)),
            sequence_numbers: Arc::new(Mutex::new(HashMap::new())),
            csrc_management_enabled: Arc::new(AtomicBool::new(csrc_management_enabled)),
            csrc_manager: Arc::new(Mutex::new(CsrcManager::new())),
        })
    }
    
    /// Get the local address currently bound to
    /// 
    /// This returns the actual bound address of the transport, which may be different
    /// from the configured address if dynamic port allocation is used.
    pub async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError> {
        let transport_guard = self.transport.lock().await;
        if let Some(transport) = transport_guard.as_ref() {
            transport.local_rtp_addr()
                .map_err(|e| MediaTransportError::Transport(format!("Failed to get local address: {}", e)))
        } else {
            Err(MediaTransportError::Transport("Transport not initialized. Connect first to bind to a port.".to_string()))
        }
    }
    
    /// Process an incoming RTP packet
    async fn process_packet(&self, packet: &[u8]) -> Result<(), MediaTransportError> {
        let mut session = self.session.lock().await;
        
        // Handle the processing here manually since we have raw packet data
        // RTP parsing and processing
        match crate::packet::RtpPacket::parse(packet) {
            Ok(rtp_packet) => {
                // Found a valid RTP packet, process it in a simplified way
                // In a full implementation, we would add it to the jitter buffer and process
                
                // Create a simplified MediaFrame from the RTP packet
                let frame = MediaFrame {
                    frame_type: self.get_frame_type_from_payload_type(rtp_packet.header.payload_type),
                    data: rtp_packet.payload.to_vec(),
                    timestamp: rtp_packet.header.timestamp,
                    sequence: rtp_packet.header.sequence_number,
                    marker: rtp_packet.header.marker,
                    payload_type: rtp_packet.header.payload_type,
                    ssrc: rtp_packet.header.ssrc,
                    csrcs: rtp_packet.header.csrc.clone(),
                };
                
                // Forward frame to the application
                if let Err(e) = self.frame_sender.send(frame).await {
                    warn!("Error sending frame to application: {}", e);
                }
                
                Ok(())
            },
            Err(e) => {
                warn!("Error parsing RTP packet: {}", e);
                Err(MediaTransportError::ReceiveError(format!("Failed to parse RTP packet: {}", e)))
            }
        }
    }
    
    /// Get frame type based on payload type
    fn get_frame_type_from_payload_type(&self, payload_type: u8) -> MediaFrameType {
        match payload_type {
            // Audio payload types (common)
            0..=34 => MediaFrameType::Audio,
            // Video payload types (common)
            35..=50 => MediaFrameType::Video,
            // Dynamic payload types - use config to determine
            96..=127 => {
                // For now, default to audio for dynamic payload types
                // In a real implementation, we would check the configured codec
                MediaFrameType::Audio
            },
            // All other types
            _ => MediaFrameType::Data,
        }
    }
    
    /// Estimate media quality level based on statistics
    fn estimate_quality_level(&self, stats: &MediaStats) -> crate::api::common::stats::QualityLevel {
        use crate::api::common::stats::QualityLevel;
        
        // Simplified quality estimation based on packet loss and jitter
        // Look for the first stream and use its quality
        if let Some(stream) = stats.streams.values().next() {
            // Return the stream's quality
            stream.quality
        } else {
            QualityLevel::Unknown
        }
    }
    
    /// Access to the RTP session (for advanced usage in examples)
    pub async fn get_session(&self) -> Result<Arc<Mutex<crate::session::RtpSession>>, MediaTransportError> {
        Ok(Arc::clone(&self.session))
    }
    
    /// Pre-register an SSRC for demultiplexing
    /// 
    /// This method pre-creates a stream for the specified SSRC to ensure it's properly tracked
    /// when packets are received. This is only useful when SSRC demultiplexing is enabled.
    /// 
    /// Returns true if the stream was created, false if it already existed or if demultiplexing
    /// is disabled.
    pub async fn register_ssrc(&self, ssrc: u32) -> Result<bool, MediaTransportError> {
        // Check if SSRC demultiplexing is enabled
        if !self.ssrc_demultiplexing_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("SSRC demultiplexing is not enabled".to_string()));
        }
        
        // Create stream for SSRC in the session
        let mut session = self.session.lock().await;
        let created = session.create_stream_for_ssrc(ssrc).await;
        
        if created {
            debug!("Pre-registered SSRC {:08x} for demultiplexing", ssrc);
            
            // Also initialize sequence numbers
            drop(session); // Release lock on session
            let mut seq_map = self.sequence_numbers.lock().await;
            if !seq_map.contains_key(&ssrc) {
                // Start with a random sequence number for this SSRC
                let mut rng = rand::thread_rng();
                seq_map.insert(ssrc, rng.gen());
                debug!("Initialized sequence number tracking for SSRC {:08x}", ssrc);
            }
        } else {
            debug!("SSRC {:08x} was already registered", ssrc);
        }
        
        Ok(created)
    }
    
    /// Get a list of all known SSRCs
    ///
    /// Returns all SSRCs that have been received or manually registered.
    pub async fn get_all_ssrcs(&self) -> Result<Vec<u32>, MediaTransportError> {
        let session = self.session.lock().await;
        let ssrcs = session.get_all_ssrcs().await;
        Ok(ssrcs)
    }
    
    /// Check if SSRC demultiplexing is enabled
    pub async fn is_ssrc_demultiplexing_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(self.ssrc_demultiplexing_enabled.load(Ordering::SeqCst))
    }
    
    /// Enable SSRC demultiplexing
    pub async fn enable_ssrc_demultiplexing(&self) -> Result<bool, MediaTransportError> {
        // Check if already enabled
        if self.ssrc_demultiplexing_enabled.load(Ordering::SeqCst) {
            return Ok(true);
        }
        
        // Set enabled flag
        self.ssrc_demultiplexing_enabled.store(true, Ordering::SeqCst);
        
        debug!("Enabled SSRC demultiplexing");
        Ok(true)
    }
    
    /// Get the sequence number for a specific SSRC, if it exists in the map
    pub async fn get_sequence_number(&self, ssrc: u32) -> Option<u16> {
        let seq_map = self.sequence_numbers.lock().await;
        seq_map.get(&ssrc).copied()
    }
    
    /// Generate a new sequence number
    fn generate_sequence_number(&self) -> u16 {
        rand::random()
    }
    
    /// Check if CSRC management is enabled
    pub async fn is_csrc_management_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(self.csrc_management_enabled.load(Ordering::SeqCst))
    }
    
    /// Enable CSRC management
    pub async fn enable_csrc_management(&self) -> Result<bool, MediaTransportError> {
        // Check if already enabled
        if self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Ok(true);
        }
        
        // Set enabled flag
        self.csrc_management_enabled.store(true, Ordering::SeqCst);
        
        debug!("Enabled CSRC management");
        Ok(true)
    }
    
    /// Add a CSRC mapping for a source
    pub async fn add_csrc_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Add mapping to the manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        let mapping_clone = mapping.clone(); // Clone before adding
        csrc_manager.add_mapping(mapping);
        
        debug!("Added CSRC mapping: {:?}", mapping_clone);
        Ok(())
    }
    
    /// Add a simple SSRC to CSRC mapping
    pub async fn add_simple_csrc_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Result<(), MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Add simple mapping to the manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        csrc_manager.add_simple_mapping(original_ssrc, csrc);
        
        debug!("Added simple CSRC mapping: {:08x} -> {:08x}", original_ssrc, csrc);
        Ok(())
    }
    
    /// Remove a CSRC mapping by SSRC
    pub async fn remove_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Remove mapping from the manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        let removed = csrc_manager.remove_by_ssrc(original_ssrc);
        
        if removed.is_some() {
            debug!("Removed CSRC mapping for SSRC: {:08x}", original_ssrc);
        }
        
        Ok(removed)
    }
    
    /// Get a CSRC mapping by SSRC
    pub async fn get_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Get mapping from the manager
        let csrc_manager = self.csrc_manager.lock().await;
        let mapping = csrc_manager.get_by_ssrc(original_ssrc).cloned();
        
        Ok(mapping)
    }
    
    /// Get a list of all CSRC mappings
    pub async fn get_all_csrc_mappings(&self) -> Result<Vec<CsrcMapping>, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Get all mappings from the manager
        let csrc_manager = self.csrc_manager.lock().await;
        let mappings = csrc_manager.get_all_mappings().to_vec();
        
        Ok(mappings)
    }
    
    /// Update the CNAME for a source
    pub async fn update_csrc_cname(&self, original_ssrc: RtpSsrc, cname: String) -> Result<bool, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Update CNAME in the manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        let updated = csrc_manager.update_cname(original_ssrc, cname.clone());
        
        if updated {
            debug!("Updated CNAME for SSRC {:08x}: {}", original_ssrc, cname);
        }
        
        Ok(updated)
    }
    
    /// Update the display name for a source
    pub async fn update_csrc_display_name(&self, original_ssrc: RtpSsrc, name: String) -> Result<bool, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Update display name in the manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        let updated = csrc_manager.update_display_name(original_ssrc, name.clone());
        
        if updated {
            debug!("Updated display name for SSRC {:08x}: {}", original_ssrc, name);
        }
        
        Ok(updated)
    }
    
    /// Get CSRC values for active sources
    pub async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Result<Vec<RtpCsrc>, MediaTransportError> {
        // Check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Get active CSRCs from the manager
        let csrc_manager = self.csrc_manager.lock().await;
        let csrcs = csrc_manager.get_active_csrcs(active_ssrcs);
        
        debug!("Got {} active CSRCs for {} active SSRCs", csrcs.len(), active_ssrcs.len());
        
        Ok(csrcs)
    }
}

// Add Clone implementation
impl Clone for DefaultMediaTransportClient {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            session: Arc::clone(&self.session),
            security: self.security.clone(),
            transport: Arc::clone(&self.transport),
            connected: Arc::clone(&self.connected),
            frame_sender: self.frame_sender.clone(),
            frame_receiver: Arc::clone(&self.frame_receiver),
            event_callbacks: Arc::clone(&self.event_callbacks),
            connect_callbacks: Arc::clone(&self.connect_callbacks),
            disconnect_callbacks: Arc::clone(&self.disconnect_callbacks),
            media_sync: Arc::clone(&self.media_sync),
            media_sync_enabled: Arc::clone(&self.media_sync_enabled),
            ssrc_demultiplexing_enabled: Arc::clone(&self.ssrc_demultiplexing_enabled),
            sequence_numbers: Arc::clone(&self.sequence_numbers),
            csrc_management_enabled: Arc::clone(&self.csrc_management_enabled),
            csrc_manager: Arc::clone(&self.csrc_manager),
        }
    }
}

#[async_trait]
impl MediaTransportClient for DefaultMediaTransportClient {
    async fn connect(&self) -> Result<(), MediaTransportError> {
        if self.connected.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        // Create UDP transport
        let transport_config = crate::transport::RtpTransportConfig {
            local_rtp_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: self.config.rtcp_mux,
            session_id: Some(format!("client-{}", uuid::Uuid::new_v4())),
            use_port_allocator: true,
        };
        
        let transport = UdpRtpTransport::new(transport_config).await
            .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to create transport: {}", e)))?;
        
        let transport = Arc::new(transport);
        
        // Set the transport
        let mut transport_guard = self.transport.lock().await;
        *transport_guard = Some(transport.clone());
        drop(transport_guard);
        
        // Get socket handle
        let socket_arc = transport.get_socket();

        // Create a proper SocketHandle
        let socket_handle = SocketHandle {
            socket: socket_arc,
            remote_addr: None,
        };
        
        // If security is enabled, set up the security context
        if let Some(security) = &self.security {
            // Set remote address
            security.set_remote_address(self.config.remote_address).await
                .map_err(|e| MediaTransportError::Security(format!("Failed to set remote address: {}", e)))?;
                
            // Set socket
            security.set_socket(socket_handle).await
                .map_err(|e| MediaTransportError::Security(format!("Failed to set socket: {}", e)))?;
                
            // If remote fingerprint is set, set it on the security context
            if let (Some(fp), Some(algo)) = (&self.config.security_config.remote_fingerprint, &self.config.security_config.remote_fingerprint_algorithm) {
                security.set_remote_fingerprint(fp, algo).await
                    .map_err(|e| MediaTransportError::Security(format!("Failed to set remote fingerprint: {}", e)))?;
            }
            
            // Start handshake
            security.start_handshake().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to start handshake: {}", e)))?;
                
            // Wait for handshake to complete
            while !security.is_handshake_complete().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to check handshake status: {}", e)))? {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        
        // Set connected flag
        self.connected.store(true, Ordering::SeqCst);
        
        // Notify callbacks
        let callbacks = self.connect_callbacks.lock().await;
        for callback in &*callbacks {
            callback();
        }
        
        // Prepare data for the background task
        let transport_clone = self.transport.clone();
        let connected = self.connected.clone();
        let frame_sender_clone = self.frame_sender.clone();

        // Spawn task to receive packets in the background
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 2048];
            
            while connected.load(Ordering::SeqCst) {
                // Get the transport
                let transport_guard = transport_clone.lock().await;
                if let Some(transport) = transport_guard.as_ref() {
                    // Receive packet from transport
                    match transport.receive_packet(&mut buffer).await {
                        Ok((size, addr)) => {
                            if size == 0 {
                                // Empty packet, ignore
                                continue;
                            }
                            
                            drop(transport_guard); // Drop the lock before lengthy processing
                            
                            // Parse as RTP packet
                            match crate::packet::RtpPacket::parse(&buffer[..size]) {
                                Ok(rtp_packet) => {
                                    // Process the packet by creating a MediaFrame
                                    let frame = MediaFrame {
                                        frame_type: match rtp_packet.header.payload_type {
                                            // Audio payload types (common)
                                            0..=34 => MediaFrameType::Audio,
                                            // Video payload types (common)
                                            35..=50 => MediaFrameType::Video,
                                            // Dynamic payload types - use config to determine
                                            96..=127 => MediaFrameType::Audio, // Default for now
                                            // All other types
                                            _ => MediaFrameType::Data,
                                        },
                                        data: rtp_packet.payload.to_vec(),
                                        timestamp: rtp_packet.header.timestamp,
                                        sequence: rtp_packet.header.sequence_number,
                                        marker: rtp_packet.header.marker,
                                        payload_type: rtp_packet.header.payload_type,
                                        ssrc: rtp_packet.header.ssrc,
                                        csrcs: rtp_packet.header.csrc.clone(),
                                    };
                                    
                                    // Send frame to application
                                    if let Err(e) = frame_sender_clone.send(frame).await {
                                        warn!("Failed to send frame to application: {}", e);
                                    }
                                },
                                Err(e) => {
                                    warn!("Failed to parse RTP packet: {}", e);
                                }
                            }
                        },
                        Err(e) => {
                            if connected.load(Ordering::SeqCst) {
                                warn!("Failed to receive packet: {}", e);
                            }
                        }
                    }
                } else {
                    // No transport available, sleep a bit
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
            
            debug!("Receive task ended");
        });
        
        Ok(())
    }
    
    async fn disconnect(&self) -> Result<(), MediaTransportError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Ok(());
        }
        
        // Close security context
        if let Some(security) = &self.security {
            security.close().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to close security context: {}", e)))?;
        }
        
        // Close transport
        let mut transport_guard = self.transport.lock().await;
        if let Some(transport) = transport_guard.as_ref() {
            if let Err(e) = transport.close().await {
                warn!("Failed to close transport: {}", e);
            }
        }
        *transport_guard = None;
        
        // Update connected flag
        self.connected.store(false, Ordering::SeqCst);
        
        // Notify callbacks
        let callbacks = self.disconnect_callbacks.lock().await;
        for callback in &*callbacks {
            callback();
        }
        
        Ok(())
    }
    
    async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError> {
        let transport_guard = self.transport.lock().await;
        if let Some(transport) = transport_guard.as_ref() {
            transport.local_rtp_addr()
                .map_err(|e| MediaTransportError::Transport(format!("Failed to get local address: {}", e)))
        } else {
            Err(MediaTransportError::Transport("Transport not initialized. Connect first to bind to a port.".to_string()))
        }
    }
    
    async fn send_frame(&self, mut frame: MediaFrame) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        let mut session = self.session.lock().await;
        
        // Select SSRC
        let ssrc = if self.ssrc_demultiplexing_enabled.load(Ordering::SeqCst) && frame.ssrc != 0 {
            // Use custom SSRC from the frame
            frame.ssrc
        } else {
            // Use default SSRC from session
            session.get_ssrc()
        };
        
        // Get sequence number
        let sequence = if self.ssrc_demultiplexing_enabled.load(Ordering::SeqCst) && frame.ssrc != 0 {
            // Use sequence number mapping for this SSRC
            let mut seq_map = self.sequence_numbers.lock().await;
            let sequence = if let Some(seq) = seq_map.get_mut(&frame.ssrc) {
                // Increment sequence number
                *seq = seq.wrapping_add(1);
                *seq
            } else {
                // Start with a random sequence number for this SSRC
                let sequence = rand::random::<u16>();
                seq_map.insert(frame.ssrc, sequence);
                sequence
            };
            sequence
        } else {
            // Default: Use sequence number from the frame or generate a new one
            if frame.sequence != 0 {
                frame.sequence
            } else {
                // Generate a new sequence number
                let sequence = self.generate_sequence_number();
                sequence
            }
        };
        
        // Store frame data length before it's moved
        let data_len = frame.data.len();
        
        // Get transport
        let transport_guard = self.transport.lock().await;
        let transport = transport_guard.as_ref()
            .ok_or_else(|| MediaTransportError::Transport("Transport not connected".to_string()))?;
        
        // Create RTP header
        let mut header = crate::packet::RtpHeader::new(
            frame.payload_type,
            sequence,
            frame.timestamp,
            ssrc
        );
        
        // Set marker flag if present in frame
        if frame.marker {
            header.marker = true;
        }
        
        // Add CSRCs if CSRC management is enabled
        if self.csrc_management_enabled.load(Ordering::SeqCst) {
            // For simplicity, we'll just use all active SSRCs as active sources
            // In a real conference mixer, this would be based on audio activity
            // Get all SSRCs from the session (we don't have get_active_streams)
            let active_ssrcs = session.get_all_ssrcs().await;
            
            if !active_ssrcs.is_empty() {
                // Get CSRC values from the manager
                let csrc_manager = self.csrc_manager.lock().await;
                let csrcs = csrc_manager.get_active_csrcs(&active_ssrcs);
                
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
        
        // Create RTP packet
        let packet = crate::packet::RtpPacket::new(
            header,
            Bytes::from(frame.data),
        );
        
        // Send packet
        let remote_addr = self.config.remote_address;
        transport.send_rtp(&packet, remote_addr).await
            .map_err(|e| MediaTransportError::SendError(format!("Failed to send RTP packet: {}", e)))?;
        
        // We don't have update_sent_stats method in RtpSession, so we'll just log
        debug!("Sent frame: PT={}, TS={}, SEQ={}, Size={} bytes", 
               frame.payload_type, frame.timestamp, sequence, data_len);
        
        Ok(())
    }
    
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError> {
        let mut receiver = self.frame_receiver.lock().await;
        
        match tokio::time::timeout(timeout, receiver.recv()).await {
            Ok(Some(frame)) => Ok(Some(frame)),
            Ok(None) => Err(MediaTransportError::ReceiveError("Channel closed".to_string())),
            Err(_) => Ok(None), // Timeout occurred
        }
    }
    
    async fn is_connected(&self) -> Result<bool, MediaTransportError> {
        Ok(self.connected.load(Ordering::SeqCst))
    }
    
    async fn on_connect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError> {
        let mut callbacks = self.connect_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn on_disconnect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError> {
        let mut callbacks = self.disconnect_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError> {
        let mut callbacks = self.event_callbacks.lock().await;
        callbacks.push(callback);
        Ok(())
    }
    
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError> {
        // Get session stats
        let session = self.session.lock().await;
        
        let rtp_stats = session.get_stats();
        
        // Create the MediaStats struct
        let mut media_stats = MediaStats::default();
        
        // Set the session duration
        media_stats.session_duration = Duration::from_secs(0); // Will be set properly when we have access to the session uptime
        
        // Create a stream entry
        let stream_stats = StreamStats {
            direction: Direction::Outbound,
            ssrc: session.get_ssrc(),
            media_type: MediaFrameType::Audio, // Default to audio
            packet_count: rtp_stats.packets_sent,
            byte_count: rtp_stats.bytes_sent,
            packets_lost: rtp_stats.packets_lost,
            fraction_lost: if rtp_stats.packets_sent > 0 {
                rtp_stats.packets_lost as f32 / rtp_stats.packets_sent as f32
            } else {
                0.0
            },
            jitter_ms: rtp_stats.jitter_ms as f32,
            rtt_ms: None, // Not available yet
            mos: None, // Not calculated yet
            remote_addr: self.config.remote_address,
            bitrate_bps: 0, // Will calculate later
            discard_rate: 0.0,
            quality: QualityLevel::Unknown,
            available_bandwidth_bps: None,
        };
        
        // Add to our stats
        media_stats.streams.insert(stream_stats.ssrc, stream_stats);
        
        // Estimate quality level
        media_stats.quality = self.estimate_quality_level(&media_stats);
        
        Ok(media_stats)
    }
    
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError> {
        // If security is enabled, get security info from the security context
        if let Some(security) = &self.security {
            security.get_security_info().await
                .map_err(|e| MediaTransportError::Security(format!("Failed to get security info: {}", e)))
        } else {
            // If security is not enabled, return empty info
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
    
    fn is_secure(&self) -> bool {
        self.security.is_some() && self.config.security_config.security_mode.is_enabled()
    }
    
    async fn set_jitter_buffer_size(&self, size_ms: Duration) -> Result<(), MediaTransportError> {
        // This is a stub implementation since RtpSession doesn't expose a direct method to change
        // the jitter buffer size at runtime. In a real implementation, we would need to:
        // 1. Create a new session with the desired jitter buffer size
        // 2. Transfer the state from the old session to the new one
        // 3. Replace the old session with the new one
        
        warn!("Changing jitter buffer size at runtime is not supported");
        
        Ok(())
    }
    
    async fn send_rtcp_receiver_report(&self) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        // Get the session and send the receiver report
        let mut session = self.session.lock().await;
        session.send_receiver_report().await
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP receiver report: {}", e)))
    }
    
    async fn send_rtcp_sender_report(&self) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        // Get the session and send the sender report
        let mut session = self.session.lock().await;
        session.send_sender_report().await
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send RTCP sender report: {}", e)))
    }
    
    async fn get_rtcp_stats(&self) -> Result<RtcpStats, MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        let session = self.session.lock().await;
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
    
    async fn set_rtcp_interval(&self, interval: Duration) -> Result<(), MediaTransportError> {
        let mut session = self.session.lock().await;
        
        // The bandwidth calculation follows from RFC 3550 where RTCP bandwidth is typically 
        // 5% of session bandwidth. If we want a specific interval, we need to set the
        // bandwidth accordingly: bandwidth = packet_size * 8 / interval_fraction
        // where interval_fraction is 0.05 for 5%
        
        // Assuming average RTCP packet is around 100 bytes, calculate bandwidth
        let bytes_per_second = 100.0 / interval.as_secs_f64();
        let bits_per_second = bytes_per_second * 8.0 / 0.05; // 5% of bandwidth for RTCP
        
        // Set bandwidth on the session
        session.set_bandwidth(bits_per_second as u32);
        
        Ok(())
    }
    
    async fn send_rtcp_app(&self, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        // Validate name (must be exactly 4 ASCII characters)
        if name.len() != 4 || !name.chars().all(|c| c.is_ascii()) {
            return Err(MediaTransportError::ConfigError(
                "APP name must be exactly 4 ASCII characters".to_string()
            ));
        }
        
        // Get session for SSRC
        let session = self.session.lock().await;
        let ssrc = session.get_ssrc();
        
        // Create APP packet
        let mut app_packet = crate::RtcpApplicationDefined::new_with_name(ssrc, name)
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to create APP packet: {}", e)))?;
        
        // Set data - clone it before using
        let data_clone = data.clone();
        app_packet.set_data(bytes::Bytes::from(data));
        
        // Create RTCP packet
        let rtcp_packet = crate::RtcpPacket::ApplicationDefined(app_packet);
        
        // Serialize
        let rtcp_data = rtcp_packet.serialize()
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize APP packet: {}", e)))?;
        
        // Get transport
        let transport_guard = self.transport.lock().await;
        let transport = transport_guard.as_ref()
            .ok_or_else(|| MediaTransportError::NotConnected)?;
        
        // Send to remote address
        transport.send_rtcp_bytes(&rtcp_data, self.config.remote_address).await
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send APP packet: {}", e)))?;
        
        debug!("Sent RTCP APP packet: name={}, data_len={}", name, data_clone.len());
        
        Ok(())
    }
    
    async fn send_rtcp_bye(&self, reason: Option<String>) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        // Get session
        let session = self.session.lock().await;
        
        // Create BYE packet with our SSRC - clone reason before moving
        let reason_clone = reason.clone();
        let bye_packet = crate::RtcpGoodbye {
            sources: vec![session.get_ssrc()],
            reason,
        };
        
        // Create RTCP packet
        let rtcp_packet = crate::RtcpPacket::Goodbye(bye_packet);
        
        // Serialize
        let rtcp_data = rtcp_packet.serialize()
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize BYE packet: {}", e)))?;
        
        // Get transport
        let transport_guard = self.transport.lock().await;
        let transport = transport_guard.as_ref()
            .ok_or_else(|| MediaTransportError::NotConnected)?;
        
        // Send to remote address
        transport.send_rtcp_bytes(&rtcp_data, self.config.remote_address).await
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send BYE packet: {}", e)))?;
        
        debug!("Sent RTCP BYE packet: reason={:?}", reason_clone);
        
        Ok(())
    }
    
    async fn send_rtcp_xr_voip_metrics(&self, metrics: VoipMetrics) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        // Get session for SSRC
        let session = self.session.lock().await;
        let ssrc = session.get_ssrc();
        
        // Create XR packet
        let mut xr_packet = crate::RtcpExtendedReport::new(ssrc);
        
        // Convert our metrics to VoipMetricsBlock
        let voip_metrics_block = crate::VoipMetricsBlock {
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
        xr_packet.add_block(crate::RtcpXrBlock::VoipMetrics(voip_metrics_block));
        
        // Create RTCP packet
        let rtcp_packet = crate::RtcpPacket::ExtendedReport(xr_packet);
        
        // Serialize
        let rtcp_data = rtcp_packet.serialize()
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to serialize XR packet: {}", e)))?;
        
        // Get transport
        let transport_guard = self.transport.lock().await;
        let transport = transport_guard.as_ref()
            .ok_or_else(|| MediaTransportError::NotConnected)?;
        
        // Send to remote address
        transport.send_rtcp_bytes(&rtcp_data, self.config.remote_address).await
            .map_err(|e| MediaTransportError::RtcpError(format!("Failed to send XR packet: {}", e)))?;
        
        debug!("Sent RTCP XR VoIP metrics: loss_rate={}%, r_factor={}", 
               metrics.loss_rate, metrics.r_factor);
        
        Ok(())
    }
    
    // Media Synchronization API Implementation
    async fn enable_media_sync(&self) -> Result<bool, MediaTransportError> {
        // Check if already enabled
        if self.media_sync_enabled.load(Ordering::SeqCst) {
            return Ok(true);
        }
        
        // Create media sync context if it doesn't exist
        let mut sync_guard = self.media_sync.write().await;
        if sync_guard.is_none() {
            *sync_guard = Some(crate::sync::MediaSync::new());
        }
        
        // Set enabled flag
        self.media_sync_enabled.store(true, Ordering::SeqCst);
        
        // Register default stream with session SSRC
        let session = self.session.lock().await;
        let ssrc = session.get_ssrc();
        let clock_rate = self.config.clock_rate;
        drop(session);
        
        if let Some(sync) = &mut *sync_guard {
            sync.register_stream(ssrc, clock_rate);
        }
        
        Ok(true)
    }
    
    async fn is_media_sync_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(self.media_sync_enabled.load(Ordering::SeqCst))
    }
    
    async fn register_sync_stream(&self, ssrc: u32, clock_rate: u32) -> Result<(), MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Register stream with media sync
        let mut sync_guard = self.media_sync.write().await;
        if let Some(sync) = &mut *sync_guard {
            sync.register_stream(ssrc, clock_rate);
            Ok(())
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn set_sync_reference_stream(&self, ssrc: u32) -> Result<(), MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Set reference stream
        let mut sync_guard = self.media_sync.write().await;
        if let Some(sync) = &mut *sync_guard {
            sync.set_reference_stream(ssrc);
            Ok(())
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn get_sync_info(&self, ssrc: u32) -> Result<Option<crate::api::client::transport::MediaSyncInfo>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Get sync info
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            // Get stream data from MediaSync
            let streams = sync.get_streams();
            if let Some(stream) = streams.get(&ssrc) {
                // Convert to MediaSyncInfo
                let info = crate::api::client::transport::MediaSyncInfo {
                    ssrc,
                    clock_rate: stream.clock_rate,
                    last_ntp: stream.last_ntp,
                    last_rtp: stream.last_rtp,
                    clock_drift_ppm: stream.clock_drift_ppm,
                };
                Ok(Some(info))
            } else {
                Ok(None)
            }
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn get_all_sync_info(&self) -> Result<std::collections::HashMap<u32, crate::api::client::transport::MediaSyncInfo>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Get all sync info
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            // Get all streams data from MediaSync
            let streams = sync.get_streams();
            let mut result = std::collections::HashMap::new();
            
            // Convert each stream to MediaSyncInfo
            for (ssrc, stream) in streams {
                let info = crate::api::client::transport::MediaSyncInfo {
                    ssrc: *ssrc,
                    clock_rate: stream.clock_rate,
                    last_ntp: stream.last_ntp,
                    last_rtp: stream.last_rtp,
                    clock_drift_ppm: stream.clock_drift_ppm,
                };
                result.insert(*ssrc, info);
            }
            
            Ok(result)
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn convert_timestamp(&self, from_ssrc: u32, to_ssrc: u32, rtp_ts: u32) -> Result<Option<u32>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Convert timestamp
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            Ok(sync.convert_timestamp(from_ssrc, to_ssrc, rtp_ts))
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn rtp_to_ntp(&self, ssrc: u32, rtp_ts: u32) -> Result<Option<crate::packet::rtcp::NtpTimestamp>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Convert RTP to NTP
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            Ok(sync.rtp_to_ntp(ssrc, rtp_ts))
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn ntp_to_rtp(&self, ssrc: u32, ntp: crate::packet::rtcp::NtpTimestamp) -> Result<Option<u32>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Convert NTP to RTP
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            Ok(sync.ntp_to_rtp(ssrc, ntp))
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn get_clock_drift_ppm(&self, ssrc: u32) -> Result<Option<f64>, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Get clock drift
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            Ok(sync.get_clock_drift_ppm(ssrc))
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    async fn are_streams_synchronized(&self, ssrc1: u32, ssrc2: u32, tolerance_ms: f64) -> Result<bool, MediaTransportError> {
        // Check if media sync is enabled
        if !self.media_sync_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("Media synchronization is not enabled".to_string()));
        }
        
        // Check if streams are synchronized
        let sync_guard = self.media_sync.read().await;
        if let Some(sync) = &*sync_guard {
            Ok(sync.are_synchronized(ssrc1, ssrc2, tolerance_ms))
        } else {
            Err(MediaTransportError::ConfigError("Media synchronization context not initialized".to_string()))
        }
    }
    
    // CSRC Management API Implementation
    
    async fn is_csrc_management_enabled(&self) -> Result<bool, MediaTransportError> {
        self.is_csrc_management_enabled().await
    }
    
    async fn enable_csrc_management(&self) -> Result<bool, MediaTransportError> {
        self.enable_csrc_management().await
    }
    
    async fn add_csrc_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaTransportError> {
        self.add_csrc_mapping(mapping).await
    }
    
    async fn add_simple_csrc_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Result<(), MediaTransportError> {
        self.add_simple_csrc_mapping(original_ssrc, csrc).await
    }
    
    async fn remove_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        self.remove_csrc_mapping_by_ssrc(original_ssrc).await
    }
    
    async fn get_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        self.get_csrc_mapping_by_ssrc(original_ssrc).await
    }
    
    async fn get_all_csrc_mappings(&self) -> Result<Vec<CsrcMapping>, MediaTransportError> {
        self.get_all_csrc_mappings().await
    }
    
    async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Result<Vec<RtpCsrc>, MediaTransportError> {
        self.get_active_csrcs(active_ssrcs).await
    }

    // Add the following methods:
    
    /// Check if header extensions are enabled
    async fn is_header_extensions_enabled(&self) -> Result<bool, MediaTransportError> {
        // For now, just check the config value
        Ok(self.config.header_extensions_enabled)
    }
    
    /// Enable header extensions with the specified format
    async fn enable_header_extensions(&self, format: ExtensionFormat) -> Result<bool, MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(true)
    }
    
    /// Configure a header extension mapping
    async fn configure_header_extension(&self, id: u8, uri: String) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Configure multiple header extension mappings
    async fn configure_header_extensions(&self, mappings: HashMap<u8, String>) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Add a header extension
    async fn add_header_extension(&self, extension: HeaderExtension) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Add audio level header extension
    async fn add_audio_level_extension(&self, voice_activity: bool, level: u8) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Add video orientation header extension
    async fn add_video_orientation_extension(&self, camera_front_facing: bool, camera_flipped: bool, rotation: u16) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Add transport-cc header extension
    async fn add_transport_cc_extension(&self, sequence_number: u16) -> Result<(), MediaTransportError> {
        // For now, just return success without actually implementing
        Ok(())
    }
    
    /// Get all header extensions received
    async fn get_received_header_extensions(&self) -> Result<Vec<HeaderExtension>, MediaTransportError> {
        // For now, return empty list
        Ok(Vec::new())
    }
    
    /// Get audio level header extension
    async fn get_received_audio_level(&self) -> Result<Option<(bool, u8)>, MediaTransportError> {
        // For now, return None
        Ok(None)
    }
    
    /// Get video orientation header extension
    async fn get_received_video_orientation(&self) -> Result<Option<(bool, bool, u16)>, MediaTransportError> {
        // For now, return None
        Ok(None)
    }
    
    /// Get transport-cc header extension
    async fn get_received_transport_cc(&self) -> Result<Option<u16>, MediaTransportError> {
        // For now, return None
        Ok(None)
    }
} 