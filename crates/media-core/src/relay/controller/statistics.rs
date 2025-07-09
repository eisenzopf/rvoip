//! Statistics and monitoring functionality
//!
//! This module provides comprehensive statistics collection and monitoring
//! for RTP sessions, including quality metrics and MOS score calculation.

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::types::{DialogId, MediaSessionId, MediaStatistics, MediaProcessingStats, QualityMetrics};
use rvoip_rtp_core::session::{RtpSessionStats, RtpStreamStats};

use super::{MediaSessionController, MediaSessionEvent};

impl MediaSessionController {
    /// Get RTP stats for a dialog (basic string format)
    pub async fn get_rtp_stats(&self, dialog_id: &DialogId) -> Option<String> {
        let rtp_session = self.get_rtp_session(dialog_id).await?;
        let session = rtp_session.lock().await;
        
        // Get basic session info
        let local_addr = session.local_addr().ok()?;
        let ssrc = session.get_ssrc();
        
        Some(format!("RTP Session - Local: {}, SSRC: 0x{:08x}", local_addr, ssrc))
    }
    
    /// Get comprehensive RTP statistics for a dialog
    pub async fn get_rtp_statistics(&self, dialog_id: &DialogId) -> Option<RtpSessionStats> {
        let rtp_session = self.get_rtp_session(dialog_id).await?;
        let session = rtp_session.lock().await;
        Some(session.get_stats())
    }
    
    /// Get all stream statistics for a dialog
    pub async fn get_stream_statistics(&self, dialog_id: &DialogId) -> Vec<RtpStreamStats> {
        if let Some(rtp_session) = self.get_rtp_session(dialog_id).await {
            let session = rtp_session.lock().await;
            session.get_all_streams().await
        } else {
            Vec::new()
        }
    }
    
    /// Get comprehensive media statistics including RTP/RTCP data
    pub async fn get_media_statistics(&self, dialog_id: &DialogId) -> Option<MediaStatistics> {
        // Get session info
        let session_info = self.get_session_info(dialog_id).await?;
        
        // Get RTP statistics
        let rtp_stats = self.get_rtp_statistics(dialog_id).await;
        
        // Get stream statistics
        let stream_stats = self.get_stream_statistics(dialog_id).await;
        
        // Get actual codec from session configuration
        let current_codec = session_info.config.preferred_codec.clone()
            .or_else(|| Some("PCMU".to_string())); // Default to PCMU if none set

        
        // Calculate quality metrics from RTP stats
        let quality_metrics = rtp_stats.as_ref().map(|stats| {
            QualityMetrics {
                packet_loss_percent: if stats.packets_received > 0 {
                    (stats.packets_lost as f32 / (stats.packets_received + stats.packets_lost) as f32) * 100.0
                } else {
                    0.0
                },
                jitter_ms: stats.jitter_ms,
                rtt_ms: None, // TODO: Extract from RTCP SR/RR when available
                mos_score: Self::calculate_mos_from_stats(stats),
                network_quality: Self::calculate_network_quality(stats),
            }
        });
        
        // Build comprehensive statistics
        Some(MediaStatistics {
            session_id: MediaSessionId::new(&dialog_id.to_string()),
            dialog_id: dialog_id.clone(),
            rtp_stats: rtp_stats.clone(),
            stream_stats,
            media_stats: MediaProcessingStats {
                // These would come from actual media processing
                packets_processed: rtp_stats.as_ref().map(|s| s.packets_received).unwrap_or(0),
                frames_encoded: 0, // TODO: Track in media processing
                frames_decoded: 0, // TODO: Track in media processing
                processing_errors: 0,
                codec_changes: 0,
                current_codec,
            },
            quality_metrics,
            session_start: session_info.created_at,
            session_duration: session_info.created_at.elapsed(),
        })
    }
    
    /// Helper to estimate MOS score from RTP statistics
    pub(super) fn calculate_mos_from_stats(stats: &RtpSessionStats) -> Option<f32> {
        // Simple E-model approximation
        let packet_loss_percent = if stats.packets_received > 0 {
            (stats.packets_lost as f32 / (stats.packets_received + stats.packets_lost) as f32) * 100.0
        } else {
            0.0
        };
        
        // Basic MOS calculation (simplified)
        // Start with perfect score and deduct based on impairments
        let mut mos: f32 = 4.5;
        
        // Deduct for packet loss (up to 2.5 points)
        mos -= (packet_loss_percent * 0.25).min(2.5);
        
        // Deduct for jitter (up to 1.0 point)
        mos -= (stats.jitter_ms as f32 * 0.01).min(1.0);
        
        // Ensure MOS is within valid range
        Some(mos.max(1.0).min(5.0))
    }
    
