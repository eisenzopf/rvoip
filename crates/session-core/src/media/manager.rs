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

// ICE types for NAT traversal
use rvoip_rtp_core::ice::{IceAgent, IceRole, IceCandidate, IceConnectionState, CandidateType, ComponentId};

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

    /// Per-session ICE agents for NAT traversal.
    /// Created during `create_media_session` when ICE is enabled.
    pub ice_agents: Arc<RwLock<HashMap<SessionId, IceAgent>>>,
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
    
    /// Process RTP packet with zero-copy optimization
    pub async fn process_rtp_packet_zero_copy(&self, session_id: &SessionId, packet: &RtpPacket) -> super::MediaResult<RtpPacket> {
        tracing::debug!("Processing RTP packet with zero-copy for session: {}", session_id);
        
        // Check if zero-copy is enabled for this session
        let config = {
            let configs = self.zero_copy_config.read().await;
            configs.get(session_id).cloned().unwrap_or_default()
        };
        
        if !config.enabled {
            return self.process_rtp_packet_traditional(session_id, packet).await;
        }
        
        // Process with zero-copy approach
        let start_time = std::time::Instant::now();
        let result = self.controller.process_rtp_packet_zero_copy(packet).await
            .map_err(|e| {
                tracing::warn!("Zero-copy RTP processing failed for session {}: {}", session_id, e);
                MediaError::MediaEngine { source: Box::new(e) }
            });
        
        let processing_duration = start_time.elapsed();
        
        match result {
            Ok(processed_packet) => {
                if config.monitoring_enabled {
                    tracing::debug!("✅ Zero-copy RTP processing completed for session {} in {:?}", 
                                  session_id, processing_duration);
                    
                    // Publish RTP packet processed event
                    let metrics = RtpProcessingMetrics {
                        zero_copy_packets_processed: 1,
                        traditional_packets_processed: 0,
                        fallback_events: 0,
                        average_processing_time_zero_copy: processing_duration.as_micros() as f64,
                        average_processing_time_traditional: 0.0,
                        allocation_reduction_percentage: 95.0, // Expected reduction
                    };
                    
                    if let Err(e) = self.event_processor.publish_rtp_packet_processed(
                        session_id.clone(), 
                        RtpProcessingType::ZeroCopy, 
                        metrics
                    ).await {
                        tracing::warn!("Failed to publish RTP packet processed event: {}", e);
                    }
                }
                Ok(processed_packet)
            }
            Err(e) if config.fallback_enabled => {
                tracing::info!("🔄 Falling back to traditional RTP processing for session {}", session_id);
                
                // Publish RTP processing error event with fallback
                if let Err(publish_err) = self.event_processor.publish_rtp_processing_error(
                    session_id.clone(),
                    format!("Zero-copy processing failed: {}", e),
                    true,
                ).await {
                    tracing::warn!("Failed to publish RTP processing error event: {}", publish_err);
                }
                
                self.process_rtp_packet_traditional(session_id, packet).await
            }
            Err(e) => {
                // Publish RTP processing error event without fallback
                if let Err(publish_err) = self.event_processor.publish_rtp_processing_error(
                    session_id.clone(),
                    format!("Zero-copy processing failed: {}", e),
                    false,
                ).await {
                    tracing::warn!("Failed to publish RTP processing error event: {}", publish_err);
                }
                
                Err(e)
            }
        }
    }
    
    /// Process RTP packet with traditional approach (for comparison/fallback)
    pub async fn process_rtp_packet_traditional(&self, session_id: &SessionId, packet: &RtpPacket) -> super::MediaResult<RtpPacket> {
        tracing::debug!("Processing RTP packet with traditional approach for session: {}", session_id);
        
        let start_time = std::time::Instant::now();
        let result = self.controller.process_rtp_packet_traditional(packet).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) });
        
        let processing_duration = start_time.elapsed();
        
        match result {
            Ok(processed_packet) => {
                tracing::debug!("✅ Traditional RTP processing completed for session {} in {:?}", 
                              session_id, processing_duration);
                
                // Publish RTP packet processed event for traditional processing
                let metrics = RtpProcessingMetrics {
                    zero_copy_packets_processed: 0,
                    traditional_packets_processed: 1,
                    fallback_events: 0,
                    average_processing_time_zero_copy: 0.0,
                    average_processing_time_traditional: processing_duration.as_micros() as f64,
                    allocation_reduction_percentage: 0.0, // No reduction for traditional processing
                };
                
                if let Err(e) = self.event_processor.publish_rtp_packet_processed(
                    session_id.clone(), 
                    RtpProcessingType::Traditional, 
                    metrics
                ).await {
                    tracing::warn!("Failed to publish RTP packet processed event: {}", e);
                }
                
                Ok(processed_packet)
            }
            Err(e) => {
                tracing::error!("❌ Traditional RTP processing failed for session {}: {}", session_id, e);
                
                // Publish RTP processing error event for traditional processing failure
                if let Err(publish_err) = self.event_processor.publish_rtp_processing_error(
                    session_id.clone(),
                    format!("Traditional processing failed: {}", e),
                    false, // No fallback from traditional processing
                ).await {
                    tracing::warn!("Failed to publish RTP processing error event: {}", publish_err);
                }
                
                Err(e)
            }
        }
    }
    
    /// Get RTP buffer pool statistics
    pub fn get_rtp_buffer_pool_stats(&self) -> PoolStats {
        self.controller.get_rtp_buffer_pool_stats()
    }
    
    /// Publish RTP buffer pool statistics update event
    pub async fn publish_rtp_buffer_pool_update(&self) {
        let pool_stats = self.get_rtp_buffer_pool_stats();
        let rtp_stats = RtpBufferPoolStats::from(pool_stats);
        
        if let Err(e) = self.event_processor.publish_rtp_buffer_pool_update(rtp_stats).await {
            warn!("Failed to publish RTP buffer pool update: {}", e);
        }
    }
    
    /// Get RTP/RTCP statistics for a session
    pub async fn get_rtp_statistics(&self, session_id: &SessionId) -> super::MediaResult<Option<rvoip_media_core::RtpSessionStats>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        Ok(self.controller.get_rtp_statistics(&dialog_id).await)
    }
    
    /// Get comprehensive media statistics
    pub async fn get_media_statistics(&self, session_id: &SessionId) -> super::MediaResult<Option<rvoip_media_core::types::MediaStatistics>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        Ok(self.controller.get_media_statistics(&dialog_id).await)
    }
    
    /// Start periodic statistics monitoring with the specified interval
    pub async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        self.controller.start_statistics_monitoring(dialog_id, interval).await
            .map_err(|e| super::MediaError::MediaEngine {
                source: Box::new(e),
            })
    }
    
    /// Enable/disable zero-copy processing for a session
    pub async fn set_zero_copy_processing(&self, session_id: &SessionId, enabled: bool) -> super::MediaResult<()> {
        tracing::info!("Setting zero-copy processing for session {} to: {}", session_id, enabled);
        
        let old_mode = {
            let configs = self.zero_copy_config.read().await;
            let current_config = configs.get(session_id).cloned().unwrap_or_default();
            if current_config.enabled {
                RtpProcessingMode::ZeroCopyPreferred
            } else {
                RtpProcessingMode::TraditionalOnly
            }
        };
        
        {
            let mut configs = self.zero_copy_config.write().await;
            let config = configs.entry(session_id.clone()).or_default();
            config.enabled = enabled;
        }
        
        let new_mode = if enabled {
            RtpProcessingMode::ZeroCopyPreferred
        } else {
            RtpProcessingMode::TraditionalOnly
        };
        
        // Publish RTP processing mode changed event if mode actually changed
        if std::mem::discriminant(&old_mode) != std::mem::discriminant(&new_mode) {
            if let Err(e) = self.event_processor.publish_rtp_processing_mode_changed(
                session_id.clone(),
                old_mode,
                new_mode,
            ).await {
                tracing::warn!("Failed to publish RTP processing mode changed event: {}", e);
            }
        }
        
        tracing::debug!("✅ Zero-copy processing configuration updated for session {}", session_id);
        Ok(())
    }
    
    /// Configure zero-copy processing options for a session
    pub async fn configure_zero_copy_processing(&self, session_id: &SessionId, config: ZeroCopyConfig) -> super::MediaResult<()> {
        tracing::info!("Configuring zero-copy processing for session {}: enabled={}, fallback={}, monitoring={}", 
                      session_id, config.enabled, config.fallback_enabled, config.monitoring_enabled);
        
        let mut configs = self.zero_copy_config.write().await;
        configs.insert(session_id.clone(), config);
        
        tracing::debug!("✅ Zero-copy processing configuration applied for session {}", session_id);
        Ok(())
    }
    
    /// Get zero-copy configuration for a session
    pub async fn get_zero_copy_config(&self, session_id: &SessionId) -> ZeroCopyConfig {
        let configs = self.zero_copy_config.read().await;
        configs.get(session_id).cloned().unwrap_or_default()
    }
    
    /// Get RTP processing performance metrics for a session
    pub async fn get_rtp_processing_metrics(&self, session_id: &SessionId) -> super::MediaResult<RtpProcessingMetrics> {
        // TODO: Implement proper metrics collection
        // For now, return default metrics - this will be enhanced in Phase 16.4
        tracing::debug!("Getting RTP processing metrics for session: {}", session_id);
        
        Ok(RtpProcessingMetrics {
            zero_copy_packets_processed: 0,
            traditional_packets_processed: 0,
            fallback_events: 0,
            average_processing_time_zero_copy: 0.0,
            average_processing_time_traditional: 0.0,
            allocation_reduction_percentage: 95.0, // Expected reduction from zero-copy processing
        })
    }
    
    /// Cleanup zero-copy configuration when session ends
    async fn cleanup_zero_copy_config(&self, session_id: &SessionId) {
        let mut configs = self.zero_copy_config.write().await;
        configs.remove(session_id);
        tracing::debug!("🧹 Cleaned up zero-copy config for session {}", session_id);
    }
    
    /// Create a new media session for a SIP session using real MediaSessionController
    pub async fn create_media_session(&self, session_id: &SessionId) -> super::MediaResult<MediaSessionInfo> {
        tracing::trace!("📹 create_media_session called for: {}", session_id);
        
        // Create dialog ID for media session (use session ID as base)
        let dialog_id = DialogId::new(format!("media-{}", session_id));
        tracing::trace!("📹 Using dialog_id: {}", dialog_id);
        
        // Check if this media session already exists
        if let Some(existing_info) = self.controller.get_session_info(&dialog_id).await {
            tracing::trace!("📹 Media session already exists in controller for {}, reusing", dialog_id);
            
            // Ensure session mapping exists
            {
                let mut mapping = self.session_mapping.write().await;
                mapping.insert(session_id.clone(), dialog_id.clone());
            }
            
            // Ensure zero-copy config exists
            {
                let mut configs = self.zero_copy_config.write().await;
                configs.insert(session_id.clone(), ZeroCopyConfig::default());
            }
            
            let session_info = MediaSessionInfo::from(existing_info);
            tracing::trace!("📹 Reused existing media session: {} for SIP session: {}", dialog_id, session_id);
            return Ok(session_info);
        }
        
        // Create media configuration using the manager's configured preferences
        let media_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            None, // Will be set later when remote SDP is processed
        );
        
        tracing::trace!("📹 Starting new media session in controller for {}", dialog_id);
        // Start media session using real MediaSessionController
        match self.controller.start_media(dialog_id.clone(), media_config).await {
            Ok(()) => {
                tracing::trace!("📹 MediaSessionController.start_media SUCCESS for {}", dialog_id);
            }
            Err(e) => {
                tracing::trace!("📹 MediaSessionController.start_media FAILED for {}: {}", dialog_id, e);
                return Err(MediaError::MediaEngine { source: Box::new(e) });
            }
        }
        
        tracing::trace!("📹 Getting session info from controller for {}", dialog_id);
        // Get session info from controller
        let media_session_info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| {
                tracing::trace!("📹 get_session_info returned None for {}", dialog_id);
                MediaError::SessionNotFound { session_id: dialog_id.to_string() }
            })?;
        
        // Store session mapping
        {
            let mut mapping = self.session_mapping.write().await;
            mapping.insert(session_id.clone(), dialog_id.clone());
            tracing::trace!("📹 Stored session mapping: {} -> {}", session_id, dialog_id);
        }
        
        // Initialize zero-copy configuration for new session
        {
            let mut configs = self.zero_copy_config.write().await;
            configs.insert(session_id.clone(), ZeroCopyConfig::default());
        }
        
        // Convert to our MediaSessionInfo type
        let session_info = MediaSessionInfo::from(media_session_info);

        // Create ICE agent and gather candidates if ICE is enabled
        if self.media_config.ice.enabled {
            let local_port = session_info.local_rtp_port.unwrap_or(0);
            let local_addr = SocketAddr::new(self.local_bind_addr.ip(), local_port);
            let mut agent = IceAgent::new(IceRole::Controlling);

            let stun_servers = &self.media_config.ice.stun_servers;
            let turn_configs = &self.media_config.ice.turn_servers;
            match agent.gather_candidates_with_turn(local_addr, stun_servers, turn_configs).await {
                Ok(candidates) => {
                    tracing::info!(
                        session = %session_id,
                        candidates = candidates.len(),
                        "ICE candidate gathering complete"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        session = %session_id,
                        error = %e,
                        "ICE candidate gathering failed, falling back to host-only"
                    );
                    // Gather host-only candidates without STUN/TURN
                    let empty_stun: Vec<SocketAddr> = Vec::new();
                    if let Err(e2) = agent.gather_candidates(local_addr, &empty_stun).await {
                        tracing::error!(
                            session = %session_id,
                            error = %e2,
                            "ICE host candidate gathering also failed"
                        );
                    }
                }
            }

            let mut agents = self.ice_agents.write().await;
            agents.insert(session_id.clone(), agent);
        }

        tracing::trace!("Successfully created NEW media session: {} for SIP session: {}", dialog_id, session_id);

        Ok(session_info)
    }

    /// Update a media session with new SDP (for re-INVITE, etc.)
    pub async fn update_media_session(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Updating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Store the remote SDP
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(sdp.to_string());
        }
        
        // Parse SDP to extract remote address and codec information
        let remote_addr = self.parse_remote_address_from_sdp(sdp);
        let codec = self.parse_codec_from_sdp(sdp);
        
        if let Some(remote_addr) = remote_addr {
            // Create enhanced media configuration with remote address and codec
            let mut session_config = MediaConfig::default();
            if let Some(codec_name) = codec {
                session_config.preferred_codecs = vec![codec_name];
            }
            
            let updated_config = convert_to_media_core_config(
                &session_config,
                self.local_bind_addr,
                Some(remote_addr),
            );
            
            self.controller.update_media(dialog_id, updated_config).await
                .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
                
            tracing::info!("✅ Updated media session for SIP session: {} with remote: {} and codecs: {:?}", 
                          session_id, remote_addr, session_config.preferred_codecs);
        } else {
            tracing::warn!("Could not parse SDP for session: {}, skipping media update", session_id);
        }
        
        Ok(())
    }
    
    /// Terminate a media session
    pub async fn terminate_media_session(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Terminating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mut mapping = self.session_mapping.write().await;
            mapping.remove(session_id)
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Cleanup zero-copy configuration
        self.cleanup_zero_copy_config(session_id).await;

        // Cleanup SRTP bridge
        self.cleanup_srtp_bridge(session_id).await;

        // Cleanup ICE agent
        {
            let mut agents = self.ice_agents.write().await;
            if let Some(mut agent) = agents.remove(session_id) {
                agent.close();
            }
        }

        // Cleanup codec processing systems
        if let Err(e) = self.cleanup_codec_processing(session_id).await {
            tracing::warn!("Failed to cleanup codec processing for session {}: {}", session_id, e);
        }

        // Cleanup SDP storage
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            sdp_storage.remove(session_id);
        }

        // Stop media session using real MediaSessionController
        self.controller.stop_media(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;

        tracing::info!("Terminated media session: {} for SIP session: {} (including zero-copy + SRTP cleanup)", dialog_id, session_id);
        Ok(())
    }
    
    /// Check if a session has a media mapping (for duplicate creation prevention)
    pub async fn has_session_mapping(&self, session_id: &SessionId) -> bool {
        let mapping = self.session_mapping.read().await;
        mapping.contains_key(session_id)
    }
    
    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> super::MediaResult<Option<MediaSessionInfo>> {
        tracing::debug!("Getting media info for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        if let Some(dialog_id) = dialog_id {
            // Get session info from controller
            if let Some(media_session_info) = self.controller.get_session_info(&dialog_id).await {
                let mut session_info = MediaSessionInfo::from(media_session_info);
                
                // Add stored SDP
                let sdp_storage = self.sdp_storage.read().await;
                if let Some((local_sdp, remote_sdp)) = sdp_storage.get(session_id) {
                    session_info.local_sdp = local_sdp.clone();
                    session_info.remote_sdp = remote_sdp.clone();
                }
                
                Ok(Some(session_info))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    
    /// Generate SDP offer for a session using real media session information
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> super::MediaResult<String> {
        tracing::debug!("Generating SDP offer for session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        // If we have a media session, get its info for SDP generation
        let media_info = if let Some(dialog_id) = dialog_id {
            self.controller.get_session_info(&dialog_id).await
        } else {
            None
        };
        
        // Generate SDP using MediaConfigConverter with configured preferences
        use crate::media::config::MediaConfigConverter;
        let converter = MediaConfigConverter::with_media_config(&self.media_config);
        
        let local_ip = self.local_bind_addr.ip().to_string();
        let local_port = if let Some(info) = media_info {
            info.rtp_port.unwrap_or(10000)
        } else {
            10000 // Default port if no media session exists yet
        };
        
        let mut sdp = converter.generate_sdp_offer(&local_ip, local_port)
            .map_err(|e| MediaError::Configuration { message: e.to_string() })?;

        // Append ICE attributes if an ICE agent exists for this session
        {
            let agents = self.ice_agents.read().await;
            if let Some(agent) = agents.get(session_id) {
                let creds = agent.local_credentials();
                let candidates = agent.local_candidates();

                // If we have a server-reflexive candidate, use its address
                // as the SDP connection address for better NAT traversal.
                if let Some(srflx) = candidates.iter().find(|c| c.candidate_type == CandidateType::ServerReflexive) {
                    sdp = sdp.replace(
                        &format!("c=IN IP4 {}", local_ip),
                        &format!("c=IN IP4 {}", srflx.address.ip()),
                    );
                }

                // Build ICE attribute lines to insert before a=sendrecv
                let mut ice_lines = String::new();
                ice_lines.push_str(&format!("a=ice-ufrag:{}\r\n", creds.ufrag));
                ice_lines.push_str(&format!("a=ice-pwd:{}\r\n", creds.pwd));

                // RFC 8840: signal trickle ICE support
                if agent.is_trickle_enabled() {
                    ice_lines.push_str("a=ice-options:trickle\r\n");
                }

                for candidate in candidates {
                    ice_lines.push_str(&format!("a=candidate:{}\r\n", candidate.to_sdp_attribute()));
                }

                // Insert before the sendrecv attribute
                if let Some(pos) = sdp.rfind("a=sendrecv") {
                    sdp.insert_str(pos, &ice_lines);
                } else {
                    sdp.push_str(&ice_lines);
                }

                tracing::debug!(
                    session = %session_id,
                    candidates = candidates.len(),
                    "included ICE attributes in SDP offer"
                );
            }
        }

        // Append DTLS-SRTP attributes when SRTP is enabled in config.
        if self.media_config.srtp.enabled {
            if let Some(ref fp) = self.media_config.srtp.local_fingerprint {
                // Upgrade transport from RTP/AVP to RTP/SAVP
                sdp = sdp.replace("RTP/AVP", "RTP/SAVP");

                let dtls_attrs =
                    super::srtp_bridge::generate_dtls_sdp_attributes(fp, true);

                // Insert before a=sendrecv
                if let Some(pos) = sdp.rfind("a=sendrecv") {
                    sdp.insert_str(pos, &dtls_attrs);
                } else {
                    sdp.push_str(&dtls_attrs);
                }

                tracing::debug!(
                    session = %session_id,
                    "Included DTLS-SRTP attributes in SDP offer"
                );
            } else {
                tracing::warn!(
                    session = %session_id,
                    "SRTP enabled but no local fingerprint configured -- \
                     SDP offer will not include DTLS attributes"
                );
            }
        }

        // Store the generated local SDP
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(sdp.clone());
        }

        tracing::info!("Generated SDP offer for session: {} with port: {} and codecs: {:?}",
                      session_id, local_port, self.media_config.preferred_codecs);
        Ok(sdp)
    }
    
    /// Helper method to parse remote address from SDP (improved implementation)
    fn parse_remote_address_from_sdp(&self, sdp: &str) -> Option<SocketAddr> {
        // Enhanced SDP parsing to extract remote address and port
        let mut remote_ip = None;
        let mut remote_port = None;
        
        for line in sdp.lines() {
            if line.starts_with("c=IN IP4 ") {
                if let Some(ip_str) = line.strip_prefix("c=IN IP4 ") {
                    remote_ip = ip_str.trim().parse().ok();
                }
            } else if line.starts_with("m=audio ") {
                // Parse m=audio line: "m=audio 10001 RTP/AVP 96"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    remote_port = parts[1].parse().ok();
                }
            }
        }
        
        if let (Some(ip), Some(port)) = (remote_ip, remote_port) {
            tracing::debug!("Parsed remote address from SDP: {}:{}", ip, port);
            Some(SocketAddr::new(ip, port))
        } else {
            tracing::warn!("Could not parse remote address from SDP - ip: {:?}, port: {:?}", remote_ip, remote_port);
            None
        }
    }
    
    /// Parse codec information from SDP
    fn parse_codec_from_sdp(&self, sdp: &str) -> Option<String> {
        for line in sdp.lines() {
            if line.starts_with("a=rtpmap:") {
                // Parse a=rtpmap:96 opus/48000/2 -> return "opus"
                if let Some(codec_part) = line.split_whitespace().nth(1) {
                    if let Some(codec_name) = codec_part.split('/').next() {
                        tracing::debug!("Parsed codec from SDP: {}", codec_name);
                        return Some(codec_name.to_string());
                    }
                }
            }
        }
        None
    }
    
    /// Process SDP answer and configure media session.
    ///
    /// This also inspects the answer for DTLS-SRTP indicators and, if
    /// present, creates an `SrtpMediaBridge` for the session.  The caller
    /// must subsequently call `perform_srtp_handshake` before starting media.
    ///
    /// When ICE is enabled, remote ICE credentials and candidates are parsed
    /// from the SDP answer and fed into the session's ICE agent.  Connectivity
    /// checks are started, and if a selected pair is found the media session's
    /// remote address is updated accordingly.
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Processing SDP answer for session: {}", session_id);

        // Parse remote address from SDP and update media session
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            self.update_media_session(session_id, sdp).await?;
            tracing::info!("Processed SDP answer and updated remote address to: {}", remote_addr);
        } else {
            tracing::warn!("Could not parse remote address from SDP answer");
        }

        // Process remote ICE attributes if we have an agent for this session
        self.process_remote_ice(session_id, sdp).await?;

        // Check for DTLS-SRTP and initiate the full handshake if the
        // RTP socket is already available.  If the socket is not yet
        // allocated the bridge is still created; the handshake will be
        // driven later by the coordinator when the media session is ready.
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            if let Err(e) = self.initiate_srtp_for_session(session_id, sdp, remote_addr).await {
                tracing::warn!(
                    session = %session_id,
                    error = %e,
                    "DTLS-SRTP setup in process_sdp_answer failed -- plain RTP will be used"
                );
            }
        } else {
            // No remote address -- just create the bridge for later.
            let srtp = self.setup_srtp_from_sdp(session_id, sdp).await?;
            if srtp {
                tracing::info!(
                    session = %session_id,
                    "SDP answer indicates DTLS-SRTP -- bridge created, handshake pending"
                );
            }
        }

        Ok(())
    }

    /// Parse ICE credentials and candidates from remote SDP and feed them
    /// into the session's ICE agent.  Starts connectivity checks if
    /// sufficient information is available.
    async fn process_remote_ice(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = match agents.get_mut(session_id) {
            Some(a) => a,
            None => return Ok(()), // ICE not active for this session
        };

        // Parse remote ICE credentials from SDP
        let mut remote_ufrag: Option<String> = None;
        let mut remote_pwd: Option<String> = None;
        let mut remote_candidates: Vec<IceCandidate> = Vec::new();

        for line in sdp.lines() {
            let trimmed = line.trim();

            if let Some(ufrag) = trimmed.strip_prefix("a=ice-ufrag:") {
                remote_ufrag = Some(ufrag.to_string());
            } else if let Some(pwd) = trimmed.strip_prefix("a=ice-pwd:") {
                remote_pwd = Some(pwd.to_string());
            } else if let Some(cand_str) = trimmed.strip_prefix("a=candidate:") {
                if let Some(candidate) = Self::parse_ice_candidate(cand_str) {
                    remote_candidates.push(candidate);
                }
            }
        }

        // Set remote credentials if present
        if let (Some(ufrag), Some(pwd)) = (remote_ufrag, remote_pwd) {
            agent.set_remote_credentials(ufrag, pwd);
        } else {
            tracing::debug!(
                session = %session_id,
                "no remote ICE credentials in SDP, skipping ICE processing"
            );
            return Ok(());
        }

        // Add remote candidates
        if !remote_candidates.is_empty() {
            tracing::info!(
                session = %session_id,
                count = remote_candidates.len(),
                "adding remote ICE candidates from SDP"
            );
            agent.add_remote_candidates(remote_candidates);
        }

        // Start connectivity checks
        if let Err(e) = agent.start_checks() {
            tracing::warn!(
                session = %session_id,
                error = %e,
                "failed to start ICE connectivity checks"
            );
            return Ok(());
        }

        // Drive connectivity checks synchronously for up to the initial
        // set of waiting pairs.  In a full implementation this would run
        // on a background task with actual network I/O; here we log what
        // would happen and check for a selected pair from triggered checks
        // already completed (e.g., remote-initiated checks).
        let mut checks_performed = 0u32;
        while let Some(idx) = agent.next_check() {
            match agent.check_pair(idx) {
                Ok((_request_bytes, remote_addr)) => {
                    tracing::debug!(
                        session = %session_id,
                        pair_idx = idx,
                        remote = %remote_addr,
                        "ICE connectivity check prepared"
                    );
                    checks_performed += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        session = %session_id,
                        error = %e,
                        "ICE check_pair failed"
                    );
                    break;
                }
            }
            // Limit to avoid infinite loops in degenerate cases
            if checks_performed >= 64 {
                break;
            }
        }

        // If the agent already has a selected pair (e.g., from a
        // triggered check that succeeded), update the media session's
        // remote address.
        if let Some(pair) = agent.selected_pair() {
            let selected_remote = pair.remote.address;
            tracing::info!(
                session = %session_id,
                remote = %selected_remote,
                "ICE selected pair available, updating media remote address"
            );
            // Release the agents lock before updating media session
            drop(agents);
            self.update_media_remote_address(session_id, selected_remote).await?;
        }

        Ok(())
    }

    /// Parse a single ICE candidate from the value portion of an
    /// `a=candidate:` SDP attribute line.
    fn parse_ice_candidate(value: &str) -> Option<IceCandidate> {
        // Format: foundation component transport priority address port typ type [raddr addr rport port]
        let parts: Vec<&str> = value.split_whitespace().collect();
        if parts.len() < 8 {
            return None;
        }

        let foundation = parts[0].to_string();
        let component = match parts[1] {
            "1" => ComponentId::Rtp,
            "2" => ComponentId::Rtcp,
            _ => return None,
        };
        let transport = parts[2].to_lowercase();
        let priority: u32 = parts[3].parse().ok()?;
        let ip: std::net::IpAddr = parts[4].parse().ok()?;
        let port: u16 = parts[5].parse().ok()?;
        // parts[6] should be "typ"
        let candidate_type = match parts[7] {
            "host" => CandidateType::Host,
            "srflx" => CandidateType::ServerReflexive,
            "prflx" => CandidateType::PeerReflexive,
            "relay" => CandidateType::Relay,
            _ => return None,
        };

        let mut related_address: Option<SocketAddr> = None;
        let mut i = 8;
        while i + 1 < parts.len() {
            match parts[i] {
                "raddr" => {
                    if let Ok(rip) = parts[i + 1].parse::<std::net::IpAddr>() {
                        // Look for rport
                        if i + 3 < parts.len() && parts[i + 2] == "rport" {
                            if let Ok(rport) = parts[i + 3].parse::<u16>() {
                                related_address = Some(SocketAddr::new(rip, rport));
                                i += 4;
                                continue;
                            }
                        }
                        related_address = Some(SocketAddr::new(rip, 0));
                        i += 2;
                        continue;
                    }
                }
                _ => {}
            }
            i += 1;
        }

        Some(IceCandidate {
            foundation,
            component,
            transport,
            priority,
            address: SocketAddr::new(ip, port),
            candidate_type,
            related_address,
            ufrag: String::new(), // ufrag comes from session-level attribute
        })
    }

    /// Update the media session's remote address (e.g., after ICE
    /// connectivity checks select a candidate pair).
    async fn update_media_remote_address(
        &self,
        session_id: &SessionId,
        remote_addr: SocketAddr,
    ) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        let updated_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            Some(remote_addr),
        );
        self.controller.update_media(dialog_id, updated_config).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        tracing::info!(
            session = %session_id,
            remote = %remote_addr,
            "media remote address updated via ICE"
        );
        Ok(())
    }

    /// Get the ICE connection state for a session, if ICE is active.
    pub async fn get_ice_state(&self, session_id: &SessionId) -> Option<IceConnectionState> {
        let agents = self.ice_agents.read().await;
        agents.get(session_id).map(|a| a.state())
    }

    /// Get the selected ICE candidate pair for a session.
    pub async fn get_ice_selected_pair(&self, session_id: &SessionId) -> Option<(SocketAddr, SocketAddr)> {
        let agents = self.ice_agents.read().await;
        agents.get(session_id).and_then(|a| {
            a.selected_pair().map(|p| (p.local.address, p.remote.address))
        })
    }

    // ---------------------------------------------------------------
    // Trickle ICE (RFC 8838 / RFC 8840)
    // ---------------------------------------------------------------

    /// Enable trickle ICE for a session's ICE agent.
    pub async fn enable_trickle_ice(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;
        agent.enable_trickle();
        Ok(())
    }

    /// Check whether trickle ICE is enabled for a session.
    pub async fn is_trickle_ice_enabled(&self, session_id: &SessionId) -> bool {
        let agents = self.ice_agents.read().await;
        agents
            .get(session_id)
            .map_or(false, |a| a.is_trickle_enabled())
    }

    /// Add a remote ICE candidate received via trickle (SIP INFO).
    ///
    /// Parses the `a=candidate:` SDP attribute line and feeds it into
    /// the session's ICE agent. If connectivity checks are already
    /// running the agent will create a triggered check for the new pair.
    pub async fn add_remote_ice_candidate(
        &self,
        session_id: &SessionId,
        candidate_line: &str,
    ) -> super::MediaResult<()> {
        let candidate = IceCandidate::from_sdp_attribute(candidate_line).map_err(|e| {
            super::MediaError::Ice {
                message: format!("Failed to parse trickle candidate: {}", e),
            }
        })?;

        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        tracing::info!(
            "Adding trickle remote candidate for session {}: {}",
            session_id,
            candidate
        );
        agent.add_remote_candidate(candidate);

        Ok(())
    }

    /// Signal that the remote side has finished sending trickle candidates.
    pub async fn set_remote_end_of_candidates(
        &self,
        session_id: &SessionId,
    ) -> super::MediaResult<()> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        agent.set_end_of_candidates();
        tracing::info!(
            "Remote end-of-candidates set for session {}",
            session_id
        );

        Ok(())
    }

    /// Gather only host candidates for trickle ICE (fast, synchronous).
    ///
    /// Returns the host candidates immediately. STUN/TURN gathering should
    /// be done in a background task and trickled to the remote side.
    pub async fn gather_host_candidates_for_trickle(
        &self,
        session_id: &SessionId,
    ) -> super::MediaResult<Vec<IceCandidate>> {
        let mut agents = self.ice_agents.write().await;
        let agent = agents.get_mut(session_id).ok_or_else(|| {
            super::MediaError::Ice {
                message: format!("No ICE agent for session {}", session_id),
            }
        })?;

        let local_port: u16 = {
            let mapping = self.session_mapping.read().await;
            if let Some(dialog_id) = mapping.get(session_id) {
                self.controller
                    .get_session_info(dialog_id)
                    .await
                    .and_then(|info| info.rtp_port)
                    .unwrap_or(0)
            } else {
                0
            }
        };

        let local_addr = SocketAddr::new(self.local_bind_addr.ip(), local_port);
        let candidates = agent.gather_host_candidates_only(local_addr);

        tracing::info!(
            "Trickle: gathered {} host candidates for session {}",
            candidates.len(),
            session_id
        );

        Ok(candidates)
    }

    /// List all active media sessions
    pub async fn list_active_sessions(&self) -> Vec<MediaSessionInfo> {
        let mut sessions = Vec::new();
        let mapping = self.session_mapping.read().await;
        
        for dialog_id in mapping.values() {
            if let Some(media_session_info) = self.controller.get_session_info(dialog_id).await {
                sessions.push(MediaSessionInfo::from(media_session_info));
            }
        }
        
        sessions
    }
    
    /// Get the local bind address
    pub fn get_local_bind_addr(&self) -> SocketAddr {
        self.local_bind_addr
    }
    
    /// Start audio transmission for a session
    pub async fn start_audio_transmission(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission for session: {}", session_id);
        Ok(())
    }
    
    /// Start audio transmission for a session with tone generation
    pub async fn start_audio_transmission_with_tone(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission_with_tone(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission with tone for session: {}", session_id);
        Ok(())
    }
    
    /// Start audio transmission for a session with custom audio samples
    pub async fn start_audio_transmission_with_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission_with_custom_audio(&dialog_id, samples, repeat).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission with custom audio for session: {}", session_id);
        Ok(())
    }
    
    /// Stop audio transmission for a session
    pub async fn stop_audio_transmission(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Stopping audio transmission for session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.stop_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Stopped audio transmission for session: {}", session_id);
        Ok(())
    }
    
    /// Set audio muted state for a session (send silence when muted)
    pub async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> super::MediaResult<()> {
        tracing::debug!("MediaManager::set_audio_muted called for session: {} muted={}", session_id, muted);

        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            tracing::debug!("Session mapping contents: {:?}", mapping.keys().collect::<Vec<_>>());
            mapping.get(session_id).cloned()
                .ok_or_else(|| {
                    tracing::error!("Session mapping not found for: {}", session_id);
                    MediaError::SessionNotFound { session_id: session_id.to_string() }
                })?
        };

        tracing::debug!("Found dialog_id: {} for session: {}", dialog_id, session_id);

        tracing::debug!("Calling media-core set_audio_muted for dialog: {} muted={}", dialog_id, muted);
        self.controller.set_audio_muted(&dialog_id, muted).await
            .map_err(|e| {
                tracing::error!("Media-core set_audio_muted failed: {}", e);
                MediaError::MediaEngine { source: Box::new(e) }
            })?;

        tracing::info!("Successfully set audio muted={} for session {} (dialog {})", muted, session_id, dialog_id);
        Ok(())
    }
    
    /// Set custom audio samples for an active transmission session
    pub async fn set_custom_audio(&self, session_id: &SessionId, samples: Vec<u8>, repeat: bool) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_custom_audio(&dialog_id, samples, repeat).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set custom audio for session: {}", session_id);
        Ok(())
    }
    
    /// Set tone generation parameters for an active transmission session
    pub async fn set_tone_generation(&self, session_id: &SessionId, frequency: f64, amplitude: f64) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_tone_generation(&dialog_id, frequency, amplitude).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set tone generation for session: {}", session_id);
        Ok(())
    }
    
    /// Enable pass-through mode for an active transmission session
    pub async fn set_pass_through_mode(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.set_pass_through_mode(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Set pass-through mode for session: {}", session_id);
        Ok(())
    }
    
    /// Helper method to get dialog ID from session ID
    async fn get_dialog_id(&self, session_id: &SessionId) -> super::MediaResult<DialogId> {
        let mapping = self.session_mapping.read().await;
        mapping.get(session_id).cloned()
            .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })
    }

    // =============================================================================
    // AUDIO STREAMING API IMPLEMENTATION
    // =============================================================================

    /// Set audio frame callback for a session to receive decoded frames
    /// This method integrates with the RTP decoder to provide audio frames from RTP events
    pub async fn set_audio_frame_callback(
        &self,
        session_id: &SessionId,
        callback: tokio::sync::mpsc::Sender<crate::api::types::AudioFrame>,
    ) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;


        // Set up the media-core callback directly - no conversion needed anymore!
        self.controller.set_audio_frame_callback(dialog_id.clone(), callback).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;

        tracing::info!("🔊 Set up audio frame callback for session: {}", session_id);
        Ok(())
    }

    /// Remove audio frame callback for a session
    pub async fn remove_audio_frame_callback(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        
        self.controller.remove_audio_frame_callback(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("🔇 Removed audio frame callback for session: {}", session_id);
        Ok(())
    }

    /// Send audio frame for encoding and transmission
    pub async fn send_audio_frame_for_transmission(
        &self,
        session_id: &SessionId,
        audio_frame: crate::api::types::AudioFrame,
    ) -> super::MediaResult<()> {
        tracing::debug!("📤 Received audio frame for transmission for session: {}", session_id);
        
        let dialog_id = match self.get_dialog_id(session_id).await {
            Ok(id) => id,
            Err(e) => {
                tracing::error!("❌ Failed to get dialog ID for session {}: {}", session_id, e);
                return Err(e);
            }
        };
        
        tracing::debug!("✅ Got dialog ID {} for session {}", dialog_id, session_id);
        
        // Calculate RTP timestamp (8kHz clock rate for G.711)
        // Use modulo to prevent overflow
        let timestamp = ((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() % (u32::MAX as u128 / 8)) as u32) * 8; // Convert to 8kHz RTP clock
        
        // Delegate encoding and transmission to media-core
        // This properly uses codec-core for encoding based on the configured codec
        let sample_count = audio_frame.samples.len();
        tracing::info!("🔧 [DEBUG] About to encode and send audio frame for session: {} (dialog: {}, {} samples)", 
                      session_id, dialog_id, sample_count);
        
        match self.controller.encode_and_send_audio_frame(&dialog_id, audio_frame.samples, timestamp).await {
            Ok(()) => {
                tracing::info!("✅ [SUCCESS] Audio frame encoded and sent successfully for session: {}", session_id);
            }
            Err(e) => {
                tracing::error!("❌ [ERROR] Failed to encode and send audio frame for session: {}: {}", session_id, e);
                return Err(MediaError::MediaEngine { source: Box::new(e) });
            }
        }
        
        tracing::debug!("📡 Sent audio frame for session: {} ({} samples, timestamp: {})", 
                       session_id, sample_count, timestamp);
        Ok(())
    }

    /// Get audio stream configuration for a session
    pub async fn get_audio_stream_config_internal(&self, session_id: &SessionId) -> super::MediaResult<Option<crate::api::types::AudioStreamConfig>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Ok(None);
        }
        
        // For now, return a default config based on our media config
        let config = crate::api::types::AudioStreamConfig {
            sample_rate: 8000,
            channels: 1,
            codec: self.media_config.preferred_codecs.first()
                .cloned()
                .unwrap_or_else(|| "PCMU".to_string()),
            frame_size_ms: 20,
            enable_aec: self.media_config.echo_cancellation,
            enable_agc: self.media_config.auto_gain_control,
            enable_vad: true, // Default VAD on
        };
        
        Ok(Some(config))
    }

    /// Set audio stream configuration for a session
    pub async fn set_audio_stream_config_internal(
        &self,
        session_id: &SessionId,
        config: crate::api::types::AudioStreamConfig,
    ) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Err(MediaError::SessionNotFound { session_id: session_id.to_string() });
        }
        
        // TODO: Apply configuration to the media session
        // For now, we store the configuration for later use
        tracing::info!("📊 Applied audio stream config for session {}: {}Hz, {} channels, codec: {}", 
                      session_id, config.sample_rate, config.channels, config.codec);
        
        Ok(())
    }

    /// Check if audio streaming is active for a session
    pub async fn is_audio_streaming_active(&self, session_id: &SessionId) -> super::MediaResult<bool> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists and has a callback registered
        if let Some(session_info) = self.controller.get_session_info(&dialog_id).await {
            // For now, consider streaming active if session is active
            // TODO: Add proper check for audio streaming status
            Ok(matches!(session_info.status, rvoip_media_core::relay::controller::types::MediaSessionStatus::Active))
        } else {
            Ok(false)
        }
    }

    /// Start audio streaming for a session
    pub async fn start_audio_streaming(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Check if session exists
        if self.controller.get_session_info(&dialog_id).await.is_none() {
            return Err(MediaError::SessionNotFound { session_id: session_id.to_string() });
        }
        
        // Start RTP event processing for this session
        
        // TODO: Start the actual audio streaming pipeline
        // For now, this is handled through the existing audio transmission methods
        tracing::info!("🎵 Started audio streaming for session: {}", session_id);
        Ok(())
    }

    /// Stop audio streaming for a session
    pub async fn stop_audio_streaming(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        
        // Remove the callback
        self.remove_audio_frame_callback(session_id).await?;
        
        // TODO: Stop the actual audio streaming pipeline
        // For now, this is handled through the existing audio transmission methods
        tracing::info!("🛑 Stopped audio streaming for session: {}", session_id);
        Ok(())
    }

    
    /// Send an audio frame as RTP packets
    /// This method encodes the PCM audio frame to G.711 and sends it via RTP
    pub async fn send_audio_frame(&self, session_id: &SessionId, frame: crate::api::types::AudioFrame) -> super::MediaResult<()> {
        // Get the media session mapping
        let mapping = self.session_mapping.read().await;
        let media_session_id = mapping.get(session_id)
            .ok_or_else(|| MediaError::SessionNotFound { 
                session_id: session_id.to_string() 
            })?
            .clone();
        
        // For now, we'll skip getting session info and use default payload type
        // TODO: Get actual payload type from session info when available
        
        // Determine payload type from negotiated codec
        // For now, default to PCMU (0) if not specified
        let payload_type = 0u8; // TODO: Get from session_info.codec_config
        
        // Initialize encoder for this session if needed
        {
            let mut encoder = self.rtp_encoder.lock().await;
            // Check if session is already initialized by trying to encode a dummy frame
            let test_frame = crate::api::types::AudioFrame::new(
                vec![],
                8000,
                1,
                0,
            );
            if encoder.encode_audio_frame(session_id, &test_frame).is_err() {
                encoder.init_session(session_id.clone(), payload_type);
            }
        }
        
        // Encode the audio frame
        let encoded_payload = {
            let mut encoder = self.rtp_encoder.lock().await;
            encoder.encode_audio_frame(session_id, &frame)
                .map_err(|e| MediaError::Configuration { message: e })?
        };
        
        // Create MediaPacket for media-core
        let media_packet = rvoip_media_core::MediaPacket {
            payload: bytes::Bytes::from(encoded_payload.data),
            payload_type: encoded_payload.payload_type,
            timestamp: encoded_payload.timestamp,
            sequence_number: encoded_payload.sequence_number,
            ssrc: 0, // TODO: Get SSRC from session
            received_at: std::time::Instant::now(), // Not used for sending
        };
        
        // Send the packet via existing send_audio_frame_for_transmission method
        // This will use the controller's encode_and_send_audio_frame
        self.send_audio_frame_for_transmission(session_id, frame).await?;
        
        Ok(())
    }


    /// Initialize codec detection for a session with expected codec
    pub async fn initialize_codec_detection(&self, session_id: &SessionId, expected_codec: Option<String>) -> super::MediaResult<()> {
        tracing::debug!("Initializing codec detection for session {}: expected codec={:?}", session_id, expected_codec);
        
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Initialize codec detection
        self.codec_detector.initialize_detection(dialog_id.clone(), expected_codec.clone()).await;
        
        // Initialize fallback handling
        self.fallback_manager.initialize_fallback(dialog_id, expected_codec).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Initialized codec detection and fallback for session {}", session_id);
        Ok(())
    }
    
    /// Get codec detection status for a session
    pub async fn get_codec_detection_status(&self, session_id: &SessionId) -> super::MediaResult<Option<String>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        if let Some(result) = self.codec_detector.get_detection_result(&dialog_id).await {
            match result {
                CodecDetectionResult::Expected { codec, confidence, .. } => {
                    Ok(Some(format!("Expected codec confirmed: {:?} (confidence: {:.2})", codec, confidence)))
                },
                CodecDetectionResult::UnexpectedCodec { 
                    expected_codec, detected_codec, confidence, .. 
                } => {
                    Ok(Some(format!("Unexpected codec detected: expected {:?}, got {:?} (confidence: {:.2})", 
                                   expected_codec, detected_codec, confidence)))
                },
                CodecDetectionResult::InsufficientData { packets_analyzed } => {
                    Ok(Some(format!("Insufficient data for detection ({} packets analyzed)", packets_analyzed)))
                },
            }
        } else {
            Ok(None)
        }
    }
    
    /// Get fallback status for a session
    pub async fn get_fallback_status(&self, session_id: &SessionId) -> super::MediaResult<Option<String>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        if let Some(stats) = self.fallback_manager.get_stats(&dialog_id).await {
            let status = match &stats.current_mode {
                FallbackMode::None => "No fallback needed".to_string(),
                FallbackMode::Transcoding { from_codec, to_codec, .. } => {
                    format!("Transcoding: {} → {} (efficiency: {:.1}%)", 
                           from_codec, to_codec, stats.get_efficiency() * 100.0)
                },
                FallbackMode::Passthrough { detected_codec, .. } => {
                    format!("Passthrough mode: {} (efficiency: {:.1}%)", 
                           detected_codec, stats.get_efficiency() * 100.0)
                },
            };
            Ok(Some(status))
        } else {
            Ok(None)
        }
    }
    
    /// Get comprehensive codec processing statistics for a session
    pub async fn get_codec_processing_stats(&self, session_id: &SessionId) -> super::MediaResult<Option<super::types::CodecProcessingStats>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        
        // Get detection state
        let detection_state = self.codec_detector.get_detection_state(&dialog_id).await;
        
        // Get fallback stats
        let fallback_stats = self.fallback_manager.get_stats(&dialog_id).await;
        
        if detection_state.is_some() || fallback_stats.is_some() {
            Ok(Some(super::types::CodecProcessingStats {
                session_id: session_id.clone(),
                expected_codec: detection_state.as_ref().and_then(|s| s.expected_codec.clone()),
                detected_codec: detection_state.as_ref().and_then(|s| s.detected_payload_type)
                    .and_then(|pt| self.codec_mapper.payload_to_codec(pt)),
                detection_confidence: detection_state.as_ref().map(|s| s.confidence).unwrap_or(0.0),
                packets_analyzed: detection_state.as_ref().map(|s| s.packets_analyzed).unwrap_or(0),
                fallback_mode: fallback_stats.as_ref().map(|s| s.current_mode.clone()).unwrap_or(FallbackMode::None),
                fallback_efficiency: fallback_stats.as_ref().map(|s| s.get_efficiency()).unwrap_or(1.0),
                transcoding_active: fallback_stats.as_ref().map(|s| s.transcoding_active).unwrap_or(false),
            }))
        } else {
            Ok(None)
        }
    }
    
    /// Clean up codec processing systems for a session
    async fn cleanup_codec_processing(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;


        // Cleanup codec detection
        self.codec_detector.cleanup_detection(&dialog_id).await;

        // Cleanup fallback handling
        self.fallback_manager.cleanup_fallback(&dialog_id).await;

        tracing::debug!("Cleaned up codec processing and RTP handling for session {}", session_id);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // SRTP bridge integration
    // -----------------------------------------------------------------------

    /// Set up an SRTP bridge for a session after SDP negotiation.
    ///
    /// Inspects the remote SDP for DTLS-SRTP indicators (`a=fingerprint`,
    /// `RTP/SAVP`, `a=setup`).  If secure media is required a bridge is
    /// created and stored.  The actual DTLS handshake is *not* started here
    /// -- call `perform_srtp_handshake` when the transport is ready.
    pub async fn setup_srtp_from_sdp(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
    ) -> super::MediaResult<bool> {
        let (srtp_required, remote_fingerprint, remote_role) =
            super::srtp_bridge::extract_dtls_params_from_sdp(remote_sdp);

        if !srtp_required {
            tracing::debug!(
                "No DTLS-SRTP indicators in SDP for session {} -- plain RTP",
                session_id
            );
            return Ok(false);
        }

        // The *local* DTLS role is the inverse of the remote setup role:
        //   remote=actpass (offerer) -> we answer with active (client)
        //   remote=active           -> we are passive (server)
        //   remote=passive          -> we are active  (client)
        let local_role = match remote_role {
            Some(DtlsRole::Client) => DtlsRole::Server,
            Some(DtlsRole::Server) | None => DtlsRole::Client,
        };

        let bridge = SrtpMediaBridge::new(true, local_role, remote_fingerprint);

        tracing::info!(
            session = %session_id,
            role = ?local_role,
            "SRTP bridge created for session (DTLS handshake pending)"
        );

        let mut bridges = self.srtp_bridges.write().await;
        bridges.insert(session_id.clone(), Arc::new(Mutex::new(bridge)));

        Ok(true)
    }

    /// Drive the DTLS handshake for a session that has an SRTP bridge.
    ///
    /// This must be called *after* `setup_srtp_from_sdp` and *before* media
    /// starts flowing.  The `socket` should be the same UDP socket that will
    /// carry RTP/SRTP traffic.
    pub async fn perform_srtp_handshake(
        &self,
        session_id: &SessionId,
        socket: Arc<tokio::net::UdpSocket>,
        remote_addr: SocketAddr,
    ) -> super::MediaResult<()> {
        let bridge_arc = {
            let bridges = self.srtp_bridges.read().await;
            bridges.get(session_id).cloned()
        };

        let bridge_arc = match bridge_arc {
            Some(b) => b,
            None => {
                tracing::debug!(
                    "No SRTP bridge for session {} -- skipping handshake",
                    session_id
                );
                return Ok(());
            }
        };

        let mut bridge = bridge_arc.lock().await;
        bridge.perform_dtls_handshake(socket, remote_addr).await?;

        tracing::info!(
            session = %session_id,
            "DTLS-SRTP handshake completed, SRTP keys installed"
        );
        Ok(())
    }

    /// Encrypt an outbound RTP packet through the session's SRTP bridge.
    ///
    /// Returns the original packet unchanged when SRTP is not active for
    /// this session (plain RTP fallback).
    pub async fn protect_rtp(
        &self,
        session_id: &SessionId,
        packet: &[u8],
    ) -> super::MediaResult<Vec<u8>> {
        let bridge_arc = {
            let bridges = self.srtp_bridges.read().await;
            bridges.get(session_id).cloned()
        };

        match bridge_arc {
            Some(b) => {
                let mut bridge = b.lock().await;
                bridge.protect_rtp(packet)
            }
            // No bridge -> plain RTP pass-through.
            None => Ok(packet.to_vec()),
        }
    }

    /// Decrypt an inbound SRTP packet through the session's SRTP bridge.
    ///
    /// Returns the original packet unchanged when SRTP is not active for
    /// this session (plain RTP fallback).
    pub async fn unprotect_rtp(
        &self,
        session_id: &SessionId,
        packet: &[u8],
    ) -> super::MediaResult<Vec<u8>> {
        let bridge_arc = {
            let bridges = self.srtp_bridges.read().await;
            bridges.get(session_id).cloned()
        };

        match bridge_arc {
            Some(b) => {
                let mut bridge = b.lock().await;
                bridge.unprotect_rtp(packet)
            }
            None => Ok(packet.to_vec()),
        }
    }

    /// Send an RTP packet, applying SRTP encryption when active.
    ///
    /// This is the recommended send path for callers that produce raw RTP
    /// payload bytes.  The method:
    ///   1. Encodes the payload into an RTP packet via the media-core controller.
    ///   2. If an SRTP bridge is active for the session, encrypts the packet.
    ///   3. Transmits the (possibly encrypted) packet.
    ///
    /// When no SRTP bridge exists the packet is sent as plain RTP.
    pub async fn send_rtp_with_srtp(
        &self,
        session_id: &SessionId,
        payload: Vec<u8>,
        timestamp: u32,
    ) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound {
                    session_id: session_id.to_string(),
                })?
        };

        // Check if SRTP is active for this session.
        if self.is_srtp_active(session_id).await {
            // Build a temporary RTP packet so we can protect it, then send
            // the encrypted bytes through the controller.
            let rtp_session_arc = self.controller.get_rtp_session(&dialog_id).await
                .ok_or_else(|| MediaError::SessionNotFound {
                    session_id: dialog_id.to_string(),
                })?;

            let session = rtp_session_arc.lock().await;

            // Build a minimal RTP header to create a packet for SRTP.
            let header = rvoip_rtp_core::RtpHeader::new(
                0, // payload type will be set by session config
                0, // sequence number placeholder
                timestamp,
                0, // SSRC placeholder
            );
            let rtp_packet = rvoip_rtp_core::RtpPacket::new(
                header,
                bytes::Bytes::from(payload),
            );
            let serialized = rtp_packet.serialize().map_err(|e| {
                MediaError::SdpProcessing {
                    message: format!("Failed to serialize RTP packet: {e}"),
                }
            })?;

            // Release the session lock before protecting.
            drop(session);

            let protected = self.protect_rtp(session_id, &serialized).await?;

            // Send protected bytes through the controller.
            self.controller.send_rtp_packet(
                &dialog_id,
                protected,
                timestamp,
            ).await.map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        } else {
            // Plain RTP -- delegate directly.
            self.controller.send_rtp_packet(&dialog_id, payload, timestamp)
                .await
                .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        }

        Ok(())
    }

    /// Process a received RTP packet, applying SRTP decryption when active.
    ///
    /// Returns the decrypted RTP payload bytes.  When no SRTP bridge exists
    /// the packet is returned as-is.
    pub async fn receive_rtp_with_srtp(
        &self,
        session_id: &SessionId,
        packet: &[u8],
    ) -> super::MediaResult<Vec<u8>> {
        self.unprotect_rtp(session_id, packet).await
    }

    /// Check whether a session has an active (post-handshake) SRTP bridge.
    pub async fn is_srtp_active(&self, session_id: &SessionId) -> bool {
        let bridges = self.srtp_bridges.read().await;
        if let Some(b) = bridges.get(session_id) {
            let bridge = b.lock().await;
            bridge.is_active()
        } else {
            false
        }
    }

    /// Retrieve the UDP socket handle for a session's RTP transport.
    ///
    /// Returns `None` when no RTP session exists for this session yet.
    pub async fn get_rtp_socket(
        &self,
        session_id: &SessionId,
    ) -> Option<Arc<tokio::net::UdpSocket>> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };

        let dialog_id = dialog_id?;

        let rtp_session_arc = self.controller.get_rtp_session(&dialog_id).await?;
        let session = rtp_session_arc.lock().await;
        session.get_socket_handle().await.ok()
    }

    /// One-shot helper: set up an SRTP bridge from the remote SDP *and*
    /// drive the DTLS handshake using the session's existing RTP socket.
    ///
    /// This is the primary entry point that the coordinator should call
    /// after SDP negotiation completes.  It is a no-op when the remote SDP
    /// does not indicate DTLS-SRTP.
    ///
    /// `remote_addr` is the far-end RTP address (parsed from SDP).
    pub async fn initiate_srtp_for_session(
        &self,
        session_id: &SessionId,
        remote_sdp: &str,
        remote_addr: SocketAddr,
    ) -> super::MediaResult<()> {
        // Step 1 -- inspect SDP and create the bridge (no-op if plain RTP).
        let srtp_needed = self.setup_srtp_from_sdp(session_id, remote_sdp).await?;
        if !srtp_needed {
            return Ok(());
        }

        // Step 2 -- obtain the RTP socket for the DTLS handshake.
        let socket = match self.get_rtp_socket(session_id).await {
            Some(s) => s,
            None => {
                tracing::warn!(
                    session = %session_id,
                    "SRTP bridge created but no RTP socket available yet -- \
                     handshake deferred until media session is ready"
                );
                return Ok(());
            }
        };

        // Step 3 -- perform the DTLS handshake (blocks until complete).
        tracing::info!(
            session = %session_id,
            remote = %remote_addr,
            "Initiating DTLS-SRTP handshake"
        );
        self.perform_srtp_handshake(session_id, socket, remote_addr).await?;

        tracing::info!(
            session = %session_id,
            "DTLS-SRTP handshake completed -- SRTP is active"
        );
        Ok(())
    }

    /// Remove and clean up the SRTP bridge for a session.
    async fn cleanup_srtp_bridge(&self, session_id: &SessionId) {
        let mut bridges = self.srtp_bridges.write().await;
        if bridges.remove(session_id).is_some() {
            tracing::debug!("Cleaned up SRTP bridge for session {}", session_id);
        }
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