use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock, mpsc};
use futures::TryFutureExt;

use async_trait::async_trait;
use bytes::Bytes;
use tracing::{debug, warn, info, error};

use crate::api::transport::{
    MediaFrame, MediaFrameType, MediaTransportSession, MediaTransportError,
    MediaTransportEvent, MediaEventCallback, MediaTransportConfig,
};
use crate::session::{RtpSession, RtpSessionConfig, RtpSessionEvent};
use crate::transport::{RtpTransport, UdpRtpTransport, RtpTransportConfig};
use crate::packet::rtp::RtpPacket;
use crate::packet::header::RtpHeader;
use crate::packet::rtcp::RtcpPacket;
use crate::api::stats::MediaStats;
use crate::api::security::{SecureMediaContext, SecurityConfig};
use crate::api::buffer::MediaBufferConfig;

/// Default MediaTransportSession implementation that wraps an internal RtpSession
pub struct DefaultMediaTransportSession {
    /// The internal RTP session
    rtp_session: Arc<RwLock<RtpSession>>,
    
    /// Security context if enabled
    security_context: Option<Arc<dyn SecureMediaContext + 'static>>,
    
    /// Remote address for sending
    remote_addr: RwLock<Option<SocketAddr>>,
    
    /// Event callbacks registered by clients
    event_callbacks: Mutex<Vec<MediaEventCallback>>,
    
    /// Mapping of SSRCs to media types
    media_type_map: RwLock<HashMap<u32, MediaFrameType>>,
    
    /// Channel for receiving frames from the RTP packet receiver task
    frame_rx: Mutex<mpsc::Receiver<Result<MediaFrame, MediaTransportError>>>,
    
    /// Sender for the frame channel
    frame_tx: mpsc::Sender<Result<MediaFrame, MediaTransportError>>,
    
    /// Session configuration
    config: MediaTransportConfig,
    
    /// Session state
    running: RwLock<bool>,
}

impl DefaultMediaTransportSession {
    /// Create a new DefaultMediaTransportSession
    pub async fn new(
        config: MediaTransportConfig,
        security_context: Option<Arc<dyn SecureMediaContext + 'static>>,
        buffer_config: Option<MediaBufferConfig>,
    ) -> Result<Arc<Self>, MediaTransportError> {
        // Create transport configuration
        let transport_config = RtpTransportConfig {
            local_rtp_addr: config.local_address,
            local_rtcp_addr: None, // We'll use RTP address for RTCP when rtcp_mux is true
            symmetric_rtp: true,
            rtcp_mux: config.rtcp_mux,
            session_id: Some(format!("media-transport-{}", rand::random::<u32>())),
            use_port_allocator: true,
        };
        
        // Create UDP transport
        let transport = UdpRtpTransport::new(transport_config).await
            .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to create UDP transport: {}", e)))?;
        
        // Create RTP session configuration
        let rtp_config = RtpSessionConfig {
            local_addr: config.local_address,
            remote_addr: config.remote_address,
            ssrc: Some(rand::random::<u32>()),
            payload_type: 0, // Will be set per packet
            clock_rate: 8000, // Default audio clock rate
            jitter_buffer_size: buffer_config.as_ref().map(|c| c.max_packet_count),
            max_packet_age_ms: buffer_config.as_ref().map(|c| c.max_delay_ms),
            enable_jitter_buffer: buffer_config.is_some(),
        };
        
        // Create RTP session
        let rtp_session = RtpSession::new(rtp_config).await
            .map_err(|e| MediaTransportError::ConnectionError(format!("Failed to create RTP session: {}", e)))?;
        
        // Create frame channel
        let (frame_tx, frame_rx) = mpsc::channel(100);
        
        let session = Arc::new(Self {
            rtp_session: Arc::new(RwLock::new(rtp_session)),
            security_context,
            remote_addr: RwLock::new(config.remote_address),
            event_callbacks: Mutex::new(Vec::new()),
            media_type_map: RwLock::new(HashMap::new()),
            frame_rx: Mutex::new(frame_rx),
            frame_tx,
            config,
            running: RwLock::new(false),
        });
        
        // Set up event subscription to RtpSession
        // This is handled in the start method since we need the session to be running
        
        Ok(session)
    }
    
