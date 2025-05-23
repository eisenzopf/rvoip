//! Default implementation of the client-side transport API
//!
//! This module contains the `DefaultMediaTransportClient` implementation
//! which combines all the functionality from the smaller module files.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::collections::HashMap;
use rand::Rng;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::{MediaTransportEvent, MediaEventCallback};
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::common::stats::{StreamStats, Direction, QualityLevel};
use crate::api::common::frame::MediaFrameType;
use crate::api::client::config::ClientConfig;
use crate::api::client::transport::MediaTransportClient;
use crate::api::client::transport::RtcpStats;
use crate::api::client::transport::VoipMetrics;
use crate::api::client::transport::MediaSyncInfo;
use crate::api::client::security::ClientSecurityContext;
use crate::api::client::security::DefaultClientSecurityContext;
use crate::session::{RtpSession, RtpSessionConfig};
use crate::transport::UdpRtpTransport;
use crate::api::common::extension::ExtensionFormat;
use crate::api::server::transport::HeaderExtension;
use crate::buffer::{
    GlobalBufferManager, BufferPool, TransmitBuffer, TransmitBufferConfig, 
    PacketPriority, TransmitBufferStats
};
use crate::{CsrcManager, CsrcMapping, RtpSsrc, RtpCsrc};

// Import module functions
use crate::api::client::transport::core::{connection, frame, events};
use crate::api::client::transport::media::{sync, csrc, extensions};
use crate::api::client::transport::rtcp::{reports, app_packets};
use crate::api::client::transport::security::client_security;
use crate::api::client::transport::buffer::{transmit, stats};

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
    
    /// Global buffer manager (only used if high-performance buffers are enabled)
    buffer_manager: Option<Arc<GlobalBufferManager>>,
    
    /// Transmit buffer for outgoing packets (only used if high-performance buffers are enabled)
    transmit_buffer: Arc<RwLock<Option<TransmitBuffer>>>,
    
    /// Buffer pool for packet allocation (only used if high-performance buffers are enabled)
    packet_pool: Option<Arc<BufferPool>>,
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
            max_packet_age_ms: Some(config.jitter_max_packet_age_ms as u32),
            enable_jitter_buffer: config.enable_jitter_buffer,
        };
        
        // Create RTP session
        let session = RtpSession::new(session_config).await
            .map_err(|e| MediaTransportError::InitializationError(format!("Failed to create RTP session: {}", e)))?;
            
        // Create security context if enabled
        let security_context = if config.security_config.security_mode.is_enabled() {
            match config.security_config.security_mode {
                crate::api::common::config::SecurityMode::Srtp => {
                    // Use SRTP-only context for pre-shared keys (no DTLS handshake)
                    let srtp_ctx = crate::api::client::security::srtp::SrtpClientSecurityContext::new(
                        config.security_config.clone(),
                    ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create SRTP security context: {}", e)))?;
                    
                    Some(srtp_ctx as Arc<dyn ClientSecurityContext>)
                },
                crate::api::common::config::SecurityMode::DtlsSrtp => {
                    // Use DTLS-SRTP context for handshake-based keys
                    let dtls_ctx = DefaultClientSecurityContext::new(
                        config.security_config.clone(),
                    ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create DTLS security context: {}", e)))?;
                    
                    Some(dtls_ctx as Arc<dyn ClientSecurityContext>)
                },
                crate::api::common::config::SecurityMode::SdesSrtp |
                crate::api::common::config::SecurityMode::MikeySrtp |
                crate::api::common::config::SecurityMode::ZrtpSrtp => {
                    // For now, treat these as DTLS-based (they would need specific implementations)
                    let dtls_ctx = DefaultClientSecurityContext::new(
                        config.security_config.clone(),
                    ).await.map_err(|e| MediaTransportError::Security(format!("Failed to create DTLS security context: {}", e)))?;
                    
                    Some(dtls_ctx as Arc<dyn ClientSecurityContext>)
                },
                crate::api::common::config::SecurityMode::None => {
                    // No security context
                    None
                }
            }
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
        
        // Initialize buffer-related components if high-performance buffers are enabled
        let (buffer_manager, transmit_buffer, packet_pool) = if config.high_performance_buffers_enabled {
            // Create buffer manager with configured limits
            let buffer_manager = Arc::new(GlobalBufferManager::new(config.buffer_limits.clone()));
            
            // Create shared buffer pools
            let pools = crate::buffer::SharedPools::new(1000); // 1000 initial packets
            let packet_pool = Arc::new(pools.medium);
            
            // Transmit buffer will be created when we connect and have an SSRC
            (Some(buffer_manager), Arc::new(RwLock::new(None)), Some(packet_pool))
        } else {
            (None, Arc::new(RwLock::new(None)), None)
        };
        
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
            buffer_manager,
            transmit_buffer,
            packet_pool,
        })
    }
    
    /// Access to the RTP session (for advanced usage in examples)
    pub async fn get_session(&self) -> Result<Arc<Mutex<crate::session::RtpSession>>, MediaTransportError> {
        Ok(Arc::clone(&self.session))
    }
}

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
            buffer_manager: self.buffer_manager.clone(),
            transmit_buffer: Arc::clone(&self.transmit_buffer),
            packet_pool: self.packet_pool.clone(),
        }
    }
}

