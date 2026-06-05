//! RTP frame pumps between webrtc-rs tracks and rvoip-core channels.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtc::statistics::report::RTCStatsReport;

use bytes::BytesMut;
use chrono::Utc;
use parking_lot::Mutex;
use rtc::rtp;
use rtc::shared::marshal::{Marshal, MarshalSize, Unmarshal};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use tracing::warn;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_local::TrackLocal;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, QualitySnapshot};

pub const FRAME_CHANNEL_CAP: usize = 64;

/// Default deadline applied when a caller of `spawn_inbound_pump` does not
/// provide one. Keep in sync with `WebRtcConfig::inbound_send_deadline_ms`.
pub const DEFAULT_INBOUND_SEND_DEADLINE_MS: u64 = 200;

/// Default Opus payload type registered in the media engine.
pub const OPUS_PT_DEFAULT: u8 = 111;

/// Inbound RTP statistics for [`QualitySnapshot`].
#[derive(Default)]
pub struct InboundStats {
    packets: AtomicU64,
    /// Frames dropped because the downstream consumer was too slow.
    pub frames_dropped: AtomicU64,
    jitter_ms: Mutex<f32>,
    last_arrival: Mutex<Option<Instant>>,
    webrtc_jitter_ms: Mutex<f32>,
    webrtc_packet_loss_pct: Mutex<f32>,
    webrtc_packets_received: AtomicU64,
    webrtc_bytes_received: AtomicU64,
    webrtc_packets_lost: AtomicU64,
    // G4 — outbound aggregates.
    webrtc_packets_sent: AtomicU64,
    webrtc_bytes_sent: AtomicU64,
    webrtc_retransmitted_packets: AtomicU64,
    webrtc_retransmitted_bytes: AtomicU64,
    webrtc_nack_count: AtomicU64,
    webrtc_pli_count: AtomicU64,
    webrtc_fir_count: AtomicU64,
    // G4 — selected candidate pair snapshot. Stored under a Mutex because
    // the fields are not all atomic-friendly.
    selected_pair: Mutex<Option<CandidatePairStats>>,
}

/// Typed snapshot of webrtc-rs outbound RTP statistics. New in G4.
#[derive(Clone, Debug, Default)]
pub struct OutboundStats {
    pub packets_sent: u64,
    pub bytes_sent: u64,
    pub retransmitted_packets: u64,
    pub retransmitted_bytes: u64,
    pub nack_count: u64,
    pub pli_count: u64,
    pub fir_count: u64,
}

/// Typed snapshot of the selected ICE candidate pair (W3C
/// `RTCIceCandidatePairStats`). New in G4. `None` until ICE has nominated a
/// pair (typically a few seconds after `setLocalDescription`).
#[derive(Clone, Debug, Default)]
pub struct CandidatePairStats {
    /// `"host"` | `"srflx"` | `"prflx"` | `"relay"` — derived from the
    /// local candidate's type. Empty if not yet known.
    pub local_candidate_type: String,
    pub remote_candidate_type: String,
    /// Round-trip time of the most recent STUN response, in milliseconds.
    pub current_round_trip_time_ms: Option<f64>,
    /// Accumulated total RTT across all STUN responses on this pair.
    pub total_round_trip_time_ms: Option<f64>,
    /// Estimated outgoing bitrate in bits/sec (from GCC / TWCC).
    pub available_outgoing_bitrate_bps: Option<u64>,
    pub responses_received: u64,
    pub nominated: bool,
}