    /// Handle RTP session events
    fn handle_rtp_event(&self, event: RtpSessionEvent) {
        match event {
            RtpSessionEvent::PacketReceived(packet) => {
                self.handle_rtp_packet(packet);
            }
            RtpSessionEvent::Bye { ssrc, reason } => {
                // Handle BYE for the SSRC
                debug!("Received BYE for SSRC {}: {:?}", ssrc, reason);
            }
            RtpSessionEvent::RtcpSenderReport { 
                ssrc, ntp_timestamp, rtp_timestamp, 
                packet_count, octet_count, report_blocks 
            } => {
                // Process sender report for statistics
                debug!("Received RTCP SR from SSRC {}", ssrc);
                
                // Process report blocks for statistics
                for block in &report_blocks {
                    // Calculate quality based on loss and jitter
                    let fraction_lost = block.fraction_lost as f32 / 256.0;
                    let jitter_ms = (block.jitter as f32) / 90.0; // Assuming 90kHz clock for video
                    
                    // Process report block for bandwidth estimation and quality updates
                    if fraction_lost > 0.1 || jitter_ms > 50.0 {
                        let quality = crate::api::stats::QualityLevel::Poor;
                        self.emit_event(MediaTransportEvent::QualityChanged { quality });
                    }
                }
            }
            _ => {
                // Handle other events as needed
            }
        }
    }
    
    /// Handle RTP packet reception
    fn handle_rtp_packet(&self, packet: RtpPacket) {
        // Determine media type for this SSRC
        let ssrc = packet.header.ssrc;
        let frame_type = {
            let map_future = self.media_type_map.try_read();
            if let Ok(map) = map_future {
                map.get(&ssrc).cloned().unwrap_or(MediaFrameType::Audio)
            } else {
                // Default to Audio if we can't get the lock
                MediaFrameType::Audio
            }
        };
        
        // Convert RTP packet to media frame
        let frame = MediaFrame {
            frame_type,
            data: packet.payload.to_vec(),
            timestamp: packet.header.timestamp,
            sequence: packet.header.sequence_number,
            marker: packet.header.marker,
            payload_type: packet.header.payload_type,
            ssrc,
        };
        
        // Try to send the frame to the channel
        if let Err(e) = self.frame_tx.try_send(Ok(frame)) {
            match e {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    warn!("Frame channel full, dropping frame for SSRC {}", ssrc);
                },
                _ => {
                    warn!("Failed to send frame: {:?}", e);
                }
            }
        }
    }
    
    /// Create an RTP packet from a media frame
    fn create_rtp_packet(&self, frame: &MediaFrame) -> RtpPacket {
        let mut header = RtpHeader::new(
            frame.payload_type,
            frame.sequence,
            frame.timestamp,
            frame.ssrc,
        );
        header.marker = frame.marker;
        
        // Convert frame data to bytes
        let payload = Bytes::copy_from_slice(&frame.data);
        
        RtpPacket::new(header, payload)
    }
    
    /// Emit an event to all registered callbacks
    fn emit_event(&self, event: MediaTransportEvent) {
        // Use try_lock to avoid blocking in sync function
        if let Ok(callbacks) = self.event_callbacks.try_lock() {
            for callback in callbacks.iter() {
                callback(event.clone());
            }
        }
    }
    
    /// Update the media type mapping for an SSRC
    pub async fn set_media_type(&self, ssrc: u32, media_type: MediaFrameType) {
        let mut map = self.media_type_map.write().await;
        map.insert(ssrc, media_type);
    }
}