#[async_trait]
impl MediaTransportClient for DefaultMediaTransportClient {
    // Core functionality
    
    async fn connect(&self) -> Result<(), MediaTransportError> {
        // Define the start_receive_task closure
        let session_clone = Arc::clone(&self.session);
        let frame_sender_clone = self.frame_sender.clone();
        let event_callbacks_clone = Arc::clone(&self.event_callbacks);
        let start_receive_task = move |transport: Arc<UdpRtpTransport>| -> Result<(), MediaTransportError> {
            // Start receive task implementation would be here
            Ok(())
        };
        
        connection::connect(
            self.config.remote_address,
            self.config.rtcp_mux,
            &self.security,
            connection::requires_dtls(self.config.security_config.security_mode),
            60, // Default 60 seconds timeout
            &self.connected,
            &self.transport,
            &self.connect_callbacks,
            start_receive_task,
        ).await?;
        
        // Initialize transmit buffer if high-performance buffers are enabled
        if self.config.high_performance_buffers_enabled {
            // Get SSRC from session
            let session = self.session.lock().await;
            let ssrc = session.get_ssrc();
            drop(session); // Release the lock early
            
            // Initialize the transmit buffer
            transmit::init_transmit_buffer(
                &self.buffer_manager,
                &self.packet_pool,
                &self.transmit_buffer,
                ssrc,
                self.config.transmit_buffer_config.clone(),
            ).await?;
        }
        
        Ok(())
    }
    
    async fn disconnect(&self) -> Result<(), MediaTransportError> {
        connection::disconnect(
            &self.security,
            &self.connected,
            &self.transport,
            &self.disconnect_callbacks,
        ).await
    }
    
    async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError> {
        connection::get_local_address(&self.transport).await
    }
    
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError> {
        frame::send_frame(
            frame,
            &self.connected,
            &self.session,
            &self.transport,
            &self.config,
            &self.sequence_numbers,
            self.config.remote_address,
            &self.csrc_manager,
            self.csrc_management_enabled.load(Ordering::SeqCst),
        ).await
    }
    
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError> {
        frame::receive_frame(
            timeout,
            &self.frame_receiver,
        ).await
    }
    
    async fn is_connected(&self) -> Result<bool, MediaTransportError> {
        Ok(connection::is_connected(&self.connected))
    }
    
    async fn on_connect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError> {
        events::register_connect_callback(
            &self.connect_callbacks,
            callback,
        ).await
    }
    
    async fn on_disconnect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError> {
        events::register_disconnect_callback(
            &self.disconnect_callbacks,
            callback,
        ).await
    }
    
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError> {
        events::register_event_callback(
            &self.event_callbacks,
            callback,
        ).await
    }
    
