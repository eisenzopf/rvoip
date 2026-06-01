//! D4 — `MediaStream` impl for SIP sessions, the wrapper that closes the
//! `SipAdapter::streams()` gap.
//!
//! Wraps the existing PCM-level audio API ([`UnifiedCoordinator::subscribe_to_audio`]
//! / [`UnifiedCoordinator::send_audio`]) so the orchestrator-level
//! [`Orchestrator::bridge_connections`](rvoip_core::orchestrator::Orchestrator::bridge_connections)
//! can talk to the SIP leg in the same vocabulary it uses for WebRTC:
//! `MediaFrame { payload: Bytes }` channels driven by `frames_in()` /
//! `frames_out()`.
//!
//! **Payload contract — important.** The WebRTC adapter today places the
//! full RTP wire image into `MediaFrame.payload` (see the inbound pump in
//! `crates/webrtc/rvoip-webrtc/src/media/pump.rs`). The orchestrator's
//! `Transcoder` (see `crates/media/media-core/src/codec/transcoding.rs`)
//! expects **codec payload bytes** (no RTP header). The SIP side here
//! emits codec payload bytes (G.711 μ-law) — the shape the transcoder
//! consumes. End-to-end audio bridging from a SIP UA through the
//! orchestrator to a WebRTC peer still requires aligning the WebRTC side
//! to the same convention; tracking that work under follow-up
//! `GAP_PLAN.md` §3.1 D4 follow-on (the contract reconciliation is a
//! separate ~3-day refactor of `pump.rs`).

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use std::sync::Mutex;
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinHandle;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::Result as RvoipResult;
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};

use crate::api::unified::UnifiedCoordinator;
use crate::SessionId;

use rvoip_media_core::codec::audio::common::AudioCodec;
use rvoip_media_core::codec::audio::g711::G711Codec;

/// SIP G.711 PCMU sample rate (8 kHz / 20 ms / 160 samples per frame).
const G711_SAMPLE_RATE: u32 = 8_000;
const G711_FRAME_SAMPLES: usize = 160; // 20 ms @ 8 kHz mono

/// Frame channel depth. Same default as `rvoip-webrtc` (see
/// `crates/webrtc/rvoip-webrtc/src/media/pump.rs::FRAME_CHANNEL_CAP`).
const FRAME_CHANNEL_CAP: usize = 64;

/// One-take wrapper for the inbound `MediaFrame` receiver — mirrors the
/// `WebRtcMediaStream` shape so consumers calling `frames_in()` twice get
/// a closed channel on the second call instead of a panic.
struct SipMediaStreamInner {
    stream_id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    frames_in_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    frames_out_tx: mpsc::Sender<MediaFrame>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    cancel: Arc<Notify>,
}

/// Concrete `MediaStream` for the SIP transport. Built lazily by
/// `SipAdapter::streams()` the first time a
/// caller asks for streams on a connection that has an active audio
/// session.
pub struct SipMediaStream {
    inner: Arc<SipMediaStreamInner>,
}

