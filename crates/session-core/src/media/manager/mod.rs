//! Media Manager for Session-Core
//!
//! Main interface for media operations, using real MediaSessionController from media-core.
//! This manager coordinates between SIP sessions and media-core components.
//!
//! # Audio Muting
//!
//! The MediaManager supports silence-based muting through the `set_audio_muted` method.
//! When muted, RTP packets continue to flow but contain silence, maintaining:
//! - NAT traversal and keepalive
//! - Continuous sequence numbers
//! - Compatibility with all endpoints
//! - Instant mute/unmute without renegotiation

mod rtp_processing;
mod session_lifecycle;
mod audio_control;
mod srtp_setup;

use crate::api::types::SessionId;
use crate::errors::Result;
use super::types::*;
use super::MediaError;
use super::rtp_encoder;
use super::srtp_bridge::SrtpMediaBridge;
use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use tracing::warn;
use tokio::sync::{RwLock, Mutex, mpsc};
use async_trait::async_trait;

// Import RTP types from media-core (media-core provides the abstraction)
// session-core should NOT directly import from rtp-core - use media-core's abstractions
use rvoip_media_core::performance::pool::PoolStats;
use rvoip_media_core::{MediaSessionId as MediaCoreSessionId};
use rvoip_media_core::prelude::RtpPacket;
use crate::manager::events::SessionEventProcessor;

// Add integration imports for new codec detection and fallback systems
use rvoip_media_core::relay::controller::{
    codec_detection::{CodecDetector, CodecDetectionResult},
    codec_fallback::{CodecFallbackManager, FallbackMode, FallbackStats},
};
use rvoip_media_core::codec::mapping::CodecMapper;

// DTLS role re-export for callers
use rvoip_rtp_core::dtls::DtlsRole;
use rvoip_rtp_core::dtls::adapter::SrtpKeyMaterial;
use rvoip_rtp_core::transport::{RtpTransport, RtpTransportConfig, UdpRtpTransport};
use rvoip_rtp_core::transport::security_transport::SecurityRtpTransport;
use rvoip_rtp_core::srtp::SrtpContext;

// ICE types for NAT traversal — using the production webrtc-ice adapter
use rvoip_rtp_core::ice::{IceAgentAdapter, IceRole, IceCandidate, IceConnectionState, CandidateType, ComponentId};

/// Main media manager for session-core using real media-core components
pub struct MediaManager {
    /// Real MediaSessionController from media-core
    pub controller: Arc<MediaSessionController>,
    
    /// Session ID mapping (SIP SessionId -> Media DialogId)
    pub session_mapping: Arc<tokio::sync::RwLock<HashMap<SessionId, DialogId>>>,
    
    /// Default local bind address for media sessions
    pub local_bind_addr: SocketAddr,
    
    /// Zero-copy processing configuration per session
    pub zero_copy_config: Arc<tokio::sync::RwLock<HashMap<SessionId, ZeroCopyConfig>>>,
    
    /// Event processor for RTP processing events
    pub event_processor: Arc<SessionEventProcessor>,
    
    /// SDP storage per session
    pub sdp_storage: Arc<tokio::sync::RwLock<HashMap<SessionId, (Option<String>, Option<String>)>>>,
    
    /// Media configuration (codec preferences, etc.)
    pub media_config: MediaConfig,
    
    /// Codec detection system for handling unexpected codec formats
    pub codec_detector: Arc<CodecDetector>,
    
    /// Codec fallback manager for handling codec mismatches
    pub fallback_manager: Arc<CodecFallbackManager>,
    
    /// Codec mapper for payload type resolution
    pub codec_mapper: Arc<CodecMapper>,
    
    
    /// RTP payload encoder for converting AudioFrames to RTP packets
    pub rtp_encoder: Arc<Mutex<rtp_encoder::RtpPayloadEncoder>>,

    /// Sessions with active RTP processing
    pub rtp_processing_active: Arc<Mutex<HashSet<SessionId>>>,

    /// Per-session SRTP bridges (DTLS-SRTP encrypt/decrypt contexts).
    /// Populated after SDP negotiation indicates secure media.
    pub srtp_bridges: Arc<RwLock<HashMap<SessionId, Arc<Mutex<SrtpMediaBridge>>>>>,

    /// Sessions where SRTP was negotiated in SDP.  Used to prevent silent
    /// security downgrade to plain RTP when the bridge is missing (RFC 5764).
    srtp_required_sessions: Arc<RwLock<HashSet<SessionId>>>,

    /// SecurityRtpTransport instances for SRTP-enabled sessions.
    /// After DTLS handshake, the SRTP context is installed here.
    security_transports: Arc<RwLock<HashMap<SessionId, Arc<SecurityRtpTransport>>>>,

    /// Per-session ICE agents for NAT traversal.
    /// Created during `create_media_session` when ICE is enabled.
    pub ice_agents: Arc<RwLock<HashMap<SessionId, IceAgentAdapter>>>,
}

