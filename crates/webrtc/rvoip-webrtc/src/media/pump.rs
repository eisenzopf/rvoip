//! RTP frame pumps between webrtc-rs tracks and rvoip-core channels.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rtc::statistics::report::RTCStatsReport;

use bytes::BytesMut;
use chrono::Utc;
use parking_lot::Mutex;
use rtc::rtp;
#[cfg(test)]
use rtc::shared::marshal::Unmarshal;
use rtc::shared::marshal::{Marshal, MarshalSize};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;
use tracing::warn;
use webrtc::media_stream::track_local::static_rtp::TrackLocalStaticRTP;
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, QualitySnapshot};

use crate::media::audio_loss::OpusLossConcealment;
use crate::media::dtmf::{DecodedDtmfEvent, DtmfDecoder, TelephoneEventCodec};
use crate::media::outbound::OutboundAudioRtpWriter;

pub const FRAME_CHANNEL_CAP: usize = 64;

/// Default for `WebRtcConfig::inbound_send_deadline_ms`: how long the inbound
/// pump waits for room in a full inbound frame queue before it drops the
/// arriving frame. It is kept below one 10 ms packet period so that a stalled
/// consumer never slows the RTP reader below the wire rate.
pub const DEFAULT_INBOUND_SEND_DEADLINE_MS: u64 = 5;

/// Default for `WebRtcConfig::inbound_frame_queue_capacity`: frames held for
/// a consumer that is not reading (160 ms of 20 ms audio).
pub const DEFAULT_INBOUND_FRAME_QUEUE_CAPACITY: usize = 8;

/// Default Opus payload type registered in the media engine.
pub const OPUS_PT_DEFAULT: u8 = 111;

pub(crate) struct MediaTaskGuard(Option<Arc<AtomicUsize>>);

impl MediaTaskGuard {
    pub(crate) fn new(counter: Option<Arc<AtomicUsize>>) -> Self {
        if let Some(counter) = &counter {
            counter.fetch_add(1, Ordering::AcqRel);
        }
        Self(counter)
    }
}

impl Drop for MediaTaskGuard {
    fn drop(&mut self) {
        if let Some(counter) = &self.0 {
            let previous = counter.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0, "WebRTC media task counter underflow");
        }
    }
}

/// Inbound RTP statistics for [`QualitySnapshot`].
#[derive(Default)]
pub struct InboundStats {
    packets: AtomicU64,
    /// Last non-DTMF wire RTP payload type plus one (`0` means unseen).
    /// Keeping this separate from the negotiated descriptor lets
    /// qualification and diagnostics prove what the remote sender actually
    /// placed on the wire.
    last_media_payload_type_plus_one: AtomicU64,
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
    /// consumer was too slow (see `WebRtcConfig::inbound_frame_queue_capacity`
    /// and `WebRtcConfig::inbound_send_deadline_ms`).
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

    fn record_media_payload_type(&self, payload_type: u8) {
        self.last_media_payload_type_plus_one
            .store(u64::from(payload_type) + 1, Ordering::Release);
    }

