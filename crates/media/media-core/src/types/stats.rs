//! Media statistics types that combine various statistics sources

use super::{DialogId, MediaSessionId};
use rvoip_infra_common::events::cross_crate::MediaQualityMetrics;
use rvoip_rtp_core::session::{RtpSessionStats, RtpStreamStats};
use std::time::{Duration, Instant};

/// Comprehensive media statistics for a session
#[derive(Debug, Clone)]
pub struct MediaStatistics {
    /// Session identifiers
    pub session_id: MediaSessionId,
    pub dialog_id: DialogId,

    /// RTP/RTCP statistics from rtp-core
    pub rtp_stats: Option<RtpSessionStats>,

    /// Per-stream statistics (for multi-stream scenarios)
    pub stream_stats: Vec<RtpStreamStats>,

    /// Media processing statistics
    pub media_stats: MediaProcessingStats,

    /// Quality metrics
    pub quality_metrics: Option<QualityMetrics>,

    /// Session timing
    pub session_start: Instant,
    pub session_duration: Duration,
}

/// Media processing statistics
#[derive(Debug, Clone, Default)]
pub struct MediaProcessingStats {
    /// Packets processed
    pub packets_processed: u64,

    /// Frames encoded
    pub frames_encoded: u64,

    /// Frames decoded
    pub frames_decoded: u64,

    /// Processing errors
    pub processing_errors: u64,

    /// Codec changes
    pub codec_changes: u32,

    /// Current codec
    pub current_codec: Option<String>,
}

/// Per-call quality metrics: local measurements of the received stream plus,
/// once the peer's RTCP arrives, what the peer reported about the stream we
/// send.
///
/// Built by [`MediaSessionController::get_media_quality`] and published per
/// call by [`MediaSessionController::publish_media_quality_updates`].
///
/// [`MediaSessionController::get_media_quality`]: crate::relay::controller::MediaSessionController::get_media_quality
/// [`MediaSessionController::publish_media_quality_updates`]: crate::relay::controller::MediaSessionController::publish_media_quality_updates
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    /// Locally observed loss on the received stream, percent (0–100):
    /// sequence gaps over packets expected.
    pub packet_loss_percent: f32,

    /// Locally measured interarrival jitter of the received stream, ms.
    pub jitter_ms: f64,

    /// Round-trip time in milliseconds (from RTCP SR/RR LSR/DLSR). `None`
    /// until the peer reflects one of our sender reports.
    pub rtt_ms: Option<f64>,

    /// MOS score estimate (1-5) for the received stream. `None` until
    /// packets have been received.
    pub mos_score: Option<f32>,

    /// Network quality indicator (0-100)
    pub network_quality: u8,

    /// RTP packets sent.
    pub packets_sent: u64,

    /// RTP packets received.
    pub packets_received: u64,

    /// Packets missing from the received stream (sequence gaps).
    pub packets_lost: u64,

    /// Loss the peer reported on our sent stream over its last reporting
    /// interval, percent (0–100). `None` until an RTCP report about our
    /// stream arrives.
    pub remote_packet_loss_percent: Option<f32>,

    /// Cumulative count of our packets the peer reported lost (may be
    /// negative when the peer saw duplicates). `None` without RTCP.
    pub remote_packets_lost: Option<i64>,

    /// Interarrival jitter the peer reported on our sent stream, ms.
    /// `None` without RTCP.
    pub remote_jitter_ms: Option<f64>,

    /// Age of the peer report the `remote_*` fields came from. Lets a
    /// caller notice a peer that stopped sending RTCP.
    pub remote_report_age: Option<Duration>,
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            packet_loss_percent: 0.0,
            jitter_ms: 0.0,
            rtt_ms: None,
            // Unknown is not excellent. A score exists only after packets
            // have produced measurements.
            mos_score: None,
            network_quality: 0,
            packets_sent: 0,
            packets_received: 0,
            packets_lost: 0,
            remote_packet_loss_percent: None,
            remote_packets_lost: None,
            remote_jitter_ms: None,
            remote_report_age: None,
        }
    }
}