/// Typed snapshot of webrtc-rs inbound RTP statistics. Returned by
/// [`crate::media::WebRtcMediaStream::webrtc_stats_snapshot`].
#[derive(Clone, Debug, Default)]
pub struct WebRtcStatsSnapshot {
    pub packets_received: u64,
    pub bytes_received: u64,
    pub packets_lost: u64,
    pub jitter_ms: f32,
    pub packet_loss_pct: f32,
    /// MOS estimate (1.0–4.5) derived from jitter + packet loss.
    pub mos: f32,
    /// Frames that the inbound pump had to drop because the downstream
    /// consumer was too slow (see `WebRtcConfig::inbound_send_deadline_ms`).
    pub frames_dropped: u64,
    /// G4 — outbound RTP aggregates (sender side). Always present;
    /// fields are zero until the first outbound stat poll completes.
    pub outbound: OutboundStats,
    /// G4 — selected candidate pair (`None` until ICE has nominated).
    pub selected_pair: Option<CandidatePairStats>,
}

impl InboundStats {
    fn record_packet(&self) {
        self.packets.fetch_add(1, Ordering::Relaxed);
        let now = Instant::now();
        let mut last = self.last_arrival.lock();
        if let Some(prev) = *last {
            let delta_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
            // Exponential moving average of inter-arrival jitter.
            let mut jitter = self.jitter_ms.lock();
            *jitter = if *jitter == 0.0 {
                delta_ms.abs()
            } else {
                *jitter * 0.9 + delta_ms.abs() * 0.1
            };
        }
        *last = Some(now);
    }

    /// Merge inbound / outbound / candidate-pair data from a webrtc-rs stats
    /// report. G4: also harvests `outbound-rtp` and the nominated
    /// `candidate-pair` so the snapshot includes the sender side and the
    /// network path.
    pub fn merge_webrtc_report(&self, report: &RTCStatsReport) {
        if let Some(inbound) = report.inbound_rtp_streams().next() {
            let received = inbound.received_rtp_stream_stats.packets_received;
            let lost = inbound.received_rtp_stream_stats.packets_lost.max(0) as u64;
            let bytes = inbound.bytes_received;
            let total = received.saturating_add(lost);
            let loss_pct = if total == 0 {
                0.0
            } else {
                (lost as f32 / total as f32) * 100.0
            };
            *self.webrtc_jitter_ms.lock() =
                (inbound.received_rtp_stream_stats.jitter * 1000.0) as f32;
            *self.webrtc_packet_loss_pct.lock() = loss_pct;
            self.webrtc_packets_received
                .store(received, Ordering::Relaxed);
            self.webrtc_bytes_received.store(bytes, Ordering::Relaxed);
            self.webrtc_packets_lost.store(lost, Ordering::Relaxed);
        }

        // G4 — aggregate outbound RTP across all senders.
        let mut packets_sent: u64 = 0;
        let mut bytes_sent: u64 = 0;
        let mut retx_packets: u64 = 0;
        let mut retx_bytes: u64 = 0;
        let mut nack_count: u64 = 0;
        let mut pli_count: u64 = 0;
        let mut fir_count: u64 = 0;
        let mut any_outbound = false;
        for outbound in report.outbound_rtp_streams() {
            any_outbound = true;
            packets_sent = packets_sent.saturating_add(outbound.sent_rtp_stream_stats.packets_sent);
            bytes_sent = bytes_sent.saturating_add(outbound.sent_rtp_stream_stats.bytes_sent);
            retx_packets = retx_packets.saturating_add(outbound.retransmitted_packets_sent);
            retx_bytes = retx_bytes.saturating_add(outbound.retransmitted_bytes_sent);
            nack_count = nack_count.saturating_add(outbound.nack_count as u64);
            pli_count = pli_count.saturating_add(outbound.pli_count as u64);
            fir_count = fir_count.saturating_add(outbound.fir_count as u64);
        }
        if any_outbound {
            self.webrtc_packets_sent
                .store(packets_sent, Ordering::Relaxed);
            self.webrtc_bytes_sent.store(bytes_sent, Ordering::Relaxed);
            self.webrtc_retransmitted_packets
                .store(retx_packets, Ordering::Relaxed);
            self.webrtc_retransmitted_bytes
                .store(retx_bytes, Ordering::Relaxed);
            self.webrtc_nack_count.store(nack_count, Ordering::Relaxed);
            self.webrtc_pli_count.store(pli_count, Ordering::Relaxed);
            self.webrtc_fir_count.store(fir_count, Ordering::Relaxed);
        }

        // G4 — selected (nominated) candidate pair. Build a small lookup
        // from candidate id → type so we can translate the pair's
        // local/remote ids.
        let mut local_types: std::collections::HashMap<String, String> = Default::default();
        let mut remote_types: std::collections::HashMap<String, String> = Default::default();
        for entry in report.iter() {
            use rtc::statistics::report::RTCStatsReportEntry;
            match entry {
                RTCStatsReportEntry::LocalCandidate(c) => {
                    local_types.insert(c.stats.id.clone(), candidate_type_label(&c.candidate_type));
                }
                RTCStatsReportEntry::RemoteCandidate(c) => {
                    remote_types
                        .insert(c.stats.id.clone(), candidate_type_label(&c.candidate_type));
                }
                _ => {}
            }
        }

        if let Some(pair) = report.candidate_pairs().find(|p| p.nominated) {
            let curr_rtt =
                if pair.current_round_trip_time.is_finite() && pair.current_round_trip_time > 0.0 {
                    Some(pair.current_round_trip_time * 1000.0)
                } else {
                    None
                };
            let total_rtt =
                if pair.total_round_trip_time.is_finite() && pair.total_round_trip_time > 0.0 {
                    Some(pair.total_round_trip_time * 1000.0)
                } else {
                    None
                };
            let avail_out = if pair.available_outgoing_bitrate.is_finite()
                && pair.available_outgoing_bitrate > 0.0
            {
                Some(pair.available_outgoing_bitrate as u64)
            } else {
                None
            };
            let snap = CandidatePairStats {
                local_candidate_type: local_types
                    .get(&pair.local_candidate_id)
                    .cloned()
                    .unwrap_or_default(),
                remote_candidate_type: remote_types
                    .get(&pair.remote_candidate_id)
                    .cloned()
                    .unwrap_or_default(),
                current_round_trip_time_ms: curr_rtt,
                total_round_trip_time_ms: total_rtt,
                available_outgoing_bitrate_bps: avail_out,
                responses_received: pair.responses_received,
                nominated: true,
            };
            *self.selected_pair.lock() = Some(snap);
        }
    }