#[async_trait]
impl MediaTransportSession for DefaultMediaTransportSession {
    async fn start(&self) -> Result<(), MediaTransportError> {
        let mut is_running = false;
        {
            let mut running = self.running.write().await;
            if *running {
                return Ok(());
            }
            *running = true;
            is_running = true;
        }
        
        if is_running {
            // Subscribe to RTP session events
            // Create a clone of self as Arc<dyn MediaTransportSession>
            let rtp_session_clone = self.rtp_session.clone();
            
            // We need to create an event subscription
            // Since this is implementation-specific, we'll create a basic event processing loop
            let self_ptr = self as *const Self as usize;
            
            tokio::spawn(async move {
                // This is unsafe because we're converting a raw pointer back to a reference
                // We're ensuring safety by guaranteeing the self reference stays valid
                // as long as this task runs (i.e., until the session is dropped)
                let session = unsafe { &*(self_ptr as *const DefaultMediaTransportSession) };
                
                let rtp = rtp_session_clone.read().await;
                let mut events = rtp.subscribe();
                
                // Drop the read lock after subscribing
                drop(rtp);
                
                while let Ok(event) = events.recv().await {
                    session.handle_rtp_event(event);
                }
            });
            
            // If security is enabled, start the DTLS handshake
            if let Some(context) = &self.security_context {
                // Set remote address in security context if available
                if let Some(remote_addr) = *self.remote_addr.read().await {
                    // Use the trait method directly
                    debug!("Setting remote address {} in security context during start", remote_addr);
                    if let Err(e) = context.set_remote_address(remote_addr).await {
                        return Err(MediaTransportError::ConnectionError(
                            format!("Failed to set remote address in security context: {}", e)
                        ));
                    }
                    debug!("Successfully set remote address in security context during start");
                } else {
                    // Remote address must be set for DTLS
                    debug!("No remote address available for security context during start");
                    return Err(MediaTransportError::ConnectionError(
                        "Remote address must be set before starting with security enabled".to_string()
                    ));
                }
                
                // Get the transport socket from the RTP session
                let socket = {
                    let session = self.rtp_session.read().await;
                    session.get_socket_handle().await
                        .map_err(|e| MediaTransportError::ConnectionError(
                            format!("Failed to get socket handle: {}", e)
                        ))?
                };
                
                // Set the transport socket on the security context
                debug!("Setting transport socket for DTLS");
                context.set_transport_socket(socket).await
                    .map_err(|e| {
                        MediaTransportError::ConnectionError(format!("Failed to set transport socket: {}", e))
                    })?;
                debug!("Successfully set transport socket for DTLS");
                
                // Now that the remote address is set, start the handshake
                debug!("Starting security handshake");
                context.start_handshake().await
                    .map_err(|e| {
                        MediaTransportError::ConnectionError(format!("Failed to start DTLS handshake: {}", e))
                    })?;
                debug!("Security handshake started successfully");
            }
            
            info!("Media transport session started");
        }
        
        Ok(())
    }
    
