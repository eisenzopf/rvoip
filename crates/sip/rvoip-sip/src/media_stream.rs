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

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use std::sync::Mutex;
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex};
use tokio::task::AbortHandle;

use rvoip_core::capability::CodecInfo;
use rvoip_core::connection::Direction;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::ids::StreamId;
use rvoip_core::stream::{MediaFrame, MediaStream, QualitySnapshot, StreamKind};

use crate::api::unified::UnifiedCoordinator;
use crate::SessionId;

use rvoip_media_core::codec::audio::common::AudioCodec;
use rvoip_media_core::codec::audio::g711::G711Codec;

/// SIP G.711 PCMU sample rate (8 kHz / 20 ms / 160 samples per frame).
const G711_SAMPLE_RATE: u32 = 8_000;

/// Frame channel depth. Same default as `rvoip-webrtc` (see
/// `crates/webrtc/rvoip-webrtc/src/media/pump.rs::FRAME_CHANNEL_CAP`).
const FRAME_CHANNEL_CAP: usize = 64;

/// Next outbound RTP timestamp for the G.711 (8 kHz) leg.
///
/// RFC 3550: the RTP timestamp is expressed in the *destination* payload
/// format's clock and counts samples emitted. The upstream/source RTP
/// timestamp (`_upstream_rtp_ts`) is **deliberately ignored**: when the source
/// leg runs on a different clock — e.g. Amazon Connect Opus at 48 kHz, which
/// advances +960 per 20 ms — stamping that value onto the 8 kHz G.711 leg makes
/// the timestamp climb 6× too fast (960 vs 160) and the caller's jitter buffer
/// reads ~100 ms of false jitter (fast, regular clicking). Mature transcoders
/// (Asterisk `lastts += samples`, FreeSWITCH, rtpengine) always regenerate the
/// timestamp on the destination clock; we do the same, advancing by the number
/// of samples actually emitted so partial frames stay correct.
fn advance_outbound_timestamp(
    clock: &mut u32,
    samples_emitted: usize,
    _upstream_rtp_ts: u32,
) -> u32 {
    let ts = *clock;
    *clock = clock.wrapping_add(samples_emitted as u32);
    ts
}

/// One-take wrapper for the inbound `MediaFrame` receiver — mirrors the
/// `WebRtcMediaStream` shape so consumers calling `frames_in()` twice get
/// a closed channel on the second call instead of a panic.
struct SipMediaStreamInner {
    stream_id: StreamId,
    codec: CodecInfo,
    direction: Direction,
    frames_in_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    frames_in_tx: Mutex<Option<mpsc::Sender<MediaFrame>>>,
    frames_out_tx: mpsc::Sender<MediaFrame>,
    frames_out_rx: Mutex<Option<mpsc::Receiver<MediaFrame>>>,
    bind_target: Mutex<Option<SipMediaBindTarget>>,
    driver_abort: Mutex<Option<AbortHandle>>,
    lifecycle_gate: AsyncMutex<()>,
    lifecycle: Arc<SipMediaLifecycleState>,
    cancel: watch::Sender<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SipMediaLifecycle {
    Dormant,
    Binding,
    Bound,
    Closing,
    Closed,
    Failed,
}

struct SipMediaBindTarget {
    coordinator: Weak<UnifiedCoordinator>,
    session_id: SessionId,
}

impl SipMediaBindTarget {
    fn matches(&self, coordinator: &Arc<UnifiedCoordinator>, session_id: &SessionId) -> bool {
        self.session_id == *session_id && self.coordinator.ptr_eq(&Arc::downgrade(coordinator))
    }
}

struct SipMediaLifecycleState {
    state: Mutex<SipMediaLifecycle>,
    updates: watch::Sender<SipMediaLifecycle>,
}

impl SipMediaLifecycleState {
    fn new() -> Self {
        let (updates, _) = watch::channel(SipMediaLifecycle::Dormant);
        Self {
            state: Mutex::new(SipMediaLifecycle::Dormant),
            updates,
        }
    }