/// Configuration for zero-copy RTP processing per session
#[derive(Debug, Clone)]
pub struct ZeroCopyConfig {
    /// Whether zero-copy processing is enabled
    pub enabled: bool,
    /// Fallback to traditional processing on errors
    pub fallback_enabled: bool,
    /// Performance monitoring enabled
    pub monitoring_enabled: bool,
}

impl Default for ZeroCopyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_enabled: true,
            monitoring_enabled: true,
        }
    }
}

// Import RtpProcessingMetrics from types module
use super::types::{RtpProcessingMetrics, RtpProcessingType, RtpProcessingMode, RtpBufferPoolStats};

impl MediaManager {
    /// Create a new MediaManager with real MediaSessionController
    pub fn new(local_bind_addr: SocketAddr) -> Self {
        let event_processor = Arc::new(SessionEventProcessor::new());
        
        // Create codec systems with proper connections
        let codec_mapper = Arc::new(CodecMapper::new());
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        
        Self {
            controller: Arc::new(MediaSessionController::new()),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config: MediaConfig::default(),
            codec_detector,
            fallback_manager,
            codec_mapper,
            rtp_encoder: Arc::new(Mutex::new(rtp_encoder::RtpPayloadEncoder::new())),
            rtp_processing_active: Arc::new(Mutex::new(HashSet::new())),
            srtp_bridges: Arc::new(RwLock::new(HashMap::new())),
            srtp_required_sessions: Arc::new(RwLock::new(HashSet::new())),
            security_transports: Arc::new(RwLock::new(HashMap::new())),
            ice_agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a MediaManager with custom port range
    pub fn with_port_range(local_bind_addr: SocketAddr, base_port: u16, max_port: u16) -> Self {
        let event_processor = Arc::new(SessionEventProcessor::new());
        
        // Create codec systems with proper connections
        let codec_mapper = Arc::new(CodecMapper::new());
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        
        Self {
            controller: Arc::new(MediaSessionController::with_port_range(base_port, max_port)),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config: MediaConfig::default(),
            codec_detector,
            fallback_manager,
            codec_mapper,
            rtp_encoder: Arc::new(Mutex::new(rtp_encoder::RtpPayloadEncoder::new())),
            rtp_processing_active: Arc::new(Mutex::new(HashSet::new())),
            srtp_bridges: Arc::new(RwLock::new(HashMap::new())),
            srtp_required_sessions: Arc::new(RwLock::new(HashSet::new())),
            security_transports: Arc::new(RwLock::new(HashMap::new())),
            ice_agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a MediaManager with custom port range and media configuration
    pub fn with_port_range_and_config(
        local_bind_addr: SocketAddr, 
        base_port: u16, 
        max_port: u16, 
        media_config: MediaConfig
    ) -> Self {
        let event_processor = Arc::new(SessionEventProcessor::new());
        
        // Create codec systems with proper connections
        let codec_mapper = Arc::new(CodecMapper::new());
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        
        Self {
            controller: Arc::new(MediaSessionController::with_port_range(base_port, max_port)),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config,
            codec_detector,
            fallback_manager,
            codec_mapper,
            rtp_encoder: Arc::new(Mutex::new(rtp_encoder::RtpPayloadEncoder::new())),
            rtp_processing_active: Arc::new(Mutex::new(HashSet::new())),
            srtp_bridges: Arc::new(RwLock::new(HashMap::new())),
            srtp_required_sessions: Arc::new(RwLock::new(HashSet::new())),
            security_transports: Arc::new(RwLock::new(HashMap::new())),
            ice_agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the underlying MediaSessionController
    pub fn controller(&self) -> Arc<MediaSessionController> {
        self.controller.clone()
    }
    
    /// Get the event processor for RTP events
    pub fn event_processor(&self) -> Arc<SessionEventProcessor> {
        self.event_processor.clone()
    }
    
    /// Start the MediaManager and its event processor
    pub async fn start(&self) -> super::MediaResult<()> {
        self.event_processor.start().await
            .map_err(|e| MediaError::internal(&format!("Failed to start event processor: {}", e)))?;
        
        // Initialize RTP event integration to connect media-core RTP events to our decoder
        
        tracing::info!("✅ MediaManager started with event processing enabled");
        Ok(())
    }
    
    /// Stop the MediaManager and its event processor
    pub async fn stop(&self) -> super::MediaResult<()> {
        self.event_processor.stop().await
            .map_err(|e| MediaError::internal(&format!("Failed to stop event processor: {}", e)))?;
        
        tracing::info!("✅ MediaManager stopped");
        Ok(())
    }
}

impl std::fmt::Debug for MediaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManager")
            .field("local_bind_addr", &self.local_bind_addr)
            .field("session_mapping_count", &"<async>")
            .finish_non_exhaustive()
    }
}

/// Builder for MediaManager configuration
pub struct MediaManagerBuilder {
    local_bind_addr: Option<SocketAddr>,
    port_range: Option<(u16, u16)>,
}

impl MediaManagerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the local bind address for media sessions
    pub fn with_local_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.local_bind_addr = Some(addr);
        self
    }
    
    /// Set custom port range for RTP sessions
    pub fn with_port_range(mut self, base_port: u16, max_port: u16) -> Self {
        self.port_range = Some((base_port, max_port));
        self
    }
    
    /// Build the MediaManager
    pub fn build(self) -> MediaManager {
        let local_bind_addr = self.local_bind_addr
            .unwrap_or_else(|| std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
        
        if let Some((base_port, max_port)) = self.port_range {
            MediaManager::with_port_range(local_bind_addr, base_port, max_port)
        } else {
            MediaManager::new(local_bind_addr)
        }
    }
}

impl std::fmt::Debug for MediaManagerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManagerBuilder")
            .field("local_bind_addr", &self.local_bind_addr)
            .field("port_range", &self.port_range)
            .finish()
    }
}

impl Default for MediaManagerBuilder {
    fn default() -> Self {
        Self {
            local_bind_addr: None,
            port_range: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_media_manager_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::new(local_addr);
        
        assert_eq!(manager.get_local_bind_addr(), local_addr);
    }
    
    #[tokio::test]
    async fn test_media_session_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        let result = manager.create_media_session(&session_id).await;
        assert!(result.is_ok());
        
        let media_session = result.unwrap();
        assert!(!media_session.session_id.as_str().is_empty());
        assert!(media_session.local_rtp_port.is_some());
        
        // Verify session is tracked
        let sessions = manager.list_active_sessions().await;
        assert_eq!(sessions.len(), 1);
    }
    