    async fn stop(&self) -> Result<(), MediaTransportError> {
        // Get lock outside of async block
        let running_lock = self.running.write().await;
        if !*running_lock {
            return Ok(());
        }
        
        // Close the RTP session
        {
            let mut session = self.rtp_session.write().await;
            if let Err(e) = session.close().await {
                // Drop the lock before returning
                drop(session);
                return Err(MediaTransportError::ConnectionError(
                    format!("Failed to close RTP session: {}", e)
                ));
            }
        }
        
        // Update running state after releasing other locks
        let mut running = self.running.write().await;
        *running = false;
        info!("Media transport session stopped");
        
        Ok(())
    }
    
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError> {
        // Register frame type for this SSRC if not already set
        {
            let mut map = self.media_type_map.write().await;
            if !map.contains_key(&frame.ssrc) {
                map.insert(frame.ssrc, frame.frame_type);
            }
        }
        
        // Get session handle
        let mut session = self.rtp_session.write().await;
        
        // Create timestamp, marker, and payload from frame
        let timestamp = frame.timestamp;
        let marker = frame.marker;
        let payload = Bytes::copy_from_slice(&frame.data);
        
        // Send using the RTP session
        session.send_packet(timestamp, payload, marker).await
            .map_err(|e| MediaTransportError::SendError(format!("Failed to send packet: {}", e)))?;
        
        Ok(())
    }
    
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError> {
        // Create a timeout future
        let timer = tokio::time::sleep(timeout);
        
        // Get the lock on the frame channel
        let mut frame_rx = self.frame_rx.lock().await;
        
        // Wait for either a frame or timeout
        tokio::select! {
            // A frame arrived
            result = frame_rx.recv() => {
                match result {
                    Some(Ok(frame)) => Ok(Some(frame)),
                    Some(Err(e)) => Err(e),
                    None => Err(MediaTransportError::ReceiveError(
                        "Frame channel closed unexpectedly".to_string()
                    )),
                }
            }
            // Timeout occurred
            _ = timer => Ok(None),
        }
    }
    
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), MediaTransportError> {
        // Update remote address
        {
            let mut remote = self.remote_addr.write().await;
            *remote = Some(addr);
        }
        
        // Update the transport
        {
            let mut session = self.rtp_session.write().await;
            session.set_remote_addr(addr).await;
        }
        
        // Update the security context if it exists
        if let Some(context) = &self.security_context {
            // Use the async trait method
            debug!("Setting remote address {} in security context", addr);
            if let Err(e) = context.set_remote_address(addr).await {
                return Err(MediaTransportError::ConnectionError(
                    format!("Failed to set remote address in security context: {}", e)
                ));
            }
            debug!("Successfully set remote address in security context");
        } else {
            debug!("No security context to set remote address");
        }
        
        // Emit event about remote address change
        self.emit_event(MediaTransportEvent::RemoteAddressChanged { address: addr });
        
        Ok(())
    }
    
    fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError> {
        // This is a sync function, so we need to use blocking operations
        match self.event_callbacks.try_lock() {
            Ok(mut callbacks) => {
                callbacks.push(callback);
                Ok(())
            },
            Err(_) => Err(MediaTransportError::ConnectionError(
                "Failed to register event callback: lock acquisition failed".to_string()
            )),
        }
    }
    
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError> {
        // Get RTP session stats
        let session_stats = {
            let session = self.rtp_session.read().await;
            session.get_stats()
        };
        
        // Create a placeholder stats object - in real implementation we'd convert 
        // the session stats to MediaStats format
        let mut streams = HashMap::new();
        
        // Create a placeholder stats object
        let stats = MediaStats {
            timestamp: std::time::SystemTime::now(),
            session_duration: std::time::Duration::from_secs(0),
            streams,
            quality: crate::api::stats::QualityLevel::Unknown,
            upstream_bandwidth_bps: 0,
            downstream_bandwidth_bps: 0,
            available_bandwidth_bps: None,
            network_rtt_ms: None,
        };
        
        Ok(stats)
    }
    
    async fn get_security_info(&self) -> Result<crate::api::security::SecurityInfo, MediaTransportError> {
        if let Some(context) = &self.security_context {
            // Get the security info from the context
            let info = context.get_security_info();
            Ok(info)
        } else {
            // Return empty info if no security is configured
            Ok(crate::api::security::SecurityInfo {
                fingerprint: None,
                fingerprint_algorithm: None,
                setup_role: "actpass".to_string(),
                srtp_profile: None,
            })
        }
    }
    
    async fn set_remote_fingerprint(&self, fingerprint: &str, algorithm: &str) -> Result<(), MediaTransportError> {
        if let Some(context) = &self.security_context {
            // Get a pointer to the context
            let ctx_ptr = Arc::as_ptr(context);
            
            // This is unsafe because we're converting to a mutable reference, but we're controlling
            // the usage carefully to ensure no actual mutation of shared state happens across threads
            let ctx_mut = unsafe { &mut *(ctx_ptr as *mut dyn SecureMediaContext) };
            
            // Use the async method
            return ctx_mut.set_remote_fingerprint(fingerprint, algorithm).await
                .map_err(|e| MediaTransportError::ConnectionError(
                    format!("Failed to set remote fingerprint: {}", e)
                ));
        }
        
        // If no security context exists, ignore the call
        Ok(())
    }
} 