    // Stats and configuration
    
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError> {
        let mut stats = MediaStats::default();
        
        // Check if connected
        if !self.connected.load(Ordering::SeqCst) {
            return Ok(stats);
        }
        
        // Get session for stats
        let session = self.session.lock().await;
        let rtp_stats = session.get_stats();
        
        // Stream statistics
        let ssrcs = session.get_all_ssrcs().await;
        for ssrc in ssrcs {
            if let Some(stream_info) = session.get_stream(ssrc).await {
                // Create a stream stats entry
                let mut stream_stats = StreamStats {
                    direction: Direction::Outbound, // Default to outbound
                    ssrc,
                    media_type: MediaFrameType::Audio, // Default to audio
                    packet_count: stream_info.packets_received, // Using received as the count
                    byte_count: stream_info.bytes_received,
                    packets_lost: stream_info.packets_lost,
                    fraction_lost: if stream_info.packets_received > 0 {
                        stream_info.packets_lost as f32 / stream_info.packets_received as f32
                    } else {
                        0.0
                    },
                    jitter_ms: rtp_stats.jitter_ms as f32,
                    rtt_ms: None, // Not available yet
                    mos: None, // Not calculated yet
                    remote_addr: self.config.remote_address,
                    bitrate_bps: 0, // Would calculate if we tracked time between packets
                    discard_rate: 0.0,
                    quality: QualityLevel::Unknown, // Would be calculated based on stats
                    available_bandwidth_bps: None,
                };
                
                // Update the quality level based on this stream's metrics
                if stream_stats.fraction_lost > 0.05 {
                    stream_stats.quality = QualityLevel::Poor;
                    stats.quality = QualityLevel::Poor;
                } else if stream_stats.jitter_ms > 50.0 {
                    stream_stats.quality = QualityLevel::Fair;
                    stats.quality = QualityLevel::Fair;
                } else {
                    stream_stats.quality = QualityLevel::Good;
                    stats.quality = QualityLevel::Good;
                }
                
                // Add to our stats
                stats.streams.insert(ssrc, stream_stats);
                
                // For demo, we'll just use the first stream's stats
                break;
            }
        }
        
        // Set bandwidth values
        stats.upstream_bandwidth_bps = 0;
        stats.downstream_bandwidth_bps = 0;
        
        Ok(stats)
    }
    
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError> {
        client_security::get_security_info(&self.security).await
    }
    
    fn is_secure(&self) -> bool {
        client_security::is_secure(&self.security, self.config.security_config.security_mode.is_enabled())
    }
    
    async fn set_jitter_buffer_size(&self, size_ms: Duration) -> Result<(), MediaTransportError> {
        let mut session = self.session.lock().await;
        // This method doesn't exist in RtpSession but would in a real implementation
        // Just return Ok for now
        Ok(())
    }
    
    // RTCP functionality
    
    async fn send_rtcp_receiver_report(&self) -> Result<(), MediaTransportError> {
        reports::send_rtcp_receiver_report(
            &self.session,
            self.connected.load(Ordering::SeqCst),
        ).await
    }
    
    async fn send_rtcp_sender_report(&self) -> Result<(), MediaTransportError> {
        reports::send_rtcp_sender_report(
            &self.session,
            self.connected.load(Ordering::SeqCst),
        ).await
    }
    
    async fn get_rtcp_stats(&self) -> Result<RtcpStats, MediaTransportError> {
        reports::get_rtcp_stats(
            &self.session,
            self.connected.load(Ordering::SeqCst),
        ).await
    }
    
    async fn set_rtcp_interval(&self, interval: Duration) -> Result<(), MediaTransportError> {
        reports::set_rtcp_interval(
            &self.session,
            interval,
        ).await
    }
    