impl SipMediaStream {
    /// Build a new stream backed by an active SIP session. Spawns two
    /// background tasks (inbound encoder + outbound decoder) that pump
    /// frames between the orchestrator-facing channels and the
    /// `UnifiedCoordinator`'s PCM audio API.
    pub async fn new(
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
        direction: Direction,
    ) -> crate::errors::Result<Arc<Self>> {
        let stream_id = StreamId::new();
        let codec = CodecInfo {
            // D4 follow-up — matches `rvoip_core::bridge::codec_to_pt` so the
            // orchestrator's bridge maps this stream to PT 0 (PCMU) for the
            // transcoder. Renamed from "g.711-mulaw" which the bridge
            // didn't recognize.
            name: "g.711-mu".to_string(),
            clock_rate_hz: G711_SAMPLE_RATE,
            channels: 1,
            fmtp: None,
        };
        let (frames_in_tx, frames_in_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let (frames_out_tx, mut frames_out_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let cancel = Arc::new(Notify::new());

        // Inbound: decoded PCM AudioFrame from SIP → G.711 encode → MediaFrame.
        let mut subscriber = coordinator
            .subscribe_to_audio(&session_id)
            .await
            .map_err(|e| crate::errors::SessionError::Other(format!("subscribe_to_audio: {e}")))?;
        let stream_id_in = stream_id.clone();
        let cancel_in = Arc::clone(&cancel);
        let frames_in_tx_pump = frames_in_tx.clone();
        let inbound_handle = tokio::spawn(async move {
            let mut encoder = match G711Codec::mu_law(G711_SAMPLE_RATE, 1) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(target: "rvoip_sip", error = %e, "SipMediaStream: G.711 mu_law encoder init failed");
                    return;
                }
            };
            loop {
                tokio::select! {
                    _ = cancel_in.notified() => break,
                    frame = subscriber.receiver.recv() => {
                        let Some(audio_frame) = frame else { break; };
                        let encoded = match encoder.encode(&audio_frame) {
                            Ok(bytes) => bytes,
                            Err(e) => {
                                tracing::trace!(target: "rvoip_sip", error = %e, "SipMediaStream: G.711 encode failed");
                                continue;
                            }
                        };
                        let media_frame = MediaFrame {
                            stream_id: stream_id_in.clone(),
                            kind: StreamKind::Audio,
                            payload: Bytes::from(encoded),
                            timestamp_rtp: audio_frame.timestamp,
                            captured_at: Utc::now(),
                            // Gap plan §4.3 — SIP `SipMediaStream` always
                            // emits G.711 mu-law (PCMU = PT 0). DTMF
                            // (RFC 4733, PT 101) arrives via a separate
                            // callback in the underlying media_adapter
                            // and never flows through this audio path,
                            // so PCMU is correct for every frame here.
                            payload_type: Some(0),
                        };
                        if frames_in_tx_pump.send(media_frame).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Outbound: MediaFrame from orchestrator → G.711 decode → AudioFrame
        // sent into the SIP session's audio path.
        let coordinator_out = Arc::clone(&coordinator);
        let session_id_out = session_id.clone();
        let cancel_out = Arc::clone(&cancel);
        let outbound_handle = tokio::spawn(async move {
            let mut decoder = match G711Codec::mu_law(G711_SAMPLE_RATE, 1) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(target: "rvoip_sip", error = %e, "SipMediaStream: G.711 mu_law decoder init failed");
                    return;
                }
            };
            let mut next_timestamp: u32 = 0;
            loop {
                tokio::select! {
                    _ = cancel_out.notified() => break,
                    frame = frames_out_rx.recv() => {
                        let Some(media_frame) = frame else { break; };
                        // Gap plan §4.3 — RFC 4733 telephone-event
                        // routing. When a cross-substrate bridge
                        // forwards a frame labelled with the
                        // telephone-event PT (101 by convention),
                        // route it through the SIP session's DTMF
                        // emitter rather than the audio decoder.
                        // The 4-byte payload encodes (event, end+r+vol,
                        // duration) per RFC 4733 §2.3; we parse the
                        // event byte and emit the corresponding DTMF
                        // digit on the start packet (end=0). The same
                        // digit retransmitted with end=1 is treated as
                        // a duplicate and skipped.
                        const TELEPHONE_EVENT_PT: u8 = 101;
                        if media_frame.payload_type == Some(TELEPHONE_EVENT_PT) {
                            if let Some(digit) = parse_rfc4733_digit(&media_frame.payload) {
                                if let Err(e) =
                                    coordinator_out.send_dtmf(&session_id_out, digit).await
                                {
                                    tracing::trace!(target: "rvoip_sip", error = %e, "SipMediaStream: send_dtmf failed");
                                }
                            }
                            continue;
                        }
                        // Skip frames that don't look like G.711 codec payload.
                        // A 20 ms G.711 mono frame is exactly 160 bytes; the
                        // transcoder upstream may have produced shorter
                        // payloads on partial frames — pass them through
                        // best-effort.
                        let mut audio_frame = match decoder.decode(&media_frame.payload) {
                            Ok(f) => f,
                            Err(e) => {
                                tracing::trace!(target: "rvoip_sip", error = %e, bytes = media_frame.payload.len(), "SipMediaStream: G.711 decode failed; dropping frame");
                                continue;
                            }
                        };
                        // Carry the upstream RTP timestamp when present;
                        // otherwise advance our own monotonic clock.
                        audio_frame.timestamp = if media_frame.timestamp_rtp != 0 {
                            media_frame.timestamp_rtp
                        } else {
                            next_timestamp
                        };
                        next_timestamp = audio_frame
                            .timestamp
                            .wrapping_add(G711_FRAME_SAMPLES as u32);
                        if let Err(e) = coordinator_out.send_audio(&session_id_out, audio_frame).await {
                            tracing::trace!(target: "rvoip_sip", error = %e, "SipMediaStream: send_audio failed");
                            // Don't break — the session may briefly be in
                            // a renegotiation window; retry the next frame.
                        }
                    }
                }
            }
        });

        // `frames_in_tx` only lives inside the inbound pump now.
        drop(frames_in_tx);

        Ok(Arc::new(Self {
            inner: Arc::new(SipMediaStreamInner {
                stream_id,
                codec,
                direction,
                frames_in_rx: Mutex::new(Some(frames_in_rx)),
                frames_out_tx,
                pumps: Mutex::new(vec![inbound_handle, outbound_handle]),
                cancel,
            }),
        }))
    }
}

#[async_trait]
impl MediaStream for SipMediaStream {
    fn id(&self) -> StreamId {
        self.inner.stream_id.clone()
    }

    fn kind(&self) -> StreamKind {
        StreamKind::Audio
    }

    fn codec(&self) -> CodecInfo {
        self.inner.codec.clone()
    }

    fn direction(&self) -> Direction {
        self.inner.direction
    }

    fn frames_in(&self) -> mpsc::Receiver<MediaFrame> {
        // Single-take per the `MediaStream` trait contract.
        self.inner
            .frames_in_rx
            .lock()
            .ok()
            .and_then(|mut g| g.take())
            .unwrap_or_else(|| mpsc::channel(1).1)
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.inner.frames_out_tx.clone()
    }

    fn quality_snapshot(&self) -> QualitySnapshot {
        // No per-session stats yet — return defaults. Wiring real loss /
        // jitter metrics from the SIP RTP layer is tracked alongside the
        // wider observability gap (`GAP_PLAN.md` §2.6 Per-pair RTT).
        QualitySnapshot::default()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.inner.cancel.notify_waiters();
        if let Ok(mut guard) = self.inner.pumps.lock() {
            for handle in guard.drain(..) {
                handle.abort();
            }
        }
        Ok(())
    }
}

/// Parse an RFC 4733 `telephone-event` payload (4 bytes) into a digit
/// character, but only on the **start** packet of an event (duration
/// field is zero). Returns `None` for retransmits (duration > 0) and
/// for malformed payloads so the caller can skip without double-
/// emitting the same DTMF.
///
/// Payload layout (§2.3 of RFC 4733):
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |     event     |E|R| volume    |          duration             |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
fn parse_rfc4733_digit(payload: &[u8]) -> Option<char> {
    if payload.len() < 4 {
        return None;
    }
    let event = payload[0];
    let duration = u16::from_be_bytes([payload[2], payload[3]]);
    if duration != 0 {
        // Retransmit / end-marker — already emitted on the start packet.
        return None;
    }
    // Event codes 0–9 → '0'..'9', 10 → '*', 11 → '#', 12–15 → 'A'..'D'.
    match event {
        0..=9 => Some((b'0' + event) as char),
        10 => Some('*'),
        11 => Some('#'),
        12 => Some('A'),
        13 => Some('B'),
        14 => Some('C'),
        15 => Some('D'),
        _ => None,
    }
}

#[cfg(test)]
mod rfc4733_tests {
    use super::parse_rfc4733_digit;

    #[test]
    fn start_packet_returns_digit() {
        // event=5, end=0, volume=10, duration=0
        let packet = [0x05, 0x0A, 0x00, 0x00];
        assert_eq!(parse_rfc4733_digit(&packet), Some('5'));
    }

    #[test]
    fn duration_nonzero_returns_none_to_avoid_duplicates() {
        // event=5, end=0, volume=10, duration=160
        let packet = [0x05, 0x0A, 0x00, 0xA0];
        assert_eq!(parse_rfc4733_digit(&packet), None);
    }

    #[test]
    fn star_hash_letters_map_correctly() {
        assert_eq!(parse_rfc4733_digit(&[10, 0, 0, 0]), Some('*'));
        assert_eq!(parse_rfc4733_digit(&[11, 0, 0, 0]), Some('#'));
        assert_eq!(parse_rfc4733_digit(&[12, 0, 0, 0]), Some('A'));
        assert_eq!(parse_rfc4733_digit(&[15, 0, 0, 0]), Some('D'));
    }

    #[test]
    fn unknown_events_return_none() {
        assert_eq!(parse_rfc4733_digit(&[99, 0, 0, 0]), None);
        assert_eq!(parse_rfc4733_digit(&[0xFF, 0, 0, 0]), None);
    }

    #[test]
    fn short_payload_returns_none() {
        assert_eq!(parse_rfc4733_digit(&[5, 0, 0]), None);
        assert_eq!(parse_rfc4733_digit(&[]), None);
    }
}