impl QualityMetrics {
    /// Derive quality metrics from an RTP session's statistics, including
    /// the peer's latest RTCP reception report when one has arrived.
    pub fn from_rtp_stats(stats: &RtpSessionStats) -> Self {
        let expected = stats.packets_received.saturating_add(stats.packets_lost);
        let packet_loss_percent = if expected > 0 {
            (stats.packets_lost as f32 / expected as f32) * 100.0
        } else {
            0.0
        };
        let mos_score = (stats.packets_received > 0).then(|| {
            crate::quality::metrics::QualityMetrics::calculate_mos(
                packet_loss_percent,
                stats.jitter_ms as f32,
                stats.rtt_ms.unwrap_or(0.0) as f32,
            )
        });
        // Score based on packet loss and jitter
        let mut score: f32 = 100.0;
        score -= packet_loss_percent * 5.0; // 5 points per percent loss
        score -= (stats.jitter_ms as f32).min(100.0) * 0.5; // 0.5 points per ms jitter
        let network_quality = score.clamp(0.0, 100.0) as u8;

        let peer = stats.peer_report.as_ref();
        Self {
            packet_loss_percent,
            jitter_ms: stats.jitter_ms,
            rtt_ms: stats.rtt_ms,
            mos_score,
            network_quality,
            packets_sent: stats.packets_sent,
            packets_received: stats.packets_received,
            packets_lost: stats.packets_lost,
            remote_packet_loss_percent: peer.map(|r| (r.fraction_lost_ratio() * 100.0) as f32),
            remote_packets_lost: peer.map(|r| i64::from(r.cumulative_lost)),
            remote_jitter_ms: peer.map(|r| r.jitter_ms),
            remote_report_age: peer.map(|r| r.received_at.elapsed()),
        }
    }

    /// Project onto the cross-crate representation that session layers
    /// receive in `MediaToSessionEvent::MediaQualityUpdate`.
    pub fn to_session_metrics(&self) -> MediaQualityMetrics {
        MediaQualityMetrics {
            mos_score: self.mos_score.map_or(0.0, f64::from),
            packet_loss: f64::from(self.packet_loss_percent) / 100.0,
            jitter_ms: self.jitter_ms,
            delay_ms: self.rtt_ms.map_or(0, |rtt| (rtt / 2.0).round() as u64),
            packets_sent: self.packets_sent,
            packets_received: self.packets_received,
            packets_lost: self.packets_lost,
            rtt_ms: self.rtt_ms,
            remote_packet_loss: self
                .remote_packet_loss_percent
                .map(|percent| f64::from(percent) / 100.0),
            remote_packets_lost: self.remote_packets_lost,
            remote_jitter_ms: self.remote_jitter_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_quality_does_not_claim_an_excellent_mos() {
        let metrics = QualityMetrics::default();
        assert_eq!(metrics.mos_score, None);
        assert_eq!(metrics.rtt_ms, None);
        assert_eq!(metrics.network_quality, 0);
        assert_eq!(metrics.remote_packet_loss_percent, None);
    }

    #[test]
    fn local_only_stats_leave_remote_fields_absent() {
        let stats = RtpSessionStats {
            packets_sent: 250,
            packets_received: 90,
            packets_lost: 10,
            jitter_ms: 4.0,
            ..RtpSessionStats::default()
        };
        let metrics = QualityMetrics::from_rtp_stats(&stats);
        assert_eq!(metrics.packets_sent, 250);
        assert_eq!(metrics.packets_received, 90);
        assert!((metrics.packet_loss_percent - 10.0).abs() < 1e-4);
        assert!(metrics.mos_score.is_some());
        assert_eq!(metrics.rtt_ms, None);
        assert_eq!(metrics.remote_packet_loss_percent, None);
        assert_eq!(metrics.remote_packets_lost, None);
        assert_eq!(metrics.remote_jitter_ms, None);

        let session = metrics.to_session_metrics();
        assert!((session.packet_loss - 0.10).abs() < 1e-6);
        assert_eq!(session.delay_ms, 0);
        assert_eq!(session.rtt_ms, None);
        assert_eq!(session.remote_packet_loss, None);
    }

    #[test]
    fn peer_reception_report_populates_remote_fields_and_rtt() {
        let mut block = rvoip_rtp_core::packet::rtcp::RtcpReportBlock::new(0x1234);
        block.fraction_lost = 51; // ~19.9 percent
        block.cumulative_lost = 40;
        block.jitter = 80; // 10 ms at 8 kHz
        let report = rvoip_rtp_core::session::PeerReceptionReport::from_report_block(
            0x9999,
            &block,
            8_000,
            Instant::now(),
        );
        let stats = RtpSessionStats {
            packets_sent: 500,
            rtt_ms: Some(42.0),
            peer_report: Some(report),
            ..RtpSessionStats::default()
        };
        let metrics = QualityMetrics::from_rtp_stats(&stats);
        // Nothing received yet: no MOS, but the remote view is present.
        assert_eq!(metrics.mos_score, None);
        assert!((metrics.remote_packet_loss_percent.unwrap() - 19.921875).abs() < 1e-4);
        assert_eq!(metrics.remote_packets_lost, Some(40));
        assert!((metrics.remote_jitter_ms.unwrap() - 10.0).abs() < 1e-9);
        assert!(metrics.remote_report_age.is_some());

        let session = metrics.to_session_metrics();
        assert_eq!(session.mos_score, 0.0);
        assert_eq!(session.rtt_ms, Some(42.0));
        assert_eq!(session.delay_ms, 21);
        assert!((session.remote_packet_loss.unwrap() - 0.19921875).abs() < 1e-6);
    }
}