    async fn send_rtcp_app(&self, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError> {
        app_packets::send_rtcp_app(
            &self.session,
            &self.transport,
            self.config.remote_address,
            self.connected.load(Ordering::SeqCst),
            name,
            data,
        ).await
    }
    
    async fn send_rtcp_bye(&self, reason: Option<String>) -> Result<(), MediaTransportError> {
        app_packets::send_rtcp_bye(
            &self.session,
            &self.transport,
            self.config.remote_address,
            self.connected.load(Ordering::SeqCst),
            reason,
        ).await
    }
    
    async fn send_rtcp_xr_voip_metrics(&self, metrics: VoipMetrics) -> Result<(), MediaTransportError> {
        app_packets::send_rtcp_xr_voip_metrics(
            &self.session,
            &self.transport,
            self.config.remote_address,
            self.connected.load(Ordering::SeqCst),
            metrics,
        ).await
    }
    
    // Media synchronization
    
    async fn enable_media_sync(&self) -> Result<bool, MediaTransportError> {
        // Get session to get SSRC and clock rate
        let session = self.session.lock().await;
        let ssrc = session.get_ssrc();
        let clock_rate = self.config.clock_rate; // Use from config since session doesn't expose this
        
        sync::enable_media_sync(
            &self.media_sync_enabled,
            &self.media_sync,
            ssrc,
            clock_rate,
        ).await
    }
    
    async fn is_media_sync_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(sync::is_media_sync_enabled(&self.media_sync_enabled))
    }
    
    async fn register_sync_stream(&self, ssrc: u32, clock_rate: u32) -> Result<(), MediaTransportError> {
        sync::register_sync_stream(
            &self.media_sync,
            ssrc,
            clock_rate,
        ).await
    }
    
    async fn set_sync_reference_stream(&self, ssrc: u32) -> Result<(), MediaTransportError> {
        sync::set_sync_reference_stream(
            &self.media_sync,
            ssrc,
        ).await
    }
    
    async fn get_sync_info(&self, ssrc: u32) -> Result<Option<MediaSyncInfo>, MediaTransportError> {
        sync::get_sync_info(
            &self.media_sync,
            ssrc,
        ).await
    }
    
    async fn get_all_sync_info(&self) -> Result<HashMap<u32, MediaSyncInfo>, MediaTransportError> {
        sync::get_all_sync_info(
            &self.media_sync,
        ).await
    }
    
    async fn convert_timestamp(&self, from_ssrc: u32, to_ssrc: u32, rtp_ts: u32) -> Result<Option<u32>, MediaTransportError> {
        sync::convert_timestamp(
            &self.media_sync,
            from_ssrc,
            to_ssrc,
            rtp_ts,
        ).await
    }
    
    async fn rtp_to_ntp(&self, ssrc: u32, rtp_ts: u32) -> Result<Option<crate::packet::rtcp::NtpTimestamp>, MediaTransportError> {
        sync::rtp_to_ntp(
            &self.media_sync,
            ssrc,
            rtp_ts,
        ).await
    }
    
    async fn ntp_to_rtp(&self, ssrc: u32, ntp: crate::packet::rtcp::NtpTimestamp) -> Result<Option<u32>, MediaTransportError> {
        sync::ntp_to_rtp(
            &self.media_sync,
            ssrc,
            ntp,
        ).await
    }
    
    async fn get_clock_drift_ppm(&self, ssrc: u32) -> Result<Option<f64>, MediaTransportError> {
        sync::get_clock_drift_ppm(
            &self.media_sync,
            ssrc,
        ).await
    }
    
    async fn are_streams_synchronized(&self, ssrc1: u32, ssrc2: u32, tolerance_ms: f64) -> Result<bool, MediaTransportError> {
        sync::are_streams_synchronized(
            &self.media_sync,
            ssrc1,
            ssrc2,
            tolerance_ms,
        ).await
    }
    
    // CSRC management
    
