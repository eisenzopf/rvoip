//! Statistics and monitoring functionality
//!
//! This module provides comprehensive statistics collection and monitoring
//! for RTP sessions, including quality metrics and MOS score calculation.

use std::time::{Duration, Instant};
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};
use crate::types::{
    DialogId, MediaProcessingStats, MediaSessionId, MediaStatistics, QualityMetrics,
};
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

        Some(format!(
            "RTP Session - Local: {}, SSRC: 0x{:08x}",
            local_addr, ssrc
        ))
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
        let current_codec = session_info
            .config
            .preferred_codec
            .clone()
            .or_else(|| Some("PCMU".to_string())); // Default to PCMU if none set

        // Calculate quality metrics from RTP stats
        let quality_metrics = rtp_stats.as_ref().and_then(|stats| {
            if stats.packets_received == 0 {
                return None;
            }
            Some(QualityMetrics::from_rtp_stats(stats))
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

    /// Current quality metrics for one dialog's media: local measurements
    /// of the received stream plus the peer's latest RTCP reception report
    /// about our stream (the `remote_*` fields, `None` without RTCP).
    ///
    /// Returns `None` when the dialog has no RTP session. Unlike
    /// [`get_media_statistics`](Self::get_media_statistics) this returns
    /// metrics even before the first packet is received, so a send-only
    /// call still reports its sent count and the peer's view of it.
    pub async fn get_media_quality(&self, dialog_id: &DialogId) -> Option<QualityMetrics> {
        let stats = self.get_rtp_statistics(dialog_id).await?;
        Some(QualityMetrics::from_rtp_stats(&stats))
    }

    /// Sample every RTP session that is mapped to a session-layer call and
    /// publish one `MediaToSessionEvent::MediaQualityUpdate` per call through
    /// the installed [`MediaEventHub`](crate::events::MediaEventHub).
    ///
    /// Sessions that have neither sent nor received RTP yet are skipped, so
    /// an update always carries a measurement. Returns the number of updates
    /// published; `0` when no event hub is installed. Call this on a timer
    /// to get periodic per-call quality (rvoip-sip does so when
    /// `Config::media_quality_interval` is set).
    pub async fn publish_media_quality_updates(&self) -> usize {
        let Some(hub) = self.event_hub.read().await.clone() else {
            return 0;
        };
        // Snapshot the targets so no DashMap shard guard is held across an
        // await; sessions torn down mid-sweep are simply skipped below.
        let targets: Vec<(DialogId, Instant, std::sync::Arc<tokio::sync::Mutex<_>>)> = self
            .rtp_sessions
            .iter()
            .map(|entry| {
                (
                    entry.key().clone(),
                    entry.value().created_at,
                    entry.value().session.clone(),
                )
            })
            .collect();

        let mut published = 0usize;
        for (dialog_id, created_at, rtp_session) in targets {
            if self
                .get_session_id(&MediaSessionId::from_dialog(&dialog_id))
                .is_none()
            {
                continue;
            }
            let stats = rtp_session.lock().await.get_stats();
            if stats.packets_sent == 0 && stats.packets_received == 0 {
                continue;
            }
            let quality = QualityMetrics::from_rtp_stats(&stats);
            let current_codec = self
                .sessions
                .get(&dialog_id)
                .and_then(|info| info.config.preferred_codec.clone());
            let event = MediaSessionEvent::StatisticsUpdated {
                dialog_id: dialog_id.clone(),
                stats: MediaStatistics {
                    session_id: MediaSessionId::from_dialog(&dialog_id),
                    dialog_id,
                    // Per-stream detail is left to `get_stream_statistics`;
                    // the periodic sample carries the session aggregate.
                    stream_stats: Vec::new(),
                    media_stats: MediaProcessingStats {
                        packets_processed: stats.packets_received,
                        current_codec,
                        ..MediaProcessingStats::default()
                    },
                    rtp_stats: Some(stats),
                    quality_metrics: Some(quality),
                    session_start: created_at,
                    session_duration: created_at.elapsed(),
                },
            };
            match hub.publish_media_event(event).await {
                Ok(()) => published += 1,
                Err(error) => {
                    debug!("Media quality update was not published: {}", error);
                }
            }
        }
        published
    }

    /// Start statistics monitoring for a dialog
    pub async fn start_statistics_monitoring(
        &self,
        dialog_id: DialogId,
        interval_duration: Duration,
    ) -> Result<()> {
        info!(
            "📊 Starting statistics monitoring for dialog: {} (interval: {:?})",
            dialog_id, interval_duration
        );

        // Verify session exists and get codec information. DashMap
        // shard guard is held only across the synchronous clone.
        let session_codec = self
            .sessions
            .get(&dialog_id)
            .map(|r| r.value().config.preferred_codec.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

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
                    (stats.packets_lost as f32
                        / (stats.packets_received + stats.packets_lost) as f32)
                        * 100.0
                } else {
                    0.0
                };

                let quality_metrics = QualityMetrics::from_rtp_stats(&stats);

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

                        warn!(
                            "⚠️ Quality degradation detected for {}: {}",
                            dialog_id_clone, reason
                        );

                        let _ = event_tx.send(MediaSessionEvent::QualityDegraded {
                            dialog_id: dialog_id_clone.clone(),
                            metrics: quality_metrics,
                            reason,
                        });

                        last_quality_alert = Instant::now();
                    }
                }

                debug!(
                    "📊 Stats for {}: packets_rx={}, loss={:.1}%, jitter={}ms",
                    dialog_id_clone, stats.packets_received, packet_loss_percent, stats.jitter_ms
                );
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::relay::controller::types::MediaConfig;
    use crate::types::DialogId;
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    async fn create_test_controller() -> MediaSessionController {
        MediaSessionController::new()
    }

    #[test]
    fn mos_requires_packets_and_uses_measured_rtt() {
        let mos = |stats: &RtpSessionStats| QualityMetrics::from_rtp_stats(stats).mos_score;
        assert_eq!(mos(&RtpSessionStats::default()), None);
        let low_latency = RtpSessionStats {
            packets_received: 100,
            rtt_ms: Some(20.0),
            ..RtpSessionStats::default()
        };
        let high_latency = RtpSessionStats {
            rtt_ms: Some(400.0),
            ..low_latency.clone()
        };
        assert!(mos(&high_latency) < mos(&low_latency));
    }

    #[tokio::test]
    async fn quality_sweep_publishes_real_session_ids_and_values_through_the_hub() {
        use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
        use rvoip_infra_common::events::cross_crate::{MediaToSessionEvent, RvoipCrossCrateEvent};
        use rvoip_infra_common::events::EventCoordinatorConfig;
        use std::sync::Arc;

        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let controller = Arc::new(MediaSessionController::new());
        let mut media_events = coordinator.subscribe("media_to_session").await.unwrap();
        // Without a hub nothing is published.
        assert_eq!(controller.publish_media_quality_updates().await, 0);
        let hub = crate::events::MediaEventHub::new(coordinator.clone(), controller.clone())
            .await
            .unwrap();
        controller.set_event_hub(hub).await;

        let local = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
        let receiver = DialogId::new("quality-receiver");
        controller
            .start_media(
                receiver.clone(),
                MediaConfig {
                    local_addr: local,
                    remote_addr: None,
                    preferred_codec: Some("PCMU".to_string()),
                    parameters: HashMap::new(),
                },
            )
            .await
            .unwrap();
        let receiver_port = controller
            .get_session_info(&receiver)
            .await
            .and_then(|info| info.rtp_port)
            .unwrap();
        let sender = DialogId::new("quality-sender");
        controller
            .start_media(
                sender.clone(),
                MediaConfig {
                    local_addr: local,
                    remote_addr: Some(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::LOCALHOST),
                        receiver_port,
                    )),
                    preferred_codec: Some("PCMU".to_string()),
                    parameters: HashMap::new(),
                },
            )
            .await
            .unwrap();
        // Idle sessions carry no measurement and are skipped.
        assert_eq!(controller.publish_media_quality_updates().await, 0);
        // Unmapped sessions have no session-layer owner and are skipped.
        for i in 0..10u32 {
            controller
                .encode_and_send_audio_frame(&sender, vec![0i16; 160], i * 160)
                .await
                .unwrap();
        }
        assert_eq!(controller.publish_media_quality_updates().await, 0);

        controller
            .store_session_mapping("call-sender".into(), MediaSessionId::from_dialog(&sender));
        controller.store_session_mapping(
            "call-receiver".into(),
            MediaSessionId::from_dialog(&receiver),
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while controller
                .get_media_quality(&receiver)
                .await
                .map_or(true, |q| q.packets_received == 0)
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("receiver never saw RTP");

        assert_eq!(controller.publish_media_quality_updates().await, 2);
        let mut seen = HashMap::new();
        while seen.len() < 2 {
            let event = tokio::time::timeout(Duration::from_secs(2), media_events.recv())
                .await
                .expect("quality update was not published")
                .expect("media_to_session closed");
            let Some(RvoipCrossCrateEvent::MediaToSession(
                MediaToSessionEvent::MediaQualityUpdate {
                    session_id,
                    quality_metrics,
                },
            )) = event
                .as_any()
                .downcast_ref::<RvoipCrossCrateEvent>()
                .cloned()
            else {
                continue;
            };
            seen.insert(session_id, quality_metrics);
        }
        let sent = &seen["call-sender"];
        assert!(sent.packets_sent >= 10, "{sent:?}");
        let received = &seen["call-receiver"];
        assert!(received.packets_received > 0, "{received:?}");
        assert!(received.mos_score > 0.0, "{received:?}");
        // No RTCP has been exchanged in this harness.
        assert_eq!(received.remote_packet_loss, None);

        controller.stop_media(&sender).await.unwrap();
        controller.stop_media(&receiver).await.unwrap();
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
        controller
            .start_media(dialog_id.clone(), config)
            .await
            .unwrap();

        // Get statistics
        let stats = controller.get_media_statistics(&dialog_id).await.unwrap();

        // Verify codec is correctly tracked
        assert_eq!(stats.media_stats.current_codec, Some("PCMU".to_string()));

        // Cleanup
        controller.stop_media(&dialog_id).await.unwrap();
    }

    #[cfg(feature = "opus")]
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
        controller
            .start_media(dialog_id.clone(), config)
            .await
            .unwrap();

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
        controller
            .start_media(dialog_id.clone(), config)
            .await
            .unwrap();

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

        controller
            .start_media(dialog_id.clone(), initial_config)
            .await
            .unwrap();

        // Verify initial codec
        let initial_stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        assert_eq!(
            initial_stats.media_stats.current_codec,
            Some("PCMU".to_string())
        );

        // Update to another always-available codec.
        let updated_config = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };

        controller
            .update_media(dialog_id.clone(), updated_config)
            .await
            .unwrap();

        // Verify updated codec
        let updated_stats = controller.get_media_statistics(&dialog_id).await.unwrap();
        assert_eq!(
            updated_stats.media_stats.current_codec,
            Some("PCMA".to_string())
        );

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

        // Configure second session with another always-available codec.
        let config_2 = MediaConfig {
            local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0),
            remote_addr: None,
            preferred_codec: Some("PCMA".to_string()),
            parameters: HashMap::new(),
        };

        // Start both sessions
        controller
            .start_media(dialog_id_1.clone(), config_1)
            .await
            .unwrap();
        controller
            .start_media(dialog_id_2.clone(), config_2)
            .await
            .unwrap();

        // Get statistics for both sessions
        let stats_1 = controller.get_media_statistics(&dialog_id_1).await.unwrap();
        let stats_2 = controller.get_media_statistics(&dialog_id_2).await.unwrap();

        // Verify each session has the correct codec
        assert_eq!(stats_1.media_stats.current_codec, Some("PCMU".to_string()));
        assert_eq!(stats_2.media_stats.current_codec, Some("PCMA".to_string()));

        // Cleanup
        controller.stop_media(&dialog_id_1).await.unwrap();
        controller.stop_media(&dialog_id_2).await.unwrap();
    }
}