    fn current(&self) -> SipMediaLifecycle {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn subscribe(&self) -> watch::Receiver<SipMediaLifecycle> {
        self.updates.subscribe()
    }

    fn transition(
        &self,
        allowed: impl FnOnce(SipMediaLifecycle) -> bool,
        next: SipMediaLifecycle,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !allowed(*state) {
            return false;
        }
        *state = next;
        self.updates.send_replace(next);
        true
    }

    fn begin_binding(&self) -> bool {
        self.transition(
            |state| state == SipMediaLifecycle::Dormant,
            SipMediaLifecycle::Binding,
        )
    }

    fn mark_bound(&self) -> bool {
        self.transition(
            |state| state == SipMediaLifecycle::Binding,
            SipMediaLifecycle::Bound,
        )
    }

    fn mark_failed(&self) -> bool {
        self.transition(
            |state| {
                matches!(
                    state,
                    SipMediaLifecycle::Dormant
                        | SipMediaLifecycle::Binding
                        | SipMediaLifecycle::Bound
                )
            },
            SipMediaLifecycle::Failed,
        )
    }

    fn begin_closing(&self) -> bool {
        self.transition(
            |state| {
                !matches!(
                    state,
                    SipMediaLifecycle::Closing | SipMediaLifecycle::Closed
                )
            },
            SipMediaLifecycle::Closing,
        )
    }

    fn mark_closed(&self) {
        self.transition(
            |state| state != SipMediaLifecycle::Closed,
            SipMediaLifecycle::Closed,
        );
    }
}

/// Concrete `MediaStream` for the SIP transport.
///
/// The adapter allocates it in a dormant, local-only state before exposing a
/// connection. Binding to coordinator audio is retained, single-flight work
/// that happens only when the corresponding signaling route is activated.
pub struct SipMediaStream {
    inner: Arc<SipMediaStreamInner>,
}

impl SipMediaStream {
    /// Allocate a local-only media stream without touching a SIP session.
    ///
    /// This constructor allocates only bounded channels and a stable stream
    /// identifier. It does not subscribe to coordinator audio, create media,
    /// start a task, allocate a socket, or emit a packet. Outbound adapters use
    /// it while their connection is durably prepared but not yet activated.
    pub fn dormant(direction: Direction) -> Arc<Self> {
        let stream_id = StreamId::new();
        let codec = CodecInfo {
            name: "g.711-mu".to_string(),
            clock_rate_hz: G711_SAMPLE_RATE,
            channels: 1,
            fmtp: None,
        };
        let (frames_in_tx, frames_in_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let (frames_out_tx, frames_out_rx) = mpsc::channel::<MediaFrame>(FRAME_CHANNEL_CAP);
        let (cancel, _) = watch::channel(false);

        Arc::new(Self {
            inner: Arc::new(SipMediaStreamInner {
                stream_id,
                codec,
                direction,
                frames_in_rx: Mutex::new(Some(frames_in_rx)),
                frames_in_tx: Mutex::new(Some(frames_in_tx)),
                frames_out_tx,
                frames_out_rx: Mutex::new(Some(frames_out_rx)),
                bind_target: Mutex::new(None),
                driver_abort: Mutex::new(None),
                lifecycle_gate: AsyncMutex::new(()),
                lifecycle: Arc::new(SipMediaLifecycleState::new()),
                cancel,
            }),
        })
    }

    /// Build a stream backed by an active SIP session.
    ///
    /// Kept as the compatibility surface for inbound and legacy callers. The
    /// implementation is the same dormant allocation followed by one retained
    /// bind, so every path shares the same lifecycle and cleanup behavior.
    pub async fn new(
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
        direction: Direction,
    ) -> crate::errors::Result<Arc<Self>> {
        let stream = Self::dormant(direction);
        stream.bind(coordinator, session_id).await?;
        Ok(stream)
    }

    /// Bind this dormant stream to one SIP session exactly once.
    ///
    /// The first caller starts a retained driver. Dropping that caller does not
    /// cancel the driver, and concurrent callers observe the same terminal
    /// outcome without creating another subscription or another pump pair.
    pub async fn bind(
        self: &Arc<Self>,
        coordinator: Arc<UnifiedCoordinator>,
        session_id: SessionId,
    ) -> crate::errors::Result<()> {
        let mut lifecycle = self.inner.lifecycle.subscribe();
        {
            let _gate = self.inner.lifecycle_gate.lock().await;
            let state = self.inner.lifecycle.current();
            if state != SipMediaLifecycle::Dormant {
                let matches = self
                    .inner
                    .bind_target
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .as_ref()
                    .is_some_and(|target| target.matches(&coordinator, &session_id));
                if !matches {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is bound to a different coordinator or session"
                            .to_string(),
                    ));
                }
            }
            match state {
                SipMediaLifecycle::Dormant => {
                    *self
                        .inner
                        .bind_target
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(SipMediaBindTarget {
                            coordinator: Arc::downgrade(&coordinator),
                            session_id: session_id.clone(),
                        });
                    let frames_in_tx = self
                        .inner
                        .frames_in_tx
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let frames_out_rx = self
                        .inner
                        .frames_out_rx
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let (Some(frames_in_tx), Some(frames_out_rx)) = (frames_in_tx, frames_out_rx)
                    else {
                        self.inner.lifecycle.mark_failed();
                        return Err(crate::errors::SessionError::Other(
                            "SIP media channels are unavailable".to_string(),
                        ));
                    };
                    if !self.inner.lifecycle.begin_binding() {
                        return Err(crate::errors::SessionError::Other(
                            "SIP media lifecycle changed during bind".to_string(),
                        ));
                    }
                    let driver = tokio::spawn(run_media_driver(
                        Arc::clone(&self.inner.lifecycle),
                        self.inner.cancel.clone(),
                        self.inner.cancel.subscribe(),
                        coordinator,
                        session_id,
                        self.inner.stream_id.clone(),
                        frames_in_tx,
                        frames_out_rx,
                    ));
                    *self
                        .inner
                        .driver_abort
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                        Some(driver.abort_handle());
                    drop(driver);
                }
                SipMediaLifecycle::Binding | SipMediaLifecycle::Bound => {}
                SipMediaLifecycle::Failed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media bind failed".to_string(),
                    ));
                }
                SipMediaLifecycle::Closing | SipMediaLifecycle::Closed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is closed".to_string(),
                    ));
                }
            }
        }

        loop {
            match *lifecycle.borrow_and_update() {
                SipMediaLifecycle::Bound => return Ok(()),
                SipMediaLifecycle::Failed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media bind failed".to_string(),
                    ));
                }
                SipMediaLifecycle::Closing | SipMediaLifecycle::Closed => {
                    return Err(crate::errors::SessionError::Other(
                        "SIP media stream is closed".to_string(),
                    ));
                }
                SipMediaLifecycle::Dormant | SipMediaLifecycle::Binding => {}
            }
            if lifecycle.changed().await.is_err() {
                return Err(crate::errors::SessionError::Other(
                    "SIP media lifecycle ended".to_string(),
                ));
            }
        }
    }

    pub(crate) fn subscribe_lifecycle(&self) -> watch::Receiver<SipMediaLifecycle> {
        self.inner.lifecycle.subscribe()
    }

    pub(crate) fn is_bound_to(
        &self,
        coordinator: &Arc<UnifiedCoordinator>,
        session_id: &SessionId,
    ) -> bool {
        self.inner
            .bind_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .is_some_and(|target| target.matches(coordinator, session_id))
    }

    fn close_local_channels(&self) {
        self.inner
            .frames_in_tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        self.inner
            .frames_out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    /// Make cancellation sticky without requiring an async runtime join.
    ///
    /// Adapter teardown calls this before dropping its last registry handle so
    /// a retained bind waiting in coordinator media cannot form a task/stream
    /// cycle. [`MediaStream::close`] performs the subsequent bounded joins.
    pub(crate) fn request_close(&self) {
        self.inner.lifecycle.begin_closing();
        self.inner.cancel.send_replace(true);
        self.close_local_channels();
    }

    async fn close_retained(self: &Arc<Self>) {
        let _gate = self.inner.lifecycle_gate.lock().await;
        if self.inner.lifecycle.current() == SipMediaLifecycle::Closed {
            return;
        }
        self.request_close();

        let driver_abort = self
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(abort) = driver_abort {
            if tokio::time::timeout(std::time::Duration::from_secs(1), async {
                while !abort.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_err()
            {
                abort.abort();
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
                    while !abort.is_finished() {
                        tokio::task::yield_now().await;
                    }
                })
                .await;
            }
        }
        self.inner.lifecycle.mark_closed();
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_media_driver(
    lifecycle: Arc<SipMediaLifecycleState>,
    cancel_tx: watch::Sender<bool>,
    mut cancel_rx: watch::Receiver<bool>,
    coordinator: Arc<UnifiedCoordinator>,
    session_id: SessionId,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
    frames_out_rx: mpsc::Receiver<MediaFrame>,
) {
    let subscription = coordinator.subscribe_to_audio(&session_id);
    tokio::pin!(subscription);
    let subscriber = tokio::select! {
        _ = wait_for_media_cancel(&mut cancel_rx) => return,
        result = &mut subscription => match result {
            Ok(subscriber) => subscriber,
            Err(error) => {
                tracing::warn!(
                    target: "rvoip_sip",
                    error = %error,
                    "SipMediaStream audio subscription failed"
                );
                lifecycle.mark_failed();
                return;
            }
        }
    };

    if *cancel_tx.borrow() {
        return;
    }
    let encoder = match G711Codec::mu_law(G711_SAMPLE_RATE, 1) {
        Ok(codec) => codec,
        Err(error) => {
            tracing::warn!(target: "rvoip_sip", error = %error, "SipMediaStream encoder initialization failed");
            lifecycle.mark_failed();
            return;
        }
    };
    let decoder = match G711Codec::mu_law(G711_SAMPLE_RATE, 1) {
        Ok(codec) => codec,
        Err(error) => {
            tracing::warn!(target: "rvoip_sip", error = %error, "SipMediaStream decoder initialization failed");
            lifecycle.mark_failed();
            return;
        }
    };
    if !lifecycle.mark_bound() {
        return;
    }

    let inbound = run_inbound_pump(subscriber, encoder, stream_id, frames_in_tx);
    let outbound = run_outbound_pump(
        Arc::clone(&coordinator),
        session_id.clone(),
        decoder,
        frames_out_rx,
    );
    tokio::pin!(inbound, outbound);
    let failed_pump = tokio::select! {
        _ = wait_for_media_cancel(&mut cancel_rx) => None,
        failure = &mut inbound => Some(failure),
        failure = &mut outbound => Some(failure),
    };
    if let Some(failure) = failed_pump {
        tracing::warn!(target: "rvoip_sip", failure, "SipMediaStream pump stopped unexpectedly");
        if lifecycle.mark_failed() {
            cancel_tx.send_replace(true);
        }
    }
}

async fn run_inbound_pump(
    mut subscriber: crate::types::AudioFrameSubscriber,
    mut encoder: G711Codec,
    stream_id: StreamId,
    frames_in_tx: mpsc::Sender<MediaFrame>,
) -> &'static str {
    while let Some(audio_frame) = subscriber.receiver.recv().await {
        let encoded = match encoder.encode(&audio_frame) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::trace!(target: "rvoip_sip", error = %error, "SipMediaStream: G.711 encode failed");
                continue;
            }
        };
        let media_frame = MediaFrame {
            stream_id: stream_id.clone(),
            kind: StreamKind::Audio,
            payload: Bytes::from(encoded),
            timestamp_rtp: audio_frame.timestamp,
            captured_at: Utc::now(),
            payload_type: Some(0),
        };
        if frames_in_tx.send(media_frame).await.is_err() {
            return "inbound-consumer-closed";
        }
    }
    "sip-audio-source-closed"
}