    async fn is_csrc_management_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(csrc::is_csrc_management_enabled(&self.csrc_management_enabled))
    }
    
    async fn enable_csrc_management(&self) -> Result<bool, MediaTransportError> {
        csrc::enable_csrc_management(&self.csrc_management_enabled).await
    }
    
    async fn add_csrc_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaTransportError> {
        csrc::add_csrc_mapping(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            mapping,
        ).await
    }
    
    async fn add_simple_csrc_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) -> Result<(), MediaTransportError> {
        csrc::add_simple_csrc_mapping(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
            csrc,
        ).await
    }
    
    async fn remove_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        csrc::remove_csrc_mapping_by_ssrc(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
        ).await
    }
    
    async fn get_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) -> Result<Option<CsrcMapping>, MediaTransportError> {
        csrc::get_csrc_mapping_by_ssrc(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            original_ssrc,
        ).await
    }
    
    async fn get_all_csrc_mappings(&self) -> Result<Vec<CsrcMapping>, MediaTransportError> {
        csrc::get_all_csrc_mappings(
            &self.csrc_management_enabled,
            &self.csrc_manager,
        ).await
    }
    
    async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc]) -> Result<Vec<RtpCsrc>, MediaTransportError> {
        csrc::get_active_csrcs(
            &self.csrc_management_enabled,
            &self.csrc_manager,
            active_ssrcs,
        ).await
    }
    
    // Header extensions
    
    async fn is_header_extensions_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(extensions::is_header_extensions_enabled(self.config.header_extensions_enabled))
    }
    
    async fn enable_header_extensions(&self, format: ExtensionFormat) -> Result<bool, MediaTransportError> {
        extensions::enable_header_extensions(format).await
    }
    
    async fn configure_header_extension(&self, id: u8, uri: String) -> Result<(), MediaTransportError> {
        extensions::configure_header_extension(id, uri).await
    }
    
    async fn configure_header_extensions(&self, mappings: HashMap<u8, String>) -> Result<(), MediaTransportError> {
        extensions::configure_header_extensions(mappings).await
    }
    
    async fn add_header_extension(&self, extension: HeaderExtension) -> Result<(), MediaTransportError> {
        extensions::add_header_extension(extension).await
    }
    
    async fn add_audio_level_extension(&self, voice_activity: bool, level: u8) -> Result<(), MediaTransportError> {
        extensions::add_audio_level_extension(voice_activity, level).await
    }
    
    async fn add_video_orientation_extension(&self, camera_front_facing: bool, camera_flipped: bool, rotation: u16) -> Result<(), MediaTransportError> {
        extensions::add_video_orientation_extension(camera_front_facing, camera_flipped, rotation).await
    }
    
    async fn add_transport_cc_extension(&self, sequence_number: u16) -> Result<(), MediaTransportError> {
        extensions::add_transport_cc_extension(sequence_number).await
    }
    
    async fn get_received_header_extensions(&self) -> Result<Vec<HeaderExtension>, MediaTransportError> {
        extensions::get_received_header_extensions().await
    }
    
    async fn get_received_audio_level(&self) -> Result<Option<(bool, u8)>, MediaTransportError> {
        extensions::get_received_audio_level().await
    }
    
    async fn get_received_video_orientation(&self) -> Result<Option<(bool, bool, u16)>, MediaTransportError> {
        extensions::get_received_video_orientation().await
    }
    
    async fn get_received_transport_cc(&self) -> Result<Option<u16>, MediaTransportError> {
        extensions::get_received_transport_cc().await
    }
    
    // Buffer management
    
    async fn send_frame_with_priority(&self, frame: MediaFrame, priority: PacketPriority) -> Result<(), MediaTransportError> {
        // Fallback function for regular sending
        let self_clone = self.clone();
        let fallback_send = move |f: MediaFrame| -> Result<(), MediaTransportError> {
            // Clone the frame and self_clone for the closure
            let frame_clone = f.clone();
            let inner_self_clone = self_clone.clone();
            
            // Spawn a task to send the frame
            tokio::spawn(async move {
                inner_self_clone.send_frame(frame_clone).await
            });
            
            // Return Ok immediately - the actual send happens in the background
            Ok(())
        };
        
        transmit::send_frame_with_priority(
            frame,
            priority,
            self.config.high_performance_buffers_enabled,
            &self.transmit_buffer,
            &self.transport,
            self.config.remote_address,
            fallback_send,
        ).await
    }
    
    async fn get_transmit_buffer_stats(&self) -> Result<TransmitBufferStats, MediaTransportError> {
        stats::get_transmit_buffer_stats(
            self.config.high_performance_buffers_enabled,
            &self.transmit_buffer,
        ).await
    }
    
    async fn update_transmit_buffer_config(&self, config: TransmitBufferConfig) -> Result<(), MediaTransportError> {
        transmit::update_transmit_buffer_config(
            self.config.high_performance_buffers_enabled,
            &self.transmit_buffer,
            config,
        ).await
    }
    
    async fn set_priority_threshold(&self, buffer_fullness: f32, priority: PacketPriority) -> Result<(), MediaTransportError> {
        transmit::set_priority_threshold(
            self.config.high_performance_buffers_enabled,
            &self.transmit_buffer,
            buffer_fullness,
            priority,
        ).await
    }
}