    pub fn snapshot(&self) -> QualitySnapshot {
        let pump_jitter = *self.jitter_ms.lock();
        let webrtc_jitter = *self.webrtc_jitter_ms.lock();
        let webrtc_loss = *self.webrtc_packet_loss_pct.lock();
        let jitter_ms = if webrtc_jitter > 0.0 {
            webrtc_jitter
        } else {
            pump_jitter
        };
        QualitySnapshot {
            jitter_ms,
            packet_loss_pct: webrtc_loss,
            mos: Some(estimate_mos(jitter_ms, webrtc_loss)),
        }
    }

    /// Full typed snapshot of inbound + outbound + candidate-pair webrtc-rs
    /// statistics (richer than the core `QualitySnapshot`).
    pub fn webrtc_snapshot(&self) -> WebRtcStatsSnapshot {
        let pump_jitter = *self.jitter_ms.lock();
        let webrtc_jitter = *self.webrtc_jitter_ms.lock();
        let webrtc_loss = *self.webrtc_packet_loss_pct.lock();
        let jitter_ms = if webrtc_jitter > 0.0 {
            webrtc_jitter
        } else {
            pump_jitter
        };
        WebRtcStatsSnapshot {
            packets_received: self.webrtc_packets_received.load(Ordering::Relaxed),
            bytes_received: self.webrtc_bytes_received.load(Ordering::Relaxed),
            packets_lost: self.webrtc_packets_lost.load(Ordering::Relaxed),
            jitter_ms,
            packet_loss_pct: webrtc_loss,
            mos: estimate_mos(jitter_ms, webrtc_loss),
            frames_dropped: self.frames_dropped.load(Ordering::Relaxed),
            outbound: OutboundStats {
                packets_sent: self.webrtc_packets_sent.load(Ordering::Relaxed),
                bytes_sent: self.webrtc_bytes_sent.load(Ordering::Relaxed),
                retransmitted_packets: self.webrtc_retransmitted_packets.load(Ordering::Relaxed),
                retransmitted_bytes: self.webrtc_retransmitted_bytes.load(Ordering::Relaxed),
                nack_count: self.webrtc_nack_count.load(Ordering::Relaxed),
                pli_count: self.webrtc_pli_count.load(Ordering::Relaxed),
                fir_count: self.webrtc_fir_count.load(Ordering::Relaxed),
            },
            selected_pair: self.selected_pair.lock().clone(),
        }
    }
}