    /// Helper to calculate network quality score
    pub(super) fn calculate_network_quality(stats: &RtpSessionStats) -> u8 {
        let packet_loss_percent = if stats.packets_received > 0 {
            (stats.packets_lost as f32 / (stats.packets_received + stats.packets_lost) as f32) * 100.0
        } else {
            0.0
        };
        
        // Score based on packet loss and jitter
        let mut score: f32 = 100.0;
        score -= packet_loss_percent * 5.0; // 5 points per percent loss
        score -= (stats.jitter_ms as f32).min(100.0) * 0.5; // 0.5 points per ms jitter
        
        score.max(0.0).min(100.0) as u8
    }
    
    /// Start statistics monitoring for a dialog
    pub async fn start_statistics_monitoring(&self, dialog_id: DialogId, interval_duration: Duration) -> Result<()> {
        info!("📊 Starting statistics monitoring for dialog: {} (interval: {:?})", dialog_id, interval_duration);
        
        // Verify session exists and get codec information
        let session_codec = {
            let sessions = self.sessions.read().await;
            let session_info = sessions.get(&dialog_id)
                .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
            session_info.config.preferred_codec.clone()
        };
        
        let event_tx = self.event_tx.clone();
        let dialog_id_clone = dialog_id.clone();
        
        // We can't clone RwLock directly, so we'll check session existence differently
        // Get the RTP session reference for monitoring
        let rtp_session = match self.get_rtp_session(&dialog_id).await {
            Some(session) => session,
            None => return Err(Error::session_not_found(dialog_id.as_str())),
        };
        
        tokio::spawn(async move {
            let mut interval_timer = interval(interval_duration);
            let mut last_quality_alert = Instant::now();
            
            // Use the captured codec information
            let current_codec = session_codec.clone().or_else(|| Some("PCMU".to_string()));
            
            loop {
                interval_timer.tick().await;
                
                // Get RTP statistics
                let stats = {
                    let session = rtp_session.lock().await;
                    session.get_stats()
                };
                
                // Calculate quality metrics
                let packet_loss_percent = if stats.packets_received > 0 {
                    (stats.packets_lost as f32 / (stats.packets_received + stats.packets_lost) as f32) * 100.0
                } else {
                    0.0
                };
                
                let quality_metrics = QualityMetrics {
                    packet_loss_percent,
                    jitter_ms: stats.jitter_ms,
                    rtt_ms: None,
                    mos_score: MediaSessionController::calculate_mos_from_stats(&stats),
                    network_quality: MediaSessionController::calculate_network_quality(&stats),
                };
                
                // Get stream statistics
                let stream_stats = {
                    let session = rtp_session.lock().await;
                    session.get_all_streams().await
                };
                
                // Create media statistics
                let media_stats = MediaStatistics {
                    session_id: MediaSessionId::new(&dialog_id_clone.to_string()),
                    dialog_id: dialog_id_clone.clone(),
                    rtp_stats: Some(stats.clone()),
                    stream_stats,
                    media_stats: MediaProcessingStats {
                        packets_processed: stats.packets_received,
                        frames_encoded: 0,
                        frames_decoded: 0,
                        processing_errors: 0,
                        codec_changes: 0,
                        current_codec: current_codec.clone(),
                    },
                    quality_metrics: Some(quality_metrics.clone()),
                    session_start: Instant::now(), // We don't have access to wrapper.created_at
                    session_duration: Duration::from_secs(0), // Will be calculated differently
                };
                
                // Send statistics update event
                let _ = event_tx.send(MediaSessionEvent::StatisticsUpdated {
                    dialog_id: dialog_id_clone.clone(),
                    stats: media_stats,
                });
                
                // Check for quality degradation
                if packet_loss_percent > 5.0 || stats.jitter_ms > 50.0 {
                    // Rate limit quality alerts to once per minute
                    if last_quality_alert.elapsed() > Duration::from_secs(60) {
                        let reason = if packet_loss_percent > 5.0 {
                            format!("High packet loss: {:.1}%", packet_loss_percent)
                        } else {
                            format!("High jitter: {}ms", stats.jitter_ms)
                        };
                        
                        warn!("⚠️ Quality degradation detected for {}: {}", dialog_id_clone, reason);
                        
                        let _ = event_tx.send(MediaSessionEvent::QualityDegraded {
                            dialog_id: dialog_id_clone.clone(),
                            metrics: quality_metrics,
                            reason,
                        });
                        
                        last_quality_alert = Instant::now();
                    }
                }
                
                debug!("📊 Stats for {}: packets_rx={}, loss={:.1}%, jitter={}ms", 
                       dialog_id_clone, stats.packets_received, packet_loss_percent, stats.jitter_ms);
            }
        });
        
        Ok(())
    }

} 

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::DialogId;
    use crate::relay::controller::types::MediaConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use tokio::time::{sleep, Duration};
    use std::collections::HashMap;
    
    async fn create_test_controller() -> MediaSessionController {
        MediaSessionController::new()
    }
    
    #[tokio::test]
    async fn test_codec_statistics_pcmu() {
        let controller = create_test_controller().await;
        let dialog_id = DialogId::new("test-dialog-pcmu");
        
        // Configure session with PCMU codec
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start media session
        controller.start_media(dialog_id.clone(), config).await.unwrap();
        
        // Get statistics
        let stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        
        // Verify codec is correctly tracked
        assert_eq!(stats.media_stats.current_codec, Some("PCMU".to_string()));
        
        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_codec_statistics_opus() {
        let controller = create_test_controller().await;
        let dialog_id = DialogId::new("test-dialog-opus");
        
        // Configure session with Opus codec
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("Opus".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start media session
        controller.start_media(dialog_id.clone(), config).await.unwrap();
        
        // Get statistics
        let stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        
        // Verify codec is correctly tracked
        assert_eq!(stats.media_stats.current_codec, Some("Opus".to_string()));
        
        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_codec_statistics_default() {
        let controller = create_test_controller().await;
        let dialog_id = DialogId::new("test-dialog-default");
        
        // Configure session with no preferred codec (should default to PCMU)
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: None,
            parameters: HashMap::new(),
        };
        
        // Start media session
        controller.start_media(dialog_id.clone(), config).await.unwrap();
        
        // Get statistics
        let stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        
        // Verify codec defaults to PCMU
        assert_eq!(stats.media_stats.current_codec, Some("PCMU".to_string()));
        
        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_codec_statistics_after_update() {
        let controller = create_test_controller().await;
        let dialog_id = DialogId::new("test-dialog-update");
        
        // Start with PCMU codec
        let initial_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        controller.start_media(dialog_id.clone(), initial_config).await.unwrap();
        
        // Verify initial codec
        let initial_stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        assert_eq!(initial_stats.media_stats.current_codec, Some("PCMU".to_string()));
        
        // Update to Opus codec
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("Opus".to_string()),
            parameters: HashMap::new(),
        };
        
        controller.update_media(dialog_id.clone(), updated_config).await.unwrap();
        
        // Verify updated codec
        let updated_stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        assert_eq!(updated_stats.media_stats.current_codec, Some("Opus".to_string()));
        
        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_statistics_monitoring_codec_tracking() {
        let controller = create_test_controller().await;
        let dialog_id = DialogId::new("test-dialog-monitoring");
        
        // Configure session with Opus codec
        let config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("Opus".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start media session
        controller.start_media(dialog_id.clone(), config).await.unwrap();
        
        // Get event receiver
        let mut event_receiver = controller.take_event_receiver().await.unwrap();
        
        // Start statistics monitoring
        let monitoring_interval = Duration::from_millis(100);
        controller.start_statistics_monitoring(dialog_id.clone(), monitoring_interval).await.unwrap();
        
        // Wait for at least one statistics update
        sleep(Duration::from_millis(150)).await;
        
        // Check for statistics update events
        let mut found_stats_event = false;
        while let Ok(event) = event_receiver.try_recv() {
            if let MediaSessionEvent::StatisticsUpdated { stats, .. } = event {
                // Verify the statistics contain the correct codec
                assert_eq!(stats.media_stats.current_codec, Some("Opus".to_string()));
                found_stats_event = true;
                break;
            }
        }
        
        assert!(found_stats_event, "Should have received statistics update event");
        
        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }
    
    #[tokio::test]
    async fn test_codec_statistics_multiple_sessions() {
        let controller = create_test_controller().await;
        let dialog_id_1 = DialogId::new("test-dialog-1");
        let dialog_id_2 = DialogId::new("test-dialog-2");
        
        // Configure first session with PCMU
        let config_1 = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        };
        
        // Configure second session with Opus
        let config_2 = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("Opus".to_string()),
            parameters: HashMap::new(),
        };
        
        // Start both sessions
        controller.start_media(dialog_id_1.clone(), config_1).await.unwrap();
        controller.start_media(dialog_id_2.clone(), config_2).await.unwrap();
        
        // Get statistics for both sessions
        let stats_1 = controller.get_media_statistics(&dialog_id_1).await.unwrap();
        let stats_2 = controller.get_media_statistics(&dialog_id_2).await.unwrap();
        
        // Verify each session has the correct codec
        assert_eq!(stats_1.media_stats.current_codec, Some("PCMU".to_string()));
        assert_eq!(stats_2.media_stats.current_codec, Some("Opus".to_string()));
        
        // Cleanup
        controller.stop_media(&dialog_id_1).await.unwrap();
        controller.stop_media(&dialog_id_2).await.unwrap();
    }
} 