async fn run_outbound_pump(
    coordinator: Arc<UnifiedCoordinator>,
    session_id: SessionId,
    mut decoder: G711Codec,
    mut frames_out_rx: mpsc::Receiver<MediaFrame>,
) -> &'static str {
    let mut next_timestamp = 0u32;
    while let Some(media_frame) = frames_out_rx.recv().await {
        const TELEPHONE_EVENT_PT: u8 = 101;
        if media_frame.payload_type == Some(TELEPHONE_EVENT_PT) {
            if let Some(digit) = parse_rfc4733_digit(&media_frame.payload) {
                if coordinator.send_dtmf(&session_id, digit).await.is_err() {
                    return "sip-dtmf-send-failed";
                }
            }
            continue;
        }
        let mut audio_frame = match decoder.decode(&media_frame.payload) {
            Ok(frame) => frame,
            Err(error) => {
                tracing::trace!(
                    target: "rvoip_sip",
                    error = %error,
                    bytes = media_frame.payload.len(),
                    "SipMediaStream: G.711 decode failed; dropping frame"
                );
                continue;
            }
        };
        let samples_emitted = audio_frame.samples.len();
        audio_frame.timestamp = advance_outbound_timestamp(
            &mut next_timestamp,
            samples_emitted,
            media_frame.timestamp_rtp,
        );
        if coordinator
            .send_audio(&session_id, audio_frame)
            .await
            .is_err()
        {
            return "sip-audio-send-failed";
        }
    }
    "outbound-producer-closed"
}