/// Translate a webrtc-rs candidate type enum into a W3C-style
/// `"host"`/`"srflx"`/`"prflx"`/`"relay"` label without coupling to the
/// concrete enum variant names (which differ between rtc-0.20-alpha
/// revisions).
fn candidate_type_label(t: &rtc::peer_connection::transport::RTCIceCandidateType) -> String {
    let s = format!("{:?}", t).to_ascii_lowercase();
    if s.contains("relay") {
        "relay".into()
    } else if s.contains("srflx") || s.contains("server") {
        "srflx".into()
    } else if s.contains("prflx") || s.contains("peer") {
        "prflx".into()
    } else {
        "host".into()
    }
}

/// Simple MOS approximation derived from jitter and packet loss only — not
/// a full E-model, but good enough for monitoring trends. Range: 1.0–4.5.
fn estimate_mos(jitter_ms: f32, packet_loss_pct: f32) -> f32 {
    let raw = 4.5_f32 - (packet_loss_pct * 0.1) - (jitter_ms * 0.02);
    raw.clamp(1.0, 4.5)
}

/// Spawn a task that reads RTP from a remote track into `frames_in`.
///
/// `send_deadline_ms`: bounded send timeout. A slow downstream consumer no
/// longer stalls the inbound RTP task forever — exceeding the deadline drops
/// the frame and bumps `InboundStats::frames_dropped`.
///
/// `cancel`: if provided, fires `notify_waiters()` to exit the pump cleanly.
/// Channel close still exits as before.
pub fn spawn_inbound_pump(
    track: Arc<dyn TrackRemote>,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    stats: Arc<InboundStats>,
    send_deadline_ms: u64,
    cancel: Option<Arc<Notify>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let deadline = Duration::from_millis(send_deadline_ms);
        loop {
            // Honor cancellation as well as the per-poll timeout.
            let poll = tokio::time::timeout(Duration::from_millis(100), track.poll());
            let event = match &cancel {
                Some(c) => tokio::select! {
                    _ = c.notified() => break,
                    e = poll => e.ok().flatten(),
                },
                None => poll.await.ok().flatten(),
            };
            let Some(event) = event else { continue };
            match event {
                TrackRemoteEvent::OnRtpPacket(pkt) => {
                    stats.record_packet();
                    // D4 follow-up — emit codec payload bytes only (no RTP
                    // header). The orchestrator's `Transcoder` consumes
                    // codec bytes; the local outbound pump re-wraps with
                    // a fresh header. Preserve the timestamp in the
                    // `MediaFrame.timestamp_rtp` field so it survives the
                    // transcode pass.
                    let frame = MediaFrame {
                        stream_id: stream_id.clone(),
                        kind: rvoip_core::stream::StreamKind::Audio,
                        payload: pkt.payload.clone(),
                        timestamp_rtp: pkt.header.timestamp,
                        captured_at: Utc::now(),
                        // Gap plan §4.3 — carry the wire RTP payload-type
                        // so the cross-transport `frame_pump` can route
                        // RFC 4733 telephone-events (PT 101 by convention)
                        // distinctly from audio.
                        payload_type: Some(pkt.header.payload_type),
                    };
                    match tokio::time::timeout(deadline, frames_in_tx.send(frame)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => break, // downstream dropped — stop pumping
                        Err(_) => {
                            // Slow consumer; drop the frame and count it.
                            let dropped = stats.frames_dropped.fetch_add(1, Ordering::Relaxed) + 1;
                            if dropped.is_power_of_two() {
                                warn!(
                                    dropped,
                                    deadline_ms = send_deadline_ms,
                                    "inbound media pump dropped frame (slow consumer)"
                                );
                            }
                        }
                    }
                }
                TrackRemoteEvent::OnEnded | TrackRemoteEvent::OnError => break,
                _ => {}
            }
        }
    })
}

