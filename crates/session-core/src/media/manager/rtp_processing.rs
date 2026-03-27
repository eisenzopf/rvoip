//! RTP processing, zero-copy optimization, and statistics methods for MediaManager

use crate::api::types::SessionId;
use tracing::warn;
use super::super::types::*;
use super::super::MediaError;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::Mutex;
use super::MediaManager;
use super::super::MediaResult;
use rvoip_media_core::performance::pool::PoolStats;
use rvoip_media_core::prelude::RtpPacket;

impl MediaManager {
    /// Process RTP packet with zero-copy optimization
    pub async fn process_rtp_packet_zero_copy(&self, session_id: &SessionId, packet: &RtpPacket) -> super::super::MediaResult<RtpPacket> {
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
    pub async fn process_rtp_packet_traditional(&self, session_id: &SessionId, packet: &RtpPacket) -> super::super::MediaResult<RtpPacket> {
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
    pub async fn get_rtp_statistics(&self, session_id: &SessionId) -> super::super::MediaResult<Option<rvoip_media_core::RtpSessionStats>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        Ok(self.controller.get_rtp_statistics(&dialog_id).await)
    }
    
    /// Get comprehensive media statistics
    pub async fn get_media_statistics(&self, session_id: &SessionId) -> super::super::MediaResult<Option<rvoip_media_core::types::MediaStatistics>> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        Ok(self.controller.get_media_statistics(&dialog_id).await)
    }
    
    /// Start periodic statistics monitoring with the specified interval
    pub async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> super::super::MediaResult<()> {
        let dialog_id = self.get_dialog_id(session_id).await?;
        self.controller.start_statistics_monitoring(dialog_id, interval).await
            .map_err(|e| super::super::MediaError::MediaEngine {
                source: Box::new(e),
            })
    }
    
    /// Enable/disable zero-copy processing for a session
    pub async fn set_zero_copy_processing(&self, session_id: &SessionId, enabled: bool) -> super::super::MediaResult<()> {
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
    pub async fn configure_zero_copy_processing(&self, session_id: &SessionId, config: super::ZeroCopyConfig) -> super::super::MediaResult<()> {
        tracing::info!("Configuring zero-copy processing for session {}: enabled={}, fallback={}, monitoring={}", 
                      session_id, config.enabled, config.fallback_enabled, config.monitoring_enabled);
        
        let mut configs = self.zero_copy_config.write().await;
        configs.insert(session_id.clone(), config);
        
        tracing::debug!("✅ Zero-copy processing configuration applied for session {}", session_id);
        Ok(())
    }
    
    /// Get zero-copy configuration for a session
    pub async fn get_zero_copy_config(&self, session_id: &SessionId) -> super::ZeroCopyConfig {
        let configs = self.zero_copy_config.read().await;
        configs.get(session_id).cloned().unwrap_or_default()
    }
    
    /// Get RTP processing performance metrics for a session
    pub async fn get_rtp_processing_metrics(&self, session_id: &SessionId) -> super::super::MediaResult<RtpProcessingMetrics> {
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
    pub(crate) async fn cleanup_zero_copy_config(&self, session_id: &SessionId) {
        let mut configs = self.zero_copy_config.write().await;
        configs.remove(session_id);
        tracing::debug!("🧹 Cleaned up zero-copy config for session {}", session_id);
    }
}