    pub fn last_media_payload_type(&self) -> Option<u8> {
        self.last_media_payload_type_plus_one
            .load(Ordering::Acquire)
            .checked_sub(1)
            .and_then(|value| u8::try_from(value).ok())
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
                    let candidate_type = candidate_type_label(&c.candidate_type);
                    local_types.insert(c.stats.id.clone(), candidate_type.clone());
                    if let Some(candidate_id) = c.stats.id.strip_prefix("RTCLocalIceCandidate_") {
                        local_types.insert(candidate_id.to_owned(), candidate_type);
                    }
                }
                RTCStatsReportEntry::RemoteCandidate(c) => {
                    let candidate_type = candidate_type_label(&c.candidate_type);
                    remote_types.insert(c.stats.id.clone(), candidate_type.clone());
                    if let Some(candidate_id) = c.stats.id.strip_prefix("RTCRemoteIceCandidate_") {
                        remote_types.insert(candidate_id.to_owned(), candidate_type);
                    }
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
            ..Default::default()
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
/// `send_deadline_ms`: bounded send timeout. When `frames_in` is full the
/// pump waits at most this long for room, then drops the arriving frame and
/// bumps `InboundStats::frames_dropped`, so a slow downstream consumer
/// cannot stall the inbound RTP task.
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
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
) -> JoinHandle<()> {
    spawn_inbound_pump_tracked(
        track,
        stream_id,
        frames_in_tx,
        stats,
        send_deadline_ms,
        cancel,
        dtmf_tx,
        vec![TelephoneEventCodec::default()],
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_inbound_pump_tracked(
    track: Arc<dyn TrackRemote>,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    stats: Arc<InboundStats>,
    send_deadline_ms: u64,
    cancel: Option<Arc<Notify>>,
    dtmf_tx: Option<mpsc::Sender<DecodedDtmfEvent>>,
    dtmf_codecs: Vec<TelephoneEventCodec>,
    task_counter: Option<Arc<AtomicUsize>>,
) -> JoinHandle<()> {
    let task_guard = MediaTaskGuard::new(task_counter);
    tokio::spawn(async move {
        let _task_guard = task_guard;
        let deadline = Duration::from_millis(send_deadline_ms);
        let mut dtmf_decoder = DtmfDecoder::new(dtmf_codecs);
        let mut loss_concealment = OpusLossConcealment::default();
        // (SSRC, payload type, negotiated Opus) of the last audio source.
        let mut audio_source: Option<(u32, u8, bool)> = None;
        'receive: loop {
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
                    if dtmf_decoder.accepts_payload_type(pkt.header.payload_type) {
                        loss_concealment.reset();
                        tracing::trace!(
                            payload_type = pkt.header.payload_type,
                            timestamp = pkt.header.timestamp,
                            "received negotiated WebRTC telephone-event RTP"
                        );
                        if let Some(event) = dtmf_decoder.decode_packet(
                            pkt.header.timestamp,
                            pkt.header.payload_type,
                            &pkt.payload,
                        ) {
                            tracing::debug!(
                                duration_ms = event.duration_ms,
                                "decoded completed WebRTC telephone-event"
                            );
                            if let Some(tx) = &dtmf_tx {
                                let _ = tx.send(event).await;
                            }
                        }
                        continue;
                    }
                    stats.record_media_payload_type(pkt.header.payload_type);
                    let (ssrc, payload_type) = (pkt.header.ssrc, pkt.header.payload_type);
                    let negotiated_opus = match audio_source {
                        Some((known_ssrc, known_pt, opus))
                            if (known_ssrc, known_pt) == (ssrc, payload_type) =>
                        {
                            opus
                        }
                        _ => {
                            let codec = track.codec(ssrc).await;
                            let opus = codec.as_ref().is_some_and(|codec| {
                                codec.mime_type.eq_ignore_ascii_case("audio/opus")
                                    && codec.clock_rate == 48_000
                            });
                            // Look the codec up again until the track knows it.
                            audio_source = codec.map(|_| (ssrc, payload_type, opus));
                            opus
                        }
                    };
                    // Short, sequence-confirmed Opus loss is forwarded as
                    // lost-frame indications ahead of this packet so the
                    // decoder conceals the missing time (see `audio_loss`).
                    let admission = loss_concealment.admit(&pkt, negotiated_opus);
                    if admission.discard {
                        continue;
                    }
                    let concealed = admission
                        .concealment
                        .into_iter()
                        .flat_map(|run| run.frames());
                    let received = std::iter::once((pkt.header.timestamp, pkt.payload.clone()));
                    for (timestamp_rtp, payload) in concealed.chain(received) {
                        // D4 follow-up — emit codec payload bytes only (no RTP
                        // header). The orchestrator's `Transcoder` consumes
                        // codec bytes; the local outbound pump re-wraps with
                        // a fresh header. Preserve the timestamp in the
                        // `MediaFrame.timestamp_rtp` field so it survives the
                        // transcode pass.
                        let frame = MediaFrame {
                            stream_id: stream_id.clone(),
                            kind: rvoip_core::stream::StreamKind::Audio,
                            payload,
                            timestamp_rtp,
                            captured_at: Utc::now(),
                            // Gap plan §4.3 — carry the wire RTP payload-type
                            // so the cross-transport `frame_pump` can route
                            // RFC 4733 telephone-events (PT 101 by convention)
                            // distinctly from audio.
                            payload_type: Some(payload_type),
                        };
                        match tokio::time::timeout(deadline, frames_in_tx.send(frame)).await {
                            Ok(Ok(())) => {}
                            Ok(Err(_)) => break 'receive, // downstream dropped — stop pumping
                            Err(_) => {
                                // Slow consumer; drop the frame and count it.
                                let dropped =
                                    stats.frames_dropped.fetch_add(1, Ordering::Relaxed) + 1;
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
/// otherwise the compatibility entry point derives 20 ms timestamps using
/// WebRTC's 48 kHz Opus clock. Use [`spawn_outbound_pump_with_clock_rate`]
/// for another negotiated clock.
///
pub fn spawn_outbound_pump(
    track: Arc<TrackLocalStaticRTP>,
    frames_out_rx: mpsc::Receiver<MediaFrame>,
    ssrc: u32,
    payload_type: u8,
) -> JoinHandle<()> {
    spawn_outbound_pump_with_clock_rate(track, frames_out_rx, ssrc, payload_type, 48_000)
}

/// Spawn an outbound RTP pump using the exact negotiated RTP clock.
///
/// Writers for the same exact local track and SSRC share sequence/timestamp
/// ownership, including with peer-level RFC 4733 sends.
pub fn spawn_outbound_pump_with_clock_rate(
    track: Arc<TrackLocalStaticRTP>,
    frames_out_rx: mpsc::Receiver<MediaFrame>,
    ssrc: u32,
    payload_type: u8,
    clock_rate_hz: u32,
) -> JoinHandle<()> {
    spawn_outbound_pump_with_writer_tracked(
        OutboundAudioRtpWriter::new(track, ssrc, clock_rate_hz),
        frames_out_rx,
        payload_type,
        None,
    )
}

pub(crate) fn spawn_outbound_pump_with_writer_tracked(
    writer: Arc<OutboundAudioRtpWriter>,
    mut frames_out_rx: mpsc::Receiver<MediaFrame>,
    payload_type: u8,
    task_counter: Option<Arc<AtomicUsize>>,
) -> JoinHandle<()> {
    let task_guard = MediaTaskGuard::new(task_counter);
    tokio::spawn(async move {
        let _task_guard = task_guard;
        while let Some(frame) = frames_out_rx.recv().await {
            // MediaFrame payload is always codec payload. RTP wire images are
            // created only at this transport boundary. The route-owned writer
            // serializes this packet with same-SSRC telephone events so their
            // sequence and timestamp timelines cannot race or reorder.
            if let Err(error) = writer
                .write_audio(payload_type, frame.timestamp_rtp, frame.payload)
                .await
            {
                warn!(
                    %error,
                    payload_type,
                    "outbound WebRTC media pump stopped after a track write failure"
                );
                return;
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

/// Build a minimal silent Opus RTP packet for loopback tests.
pub fn silent_rtp_payload(seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet(seq, timestamp))
}

/// Marshalled silent Opus RTP with an explicit SSRC.
pub fn silent_rtp_payload_for_ssrc(ssrc: u32, seq: u16, timestamp: u32) -> bytes::Bytes {
    rtp_packet_to_bytes(&silent_rtp_packet_for_ssrc(ssrc, seq, timestamp))
}

/// Canonical codec-payload-only Opus silence used by `MediaFrame` fixtures.
/// RTP headers are added by the WebRTC outbound pump.
pub fn silent_opus_payload() -> bytes::Bytes {
    bytes::Bytes::from_static(&[0xF8, 0xFF, 0xFE])
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

    /// This adversarial codec payload happens to parse as RTP. It remains
    /// codec payload because the outbound pump never attempts that parse.
    #[test]
    fn rtp_looking_codec_payload_stays_opaque() {
        let raw_codec = bytes::Bytes::from_static(&[
            0x80,
            OPUS_PT_DEFAULT,
            0,
            1,
            0,
            0,
            3,
            0,
            1,
            2,
            3,
            4,
            0xF8,
            0xFF,
            0xFE,
        ]);
        let mut wire_like = raw_codec.clone();
        assert!(rtp::Packet::unmarshal(&mut wire_like).is_ok());
        assert_eq!(&raw_codec[12..], &[0xF8, 0xFF, 0xFE]);
    }

    mod loss_concealment {
        use super::*;
        use async_trait::async_trait;
        use rtc::media_stream::{
            MediaStreamId, MediaStreamTrackId, MediaStreamTrackState, MediaTrackCapabilities,
            MediaTrackConstraints, MediaTrackSettings,
        };
        use rtc::rtp_transceiver::rtp_sender::{
            RTCRtpCodec, RTCRtpCodingParameters, RTCRtpEncodingParameters, RtpCodecKind,
        };
        use rtc::rtp_transceiver::RtpStreamId;
        use webrtc::media_stream::{MediaStreamTrack, Track};

        /// Remote track fed by the test. Like the peer-connection driver, the
        /// producer `try_send`s into a bounded event queue and loses packets
        /// when the inbound pump does not keep up.
        struct ScriptedRemoteTrack {
            track: parking_lot::Mutex<MediaStreamTrack>,
            events: tokio::sync::Mutex<mpsc::Receiver<TrackRemoteEvent>>,
        }

        impl ScriptedRemoteTrack {
            fn opus(ssrc: u32, capacity: usize) -> (Arc<Self>, mpsc::Sender<TrackRemoteEvent>) {
                let (tx, rx) = mpsc::channel(capacity);
                let track = MediaStreamTrack::new(
                    "remote-stream".into(),
                    "remote-audio".into(),
                    "remote-audio".into(),
                    RtpCodecKind::Audio,
                    vec![RTCRtpEncodingParameters {
                        rtp_coding_parameters: RTCRtpCodingParameters {
                            ssrc: Some(ssrc),
                            ..Default::default()
                        },
                        codec: RTCRtpCodec {
                            mime_type: "audio/opus".into(),
                            clock_rate: 48_000,
                            channels: 2,
                            ..Default::default()
                        },
                        ..Default::default()
                    }],
                );
                let remote = Arc::new(Self {
                    track: parking_lot::Mutex::new(track),
                    events: tokio::sync::Mutex::new(rx),
                });
                (remote, tx)
            }
        }

        #[async_trait]
        impl Track for ScriptedRemoteTrack {
            async fn stream_id(&self) -> MediaStreamId {
                self.track.lock().stream_id().to_owned()
            }
            async fn track_id(&self) -> MediaStreamTrackId {
                self.track.lock().track_id().to_owned()
            }
            async fn label(&self) -> String {
                self.track.lock().label().to_owned()
            }
            async fn kind(&self) -> RtpCodecKind {
                self.track.lock().kind()
            }
            async fn rid(&self, ssrc: u32) -> Option<RtpStreamId> {
                self.track.lock().rid(ssrc).cloned()
            }
            async fn codec(&self, ssrc: u32) -> Option<RTCRtpCodec> {
                self.track.lock().codec(ssrc).cloned()
            }
            async fn ssrcs(&self) -> Vec<u32> {
                self.track.lock().ssrcs().collect()
            }
            async fn enabled(&self) -> bool {
                self.track.lock().enabled()
            }
            async fn set_enabled(&self, enabled: bool) {
                self.track.lock().set_enabled(enabled);
            }
            async fn muted(&self) -> bool {
                self.track.lock().muted()
            }
            async fn set_muted(&self, muted: bool) {
                self.track.lock().set_muted(muted);
            }
            async fn ready_state(&self) -> MediaStreamTrackState {
                self.track.lock().ready_state()
            }
            async fn stop(&self) {
                self.track.lock().stop();
            }
            async fn get_capabilities(&self) -> MediaTrackCapabilities {
                self.track.lock().get_capabilities().clone()
            }
            async fn get_constraints(&self) -> MediaTrackConstraints {
                self.track.lock().get_constraints().clone()
            }
            async fn get_settings(&self) -> MediaTrackSettings {
                self.track.lock().get_settings().clone()
            }
            async fn apply_constraints(&self, constraints: Option<MediaTrackConstraints>) {
                self.track.lock().apply_constraints(constraints);
            }
            async fn codings(&self) -> Vec<RTCRtpEncodingParameters> {
                self.track.lock().codings().to_vec()
            }
            async fn add_coding(&self, coding: RTCRtpEncodingParameters) {
                self.track.lock().add_coding(coding);
            }
        }

        #[async_trait]
        impl TrackRemote for ScriptedRemoteTrack {
            async fn write_rtcp(
                &self,
                _packets: Vec<Box<dyn rtc::rtcp::Packet>>,
            ) -> webrtc::error::Result<()> {
                Ok(())
            }

            async fn poll(&self) -> Option<TrackRemoteEvent> {
                self.events.lock().await.recv().await
            }
        }

        const REMOTE_SSRC: u32 = 0x0a0b_0c0d;

        #[tokio::test]
        async fn short_opus_gap_is_concealed_before_the_next_frame() {
            let (remote, events) = ScriptedRemoteTrack::opus(REMOTE_SSRC, 64);
            let (frames_tx, mut frames_rx) = mpsc::channel(64);
            let pump = spawn_inbound_pump(
                remote,
                StreamId::new(),
                frames_tx,
                Arc::new(InboundStats::default()),
                DEFAULT_INBOUND_SEND_DEADLINE_MS,
                None,
                None,
            );
            // 102 and 103 are lost and 102 then arrives late; 105..=110
            // (six packets) are lost.
            for (sequence, frame) in [(100u16, 0u32), (101, 1), (104, 4), (102, 2), (111, 11)] {
                events
                    .send(TrackRemoteEvent::OnRtpPacket(silent_rtp_packet_for_ssrc(
                        REMOTE_SSRC,
                        sequence,
                        frame * 960,
                    )))
                    .await
                    .unwrap();
            }
            events.send(TrackRemoteEvent::OnEnded).await.unwrap();

            let mut received = Vec::new();
            while let Some(frame) = frames_rx.recv().await {
                received.push((frame.timestamp_rtp / 960, frame.payload.len()));
            }
            pump.await.unwrap();
            // Silent Opus frames are three bytes; a lost-frame indication is
            // the one-byte TOC.
            assert_eq!(
                received,
                vec![(0, 3), (1, 3), (2, 1), (3, 1), (4, 3), (11, 3)],
                "(frame, payload bytes) forwarded by the inbound pump"
            );
        }
    }
}