/// Spawn a task that reads `frames_out` and writes RTP to a local track.
///
/// D4 follow-up — `MediaFrame.payload` is expected to be **codec payload
/// bytes** (no RTP header). The pump wraps each frame in a fresh RTP
/// packet using the supplied `ssrc` and `payload_type` and an internal
/// sequence counter. `MediaFrame.timestamp_rtp` is honored when non-zero;
/// otherwise the pump derives a monotonically-increasing timestamp from
/// the codec's expected frame duration.
///
/// Legacy callers that put full RTP wire bytes in `MediaFrame.payload`
/// (pre-D4) are still tolerated — if the payload parses as a valid RTP
/// packet, we forward it verbatim (the test fixtures
/// `silent_rtp_payload_for_ssrc` use this path).
pub fn spawn_outbound_pump(
    track: Arc<TrackLocalStaticRTP>,
    mut frames_out_rx: mpsc::Receiver<MediaFrame>,
    ssrc: u32,
    payload_type: u8,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut seq: u16 = 1;
        let mut next_timestamp: u32 = 0;
        while let Some(frame) = frames_out_rx.recv().await {
            // Try the legacy "full RTP wire image" path first — preserves
            // the long-standing semantics of `silent_rtp_payload_for_ssrc`
            // and the `loopback_rtp_inbound_round_trip` test gate.
            let pkt = if let Ok(mut legacy) = bytes_to_rtp_packet(&frame.payload) {
                if legacy.header.timestamp == 0 {
                    legacy.header.timestamp = frame.timestamp_rtp;
                }
                legacy
            } else {
                // New contract — codec bytes only, wrap in a fresh RTP header.
                let ts = if frame.timestamp_rtp != 0 {
                    frame.timestamp_rtp
                } else {
                    let t = next_timestamp;
                    next_timestamp = next_timestamp.wrapping_add(960); // 20 ms @ 48 kHz default
                    t
                };
                let pkt = rtp::Packet {
                    header: rtp::Header {
                        version: 2,
                        padding: false,
                        extension: false,
                        marker: false,
                        payload_type,
                        sequence_number: seq,
                        timestamp: ts,
                        ssrc,
                        ..Default::default()
                    },
                    payload: frame.payload,
                };
                seq = seq.wrapping_add(1);
                pkt
            };

            loop {
                match track.write_rtp(pkt.clone()).await {
                    Ok(()) => break,
                    Err(e) if e.to_string().contains("not binding") => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    Err(_) => return,
                }
            }
        }
    })
}

fn rtp_packet_to_bytes(pkt: &rtp::Packet) -> bytes::Bytes {
    let size = pkt.marshal_size();
    let mut buf = BytesMut::with_capacity(size);
    buf.resize(size, 0);
    if pkt.marshal_to(&mut buf).is_ok() {
        buf.freeze()
    } else {
        bytes::Bytes::new()
    }
}

