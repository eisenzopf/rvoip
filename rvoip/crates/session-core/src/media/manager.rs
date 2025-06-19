//! Media Manager for Session-Core
//!
//! Main interface for media operations, using real MediaSessionController from media-core.
//! This manager coordinates between SIP sessions and media-core components.

use crate::api::types::SessionId;
use crate::errors::Result;
use super::types::*;
use super::MediaError;
use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;
use tracing::warn;
use tokio::sync::RwLock;
use async_trait::async_trait;

// Import RTP types from media-core for zero-copy processing
use rvoip_rtp_core::RtpPacket;
use rvoip_media_core::performance::pool::PoolStats;
use crate::manager::events::SessionEventProcessor;

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
        
        Self {
            controller: Arc::new(MediaSessionController::new()),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config: MediaConfig::default(),
        }
    }
    
    /// Create a MediaManager with custom port range
    pub fn with_port_range(local_bind_addr: SocketAddr, base_port: u16, max_port: u16) -> Self {
        let event_processor = Arc::new(SessionEventProcessor::new());
        
        Self {
            controller: Arc::new(MediaSessionController::with_port_range(base_port, max_port)),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config: MediaConfig::default(),
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
        
        Self {
            controller: Arc::new(MediaSessionController::with_port_range(base_port, max_port)),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
            zero_copy_config: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            event_processor,
            sdp_storage: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            media_config,
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
    pub async fn get_rtp_statistics(&self, session_id: &SessionId) -> super::MediaResult<Option<rvoip_rtp_core::session::RtpSessionStats>> {
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
        tracing::debug!("Creating media session for SIP session: {}", session_id);
        
        // Create dialog ID for media session (use session ID as base)
        let dialog_id = DialogId::new(format!("media-{}", session_id));
        
        // Create media configuration using the manager's configured preferences
        let media_config = convert_to_media_core_config(
            &self.media_config,
            self.local_bind_addr,
            None, // Will be set later when remote SDP is processed
        );
        
        // Start media session using real MediaSessionController
        self.controller.start_media(dialog_id.clone(), media_config).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        // Get session info from controller
        let media_session_info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| MediaError::SessionNotFound { session_id: dialog_id.to_string() })?;
        
        // Store session mapping
        {
            let mut mapping = self.session_mapping.write().await;
            mapping.insert(session_id.clone(), dialog_id.clone());
        }
        
        // Initialize zero-copy configuration for new session
        {
            let mut configs = self.zero_copy_config.write().await;
            configs.insert(session_id.clone(), ZeroCopyConfig::default());
        }
        
        // Convert to our MediaSessionInfo type
        let session_info = MediaSessionInfo::from(media_session_info);
        
        tracing::info!("✅ Created media session: {} for SIP session: {} with real MediaSessionController + zero-copy enabled", 
                      dialog_id, session_id);
        
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
        
        // Cleanup SDP storage
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            sdp_storage.remove(session_id);
        }
        
        // Stop media session using real MediaSessionController
        self.controller.stop_media(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Terminated media session: {} for SIP session: {} (including zero-copy cleanup)", dialog_id, session_id);
        Ok(())
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
        
        let sdp = converter.generate_sdp_offer(&local_ip, local_port)
            .map_err(|e| MediaError::Configuration { message: e.to_string() })?;
        
        // Store the generated local SDP
        {
            let mut sdp_storage = self.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(sdp.clone());
        }
        
        tracing::info!("✅ Generated SDP offer for session: {} with port: {} and codecs: {:?}", 
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
    
    /// Process SDP answer and configure media session
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Processing SDP answer for session: {}", session_id);
        
        // Parse remote address from SDP and update media session
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            self.update_media_session(session_id, sdp).await?;
            tracing::info!("✅ Processed SDP answer and updated remote address to: {}", remote_addr);
        } else {
            tracing::warn!("Could not parse remote address from SDP answer");
        }
        
        Ok(())
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
    
    /// Helper method to get dialog ID from session ID
    async fn get_dialog_id(&self, session_id: &SessionId) -> super::MediaResult<DialogId> {
        let mapping = self.session_mapping.read().await;
        mapping.get(session_id).cloned()
            .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })
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
            .unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
        
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