// Additional methods not part of the MediaTransportClient trait
impl DefaultMediaTransportClient {
    /// Check if SSRC demultiplexing is enabled
    pub async fn is_ssrc_demultiplexing_enabled(&self) -> Result<bool, MediaTransportError> {
        Ok(crate::api::client::transport::media::ssrc::is_ssrc_demultiplexing_enabled(
            &self.ssrc_demultiplexing_enabled
        ))
    }
    
    /// Register an SSRC for demultiplexing
    pub async fn register_ssrc(&self, ssrc: u32) -> Result<bool, MediaTransportError> {
        crate::api::client::transport::media::ssrc::register_ssrc(
            ssrc,
            &self.session,
            &self.ssrc_demultiplexing_enabled
        ).await
    }
    
    /// Get the sequence number for a specific SSRC
    pub async fn get_sequence_number(&self, ssrc: u32) -> Result<u16, MediaTransportError> {
        crate::api::client::transport::media::ssrc::get_sequence_number(
            ssrc,
            &self.session,
            &self.sequence_numbers
        ).await
    }
    
    /// Get all registered SSRCs
    pub async fn get_all_ssrcs(&self) -> Result<Vec<u32>, MediaTransportError> {
        crate::api::client::transport::media::ssrc::get_all_ssrcs(
            &self.session
        ).await
    }
    
    /// Update CSRC CNAME for a specific SSRC
    pub async fn update_csrc_cname(&self, ssrc: u32, cname: String) -> Result<(), MediaTransportError> {
        // First check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Get the csrc manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        
        // Check if we already have a mapping
        if csrc_manager.update_cname(ssrc, cname.clone()) {
            // Mapping updated
            return Ok(());
        }
        
        // Create a new mapping if it doesn't exist
        let csrc = (csrc_manager.len() as u32) % 15; // Use an available CSRC ID
        let mapping = CsrcMapping::with_cname(ssrc, csrc, cname);
        csrc_manager.add_mapping(mapping);
        
        Ok(())
    }
    
    /// Update CSRC display name for a specific SSRC
    pub async fn update_csrc_display_name(&self, ssrc: u32, name: String) -> Result<(), MediaTransportError> {
        // First check if CSRC management is enabled
        if !self.csrc_management_enabled.load(Ordering::SeqCst) {
            return Err(MediaTransportError::ConfigError("CSRC management is not enabled".to_string()));
        }
        
        // Get the csrc manager
        let mut csrc_manager = self.csrc_manager.lock().await;
        
        // Check if we already have a mapping
        if csrc_manager.update_display_name(ssrc, name.clone()) {
            // Mapping updated
            return Ok(());
        }
        
        // Create a new mapping if it doesn't exist
        let csrc = (csrc_manager.len() as u32) % 15; // Use an available CSRC ID
        let mut mapping = CsrcMapping::new(ssrc, csrc);
        mapping.display_name = Some(name);
        csrc_manager.add_mapping(mapping);
        
        Ok(())
    }
} 