fn bytes_to_rtp_packet(data: &bytes::Bytes) -> Result<rtp::Packet, rtc::shared::error::Error> {
    let mut buf = data.clone();
    // Pre-D4 this function discarded the unmarshal result and always
    // returned `Packet::default()` (PT=0, SSRC=0, empty payload), which
    // meant the legacy outbound-pump path silently wrote zero-byte
    // PCMU packets instead of the caller's RTP bytes. Tests only ever
    // asserted `!payload.is_empty()` after a remote round-trip, so the
    // regression hid in plain sight. Fixed here as part of the D4
    // contract reconciliation.
    let pkt = rtp::Packet::unmarshal(&mut buf)?;
    Ok(pkt)
}

/// Build a minimal silent Opus RTP packet for loopback tests.
pub fn silent_rtp_payload(seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet(seq, timestamp))
}

/// Marshalled silent Opus RTP with an explicit SSRC.
pub fn silent_rtp_payload_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet_for_ssrc(ssrc, seq, timestamp))
}

/// Parsed silent Opus RTP packet for direct `TrackLocal::write_rtp`.
pub fn silent_rtp_packet(seq: u16, timestamp: u32) -> rtp::Packet {
    silent_rtp_packet_for_ssrc(1, seq, timestamp)
}

/// Parsed silent Opus RTP packet with an explicit SSRC (must match the local track).
pub fn silent_rtp_packet_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type: 111,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE]),
    }
}

/// D3a — wrap an arbitrary Opus payload in a marshalled RTP packet.
///
/// Used by `CpalAudioSource` to hand
/// already-encoded Opus frames to the outbound pump. The PT is fixed at
/// 111 to match the codec registered by
/// [`build_media_engine`](crate::peer::builder::build_media_engine).
pub fn opus_rtp_payload(
    ssrc: u32,
    seq: u16,
    timestamp: u32,
    marker: bool,
    opus_bytes: bytes::Bytes,
) -> bytes::Bytes {
    let pkt = rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker,
            payload_type: OPUS_PT_DEFAULT,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: opus_bytes,
    };
    rtp_packet_to_bytes(&pkt)
}

/// Parsed silent VP8 RTP packet with an explicit SSRC.
pub fn silent_vp8_rtp_packet_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> rtp::Packet {
    rtp::Packet {
        header: rtp::Header {
            version: 2,
            padding: false,
            extension: false,
            marker: false,
            payload_type: crate::peer::builder::VP8_PAYLOAD_TYPE,
            sequence_number: seq,
            timestamp,
            ssrc,
            ..Default::default()
        },
        payload: bytes::Bytes::from_static(&[0x10, 0x00, 0x00, 0x00]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D4 follow-up — the legacy-compat path on the outbound pump must
    /// detect a full RTP wire image in `MediaFrame.payload` and forward it
    /// verbatim, so the long-standing `silent_rtp_payload_for_ssrc` fixture
    /// (used by `loopback_rtp_inbound_round_trip` and the QUIC bridge
    /// test) keeps working.
    #[test]
    fn legacy_rtp_payload_round_trips_through_bytes_to_rtp_packet() {
        let bytes = silent_rtp_payload_for_ssrc(0xCAFEBABE, 42, 9600);
        let pkt = bytes_to_rtp_packet(&bytes).expect("legacy RTP must parse");
        assert_eq!(pkt.header.payload_type, OPUS_PT_DEFAULT);
        assert_eq!(pkt.header.ssrc, 0xCAFEBABE);
    }

    /// D4 follow-up — codec payload bytes must NOT happen to parse as a
    /// valid RTP packet (otherwise the legacy-compat path would
    /// mis-trigger). The 3-byte silent Opus payload (`F8 FF FE`) has
    /// version bits `11` (≠ 2), so it's reliably rejected.
    #[test]
    fn opus_silence_codec_bytes_do_not_parse_as_rtp() {
        let raw_opus = bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE]);
        assert!(
            bytes_to_rtp_packet(&raw_opus).is_err(),
            "Opus codec bytes must be rejected as RTP — otherwise the outbound pump's legacy path mis-fires"
        );
    }
}