async fn wait_for_media_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
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
        self.try_frames_in().unwrap_or_else(|_| mpsc::channel(1).1)
    }

    fn try_frames_in(&self) -> RvoipResult<mpsc::Receiver<MediaFrame>> {
        self.inner
            .frames_in_rx
            .lock()
            .map_err(|_| RvoipError::InvalidState("SIP media receiver lock is poisoned"))?
            .take()
            .ok_or(RvoipError::InvalidState(
                "SIP media receiver has already been acquired",
            ))
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
        self.close_retained().await;
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

#[cfg(test)]
mod outbound_timestamp_tests {
    use super::advance_outbound_timestamp;

    /// A full 20 ms G.711 frame at 8 kHz mono.
    const G711_FRAME_SAMPLES: usize = 160;

    /// Regression: the outbound G.711 timestamp must run on its own 8 kHz clock
    /// (+160 per 20 ms frame) and ignore the upstream timestamp — even when the
    /// source is Opus at 48 kHz (which advances +960 per frame). Passing the
    /// 48 kHz value through made the caller hear ~100 ms of jitter (fast clicks).
    #[test]
    fn ignores_upstream_48khz_timestamp_and_advances_by_160() {
        let mut clock = 0u32;
        // Simulated Amazon Connect Opus 48 kHz timestamps: +960 per 20 ms.
        let upstream = [1_000_000u32, 1_000_960, 1_001_920, 1_002_880];
        let out: Vec<u32> = upstream
            .iter()
            .map(|&u| advance_outbound_timestamp(&mut clock, G711_FRAME_SAMPLES, u))
            .collect();
        // Clean 8 kHz cadence: +160 each, NOT +960, and independent of upstream.
        assert_eq!(out, vec![0, 160, 320, 480]);
    }

    /// Partial frames advance the clock by their actual sample count.
    #[test]
    fn advances_by_actual_samples_for_partial_frames() {
        let mut clock = 500u32;
        assert_eq!(advance_outbound_timestamp(&mut clock, 80, 9_999_999), 500);
        assert_eq!(advance_outbound_timestamp(&mut clock, 160, 0), 580);
        assert_eq!(clock, 740);
    }

    /// The clock wraps at u32 like an RTP timestamp.
    #[test]
    fn wraps_at_u32_boundary() {
        let mut clock = u32::MAX - 100;
        let first = advance_outbound_timestamp(&mut clock, 160, 0);
        assert_eq!(first, u32::MAX - 100);
        assert_eq!(clock, 59); // (MAX - 100) + 160 wraps to 59
    }
}

#[cfg(test)]
mod receiver_ownership_tests {
    use super::*;
    use crate::api::unified::Config as ApiConfig;

    #[test]
    fn second_receiver_acquisition_is_a_typed_error() {
        let stream = SipMediaStream::dormant(Direction::Inbound);

        assert!(stream.try_frames_in().is_ok());
        assert!(matches!(
            stream.try_frames_in(),
            Err(RvoipError::InvalidState(_))
        ));
    }

    #[tokio::test]
    async fn dormant_stream_allocates_no_task_and_close_is_sticky() {
        let stream = SipMediaStream::dormant(Direction::Outbound);
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Dormant);
        assert!(stream
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());
        assert!(stream
            .inner
            .bind_target
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none());

        Arc::clone(&stream).close().await.unwrap();
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
        Arc::clone(&stream).close().await.unwrap();
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
    }

    #[tokio::test]
    async fn one_hundred_bind_callers_share_one_immutable_target() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("media-bind-singleflight", 0))
            .await
            .expect("coordinator");
        let other = UnifiedCoordinator::new(ApiConfig::local("media-bind-mismatch", 0))
            .await
            .expect("second coordinator");
        let stream = SipMediaStream::dormant(Direction::Outbound);
        let session_id = SessionId::new();
        let gate = Arc::new(tokio::sync::Barrier::new(101));
        let mut callers = Vec::new();
        for _ in 0..100 {
            let caller_stream = Arc::clone(&stream);
            let caller_coordinator = Arc::clone(&coordinator);
            let caller_session = session_id.clone();
            let caller_gate = Arc::clone(&gate);
            callers.push(tokio::spawn(async move {
                caller_gate.wait().await;
                caller_stream.bind(caller_coordinator, caller_session).await
            }));
        }
        gate.wait().await;
        for caller in callers {
            assert!(caller.await.expect("bind caller").is_err());
        }
        {
            let target = stream
                .inner
                .bind_target
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(target
                .as_ref()
                .is_some_and(|target| target.matches(&coordinator, &session_id)));
        }
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Failed);

        let _mismatch = stream
            .bind(Arc::clone(&other), session_id.clone())
            .await
            .expect_err("coordinator identity is immutable");
        let _mismatch = stream
            .bind(Arc::clone(&coordinator), SessionId::new())
            .await
            .expect_err("session identity is immutable");
        assert!(stream.is_bound_to(&coordinator, &session_id));
        assert!(!stream.is_bound_to(&other, &session_id));

        Arc::clone(&stream).close().await.unwrap();
        drop(stream);
        coordinator
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown");
        other
            .shutdown_gracefully(Some(std::time::Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn closing_is_monotonic_against_bound_and_failed_races() {
        for _ in 0..100 {
            let lifecycle = Arc::new(SipMediaLifecycleState::new());
            assert!(lifecycle.begin_binding());
            let close_lifecycle = Arc::clone(&lifecycle);
            let close = tokio::spawn(async move { close_lifecycle.begin_closing() });
            let bind_lifecycle = Arc::clone(&lifecycle);
            let bind = tokio::spawn(async move { bind_lifecycle.mark_bound() });
            let _ = tokio::join!(close, bind);
            lifecycle.begin_closing();
            assert_eq!(lifecycle.current(), SipMediaLifecycle::Closing);
            assert!(!lifecycle.mark_bound());
            assert!(!lifecycle.mark_failed());
            lifecycle.mark_closed();
            assert_eq!(lifecycle.current(), SipMediaLifecycle::Closed);
            assert!(!lifecycle.begin_closing());
        }
    }

    #[tokio::test]
    async fn closed_is_published_only_after_driver_termination() {
        let stream = SipMediaStream::dormant(Direction::Outbound);
        assert!(stream.inner.lifecycle.begin_binding());
        let driver = tokio::spawn(std::future::pending::<()>());
        let abort = driver.abort_handle();
        *stream
            .inner
            .driver_abort
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(abort.clone());
        drop(driver);

        let closing_stream = Arc::clone(&stream);
        let close = tokio::spawn(async move { closing_stream.close().await });
        tokio::task::yield_now().await;
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closing);
        assert!(!abort.is_finished());
        abort.abort();
        close.await.expect("close task").expect("stream close");
        assert!(abort.is_finished());
        assert_eq!(stream.inner.lifecycle.current(), SipMediaLifecycle::Closed);
    }
}
