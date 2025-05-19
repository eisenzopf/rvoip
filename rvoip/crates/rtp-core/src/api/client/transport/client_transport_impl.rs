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

use crate::api::common::frame::MediaFrame;
use crate::api::common::frame::MediaFrameType;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::{MediaTransportEvent, MediaEventCallback};
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::common::stats::{StreamStats, Direction, QualityLevel};
use crate::api::client::config::ClientConfig;
use crate::api::client::transport::MediaTransportClient;
use crate::api::client::security::{ClientSecurityContext, DefaultClientSecurityContext};
use crate::session::{RtpSession, RtpSessionConfig};
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::api::server::security::SocketHandle;

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
    connected: Arc<Mutex<bool>>,
    
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
            
            // Store security context directly as Arc<dyn ClientSecurityContext>
            Some(security_ctx)
        } else {
            None
        };
        
        Ok(Self {
            config,
            session: Arc::new(Mutex::new(session)),
            security: security_context,
            transport: Arc::new(Mutex::new(None)),
            connected: Arc::new(Mutex::new(false)),
            frame_sender,
            frame_receiver: Arc::new(Mutex::new(frame_receiver)),
            event_callbacks: Arc::new(Mutex::new(Vec::new())),
            connect_callbacks: Arc::new(Mutex::new(Vec::new())),
            disconnect_callbacks: Arc::new(Mutex::new(Vec::new())),
        })
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
}

#[async_trait]
impl MediaTransportClient for DefaultMediaTransportClient {
    async fn connect(&self) -> Result<(), MediaTransportError> {
        let mut connected = self.connected.lock().await;
        if *connected {
            return Ok(());
        }
        
        // Create UDP transport
        let transport_config = crate::transport::RtpTransportConfig {
            local_rtp_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
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
        let socket_handle = transport.get_socket_handle().await
            .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to get socket handle: {}", e)))?;
        
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
        *connected = true;
        
        // Notify callbacks
        let callbacks = self.connect_callbacks.lock().await;
        for callback in &*callbacks {
            callback();
        }
        
        // Start packet reception loop
        let transport_clone = transport.clone();
        let self_clone = Arc::new(self.clone());
        
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 2048];
            
            loop {
                // Receive packet
                match transport_clone.receive_packet(&mut buffer).await {
                    Ok((size, addr)) => {
                        // Process packet
                        if let Err(e) = self_clone.process_packet(&buffer[..size]).await {
                            warn!("Error processing packet: {}", e);
                        }
                    },
                    Err(e) => {
                        warn!("Error receiving packet: {}", e);
                        break;
                    }
                }
            }
            
            // Disconnected
            let mut connected = self_clone.connected.lock().await;
            *connected = false;
            
            // Notify callbacks
            let callbacks = self_clone.disconnect_callbacks.lock().await;
            for callback in &*callbacks {
                callback();
            }
        });
        
        Ok(())
    }
    
    async fn disconnect(&self) -> Result<(), MediaTransportError> {
        let mut connected = self.connected.lock().await;
        if !*connected {
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
        *connected = false;
        
        // Notify callbacks
        let callbacks = self.disconnect_callbacks.lock().await;
        for callback in &*callbacks {
            callback();
        }
        
        Ok(())
    }
    
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError> {
        // Check if connected
        if !self.is_connected().await? {
            return Err(MediaTransportError::NotConnected);
        }
        
        let mut session = self.session.lock().await;
        
        // Convert MediaFrame to RTP packets
        let timestamp = frame.timestamp;
        
        // Convert frame data to Bytes
        let data = bytes::Bytes::from(frame.data);
        
        // Send the frame through the session
        if let Err(e) = session.send_packet(timestamp, data, frame.marker).await {
            return Err(MediaTransportError::SendError(format!("Failed to send frame: {}", e)));
        }
        
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
        let connected = *self.connected.lock().await;
        Ok(connected)
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
} 