    #[tokio::test]
    async fn test_sdp_generation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // First create a media session
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        
        // Then generate SDP
        let sdp = manager.generate_sdp_offer(&session_id).await;
        assert!(sdp.is_ok());
        
        let sdp_content = sdp.unwrap();
        assert!(sdp_content.contains("m=audio"));
        assert!(sdp_content.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp_content.contains("a=rtpmap:8 PCMA/8000"));
        
        // Verify SDP contains the allocated port from the media session
        let media_info = manager.get_media_info(&session_id).await.unwrap().unwrap();
        let allocated_port = media_info.local_rtp_port.unwrap();
        assert!(sdp_content.contains(&allocated_port.to_string())); // Should contain the actual allocated port
    }
    
    #[tokio::test]
    async fn test_media_session_termination() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // Create and then terminate session
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        assert_eq!(manager.list_active_sessions().await.len(), 1);
        
        let result = manager.terminate_media_session(&session_id).await;
        assert!(result.is_ok());
        
        // Verify session is removed
        assert_eq!(manager.list_active_sessions().await.len(), 0);
    }
    
    #[tokio::test]
    async fn test_zero_copy_rtp_processing_integration() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // Create media session first
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        
        // Test zero-copy configuration
        let result = manager.set_zero_copy_processing(&session_id, true).await;
        assert!(result.is_ok());
        
        let config = manager.get_zero_copy_config(&session_id).await;
        assert!(config.enabled);
        assert!(config.fallback_enabled);
        assert!(config.monitoring_enabled);
        
        // Test RTP buffer pool statistics
        let stats = manager.get_rtp_buffer_pool_stats();
        // Buffer pool should be initialized
        assert!(stats.total_allocated >= 0);
        
        // Test performance metrics (should return default values for now)
        let metrics = manager.get_rtp_processing_metrics(&session_id).await;
        assert!(metrics.is_ok());
        let metrics = metrics.unwrap();
        assert_eq!(metrics.allocation_reduction_percentage, 95.0); // Expected reduction
        
        // Cleanup
        let _cleanup = manager.terminate_media_session(&session_id).await;
    }
    
    #[tokio::test]
    async fn test_zero_copy_configuration_lifecycle() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // Create media session first
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        
        // Test custom zero-copy configuration
        let custom_config = ZeroCopyConfig {
            enabled: false,
            fallback_enabled: false,
            monitoring_enabled: true,
        };
        
        let result = manager.configure_zero_copy_processing(&session_id, custom_config.clone()).await;
        assert!(result.is_ok());
        
        let retrieved_config = manager.get_zero_copy_config(&session_id).await;
        assert!(!retrieved_config.enabled);
        assert!(!retrieved_config.fallback_enabled);
        assert!(retrieved_config.monitoring_enabled);
        
        // Verify cleanup removes configuration
        let _cleanup = manager.terminate_media_session(&session_id).await;
        
        // Config should be reset to default for new session
        let new_session_id = SessionId::new();
        let _new_session = manager.create_media_session(&new_session_id).await.unwrap();
        let default_config = manager.get_zero_copy_config(&new_session_id).await;
        assert!(default_config.enabled); // Should be default (true)
    }
}
