//! The SIP inbound pump fused onto the RTP socket owner (media executor v2).
//!
//! With [`MediaExecutorConfig::fused_inbound_pump`], a G.711 dialog's decoded
//! frames are paced by media-core's inline sink on the task that owns the
//! RTP socket. Without it, each paced frame then crosses the decoded-audio
//! channel to `run_inbound_pump`, a separate task with its own playout timer,
//! which re-paces it through a [`PlayoutBuffer`], re-encodes it and sends a
//! [`MediaFrame`] on `frames_in`. [`FusedInboundPump`] is that pump's state
//! machine as an [`InlineAudioConsumer`]: the owner hands it each frame
//! directly and serves its playout clock from the owner's single timer, so
//! one task wakeup, one queue and one per-call runtime timer leave the
//! per-packet path.
//!
//! The work per event is the task's, in the same order:
//! - a codec generation change is applied before the next frame (the task's
//!   `biased` select), rebuilding the encoder, the playout buffer and the
//!   PCM framer;
//! - a frame is pushed into the playout buffer (or passed straight through
//!   when playout is disabled), then at most eight due frames are popped;
//! - each popped frame goes through the PCM framer (a pass-through for
//!   G.711), is encoded and becomes one `MediaFrame` per encoded payload,
//!   with the same fields.
//!
//! The task awaits `frames_in` capacity, which stalls it and lets decoded
//! frames queue in the 128-frame audio channel until media-core drops new
//! ones. The owner cannot await, so the fused pump keeps the same shape
//! explicitly: unsent frames stay in an outbox (the task parked in `send`),
//! new decoded frames wait in a backlog bounded like the channel (`Full`
//! drops the newest, exactly where media-core's `try_send` would), the
//! playout clock does not advance while parked, and sending is retried every
//! [`BACKPRESSURE_RETRY`] instead of on the consumer's wake. Failures are
//! reported to the idle pump task through a oneshot so the media driver sees
//! the same reason the task would return.
//!
//! With [`MediaExecutorConfig::direct_inbound_sink`], a consumer that only
//! needs the frames synchronously offers an [`InboundFrameSink`] on the
//! stream ([`InboundSinkSlot`]). The pump adopts it at the first turn with
//! nothing parked for `frames_in`, and from then on hands it each encoded
//! frame, borrowed, instead of building a `MediaFrame` (no `StreamId` clone,
//! no `Bytes`, no capture timestamp) and waking the consumer's task through
//! `frames_in`. A sink that reports `Closed` ends the pump with the task's
//! `inbound-consumer-closed`, as a closed `frames_in` does.
//!
//! With [`MediaExecutorConfig::count_only_sink`], a sink that does not read
//! payloads ([`InboundFrameSink::wants_payload`] is false) is handed each
//! G.711 frame with an empty payload and no re-encode. The frames handed
//! over, their order, timestamps and payload type are exactly those of the
//! encoding path.
//!
//! [`MediaExecutorConfig::fused_inbound_pump`]: rvoip_rtp_core::executor::MediaExecutorConfig::fused_inbound_pump
//! [`MediaExecutorConfig::direct_inbound_sink`]: rvoip_rtp_core::executor::MediaExecutorConfig::direct_inbound_sink
//! [`MediaExecutorConfig::count_only_sink`]: rvoip_rtp_core::executor::MediaExecutorConfig::count_only_sink

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use chrono::Utc;
use tokio::sync::{mpsc, oneshot, watch};

use rvoip_core::ids::StreamId;
use rvoip_core::stream::{
    InboundFrameDelivery, InboundFrameSink, InboundFrameView, MediaFrame, StreamKind,
};
use rvoip_media_core::pcm_pool::recycle_pcm_buffer;
use rvoip_media_core::processing::audio::playout::{PlayoutBuffer, PlayoutConfig};
use rvoip_media_core::relay::controller::{InlineAudioConsumer, InlineAudioDelivery};
use rvoip_media_core::types::AudioFrame;
use rvoip_rtp_core::executor::DirectSinkCounters;

use super::{InboundPcmFramer, SipMediaCodecRuntime, SipPayloadCodec};

/// The task emits at most eight frames per wake (`run_inbound_pump`).
const READY_BURST: usize = 8;

/// The payload lent to a count-only sink.
const NO_PAYLOAD: &[u8] = &[];

/// How soon a parked fused pump retries `frames_in` (the task is woken by
/// the consumer instead). Only matters while the consumer is backed up.
pub(super) const BACKPRESSURE_RETRY: Duration = Duration::from_millis(2);

/// The stream's offered inbound frame sink, handed once from the consumer's
/// task to the fused pump on the owner. The pump checks `is_dirty` (one
/// Acquire load per turn) and locks only to take the sink.
pub(super) struct InboundSinkSlot {
    claimed: AtomicBool,
    dirty: AtomicBool,
    sink: Mutex<Option<Box<dyn InboundFrameSink>>>,
}

impl InboundSinkSlot {
    pub(super) fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
            dirty: AtomicBool::new(false),
            sink: Mutex::new(None),
        }
    }

    /// Offer once per stream; a second offer hands the sink back.
    pub(super) fn offer(
        &self,
        sink: Box<dyn InboundFrameSink>,
    ) -> Result<(), Box<dyn InboundFrameSink>> {
        if self.claimed.swap(true, Ordering::AcqRel) {
            return Err(sink);
        }
        *self
            .sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sink);
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }

    #[inline]
    fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::Acquire)
    }

    fn take(&self) -> Option<Box<dyn InboundFrameSink>> {
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return None;
        }
        self.sink
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

pub(super) struct FusedInboundPump {
    runtime: SipMediaCodecRuntime,
    codec_updates: watch::Receiver<Option<SipMediaCodecRuntime>>,
    encoder: SipPayloadCodec,
    framer: InboundPcmFramer,
    stream_id: StreamId,
    /// `None` once the pump failed (the task returned).
    frames_in_tx: Option<mpsc::Sender<MediaFrame>>,
    playout_policy: Option<PlayoutConfig>,
    /// Built at installation, so a refused pump builds no playout state.
    playout: Option<PlayoutBuffer>,
    ready: Vec<AudioFrame>,
    /// Encoded frames not yet accepted by `frames_in`, in order.
    outbox: VecDeque<MediaFrame>,
    /// Decoded frames that arrived while parked (the task: queued in the
    /// audio channel).
    backlog: VecDeque<AudioFrame>,
    backlog_capacity: usize,
    retry_at: Option<Instant>,
    failure: Option<oneshot::Sender<&'static str>>,
    reported: Instant,
    /// Where the consumer may offer a direct sink.
    sink_slot: Arc<InboundSinkSlot>,
    /// Whether an adopted sink that reads no payloads skips the re-encode
    /// (read when the pump is built).
    count_only_enabled: bool,
    /// The adopted direct sink; frames go to it instead of `frames_in` while
    /// nothing is parked in the outbox.
    sink: Option<Box<dyn InboundFrameSink>>,
    /// The direct sink reported `Closed` (the task: `frames_in` closed).
    sink_closed: bool,
    direct_frames: DirectSinkCounters,
    /// The encoded G.711 payload lent to a direct sink, reused for every
    /// frame instead of a fresh `Vec` per frame.
    sink_scratch: Vec<u8>,
    /// The adopted sink reads no payloads: G.711 frames reach it without
    /// being encoded.
    sink_count_only: bool,
}

impl FusedInboundPump {
    /// Build the pump for one stream. The receiver reports the reason the
    /// fused pump stopped, if it stops on its own; it closes without a value
    /// when the pump is merely dropped (detached or torn down).
    pub(super) fn new(
        runtime: SipMediaCodecRuntime,
        codec_updates: watch::Receiver<Option<SipMediaCodecRuntime>>,
        stream_id: StreamId,
        frames_in_tx: mpsc::Sender<MediaFrame>,
        playout_policy: Option<PlayoutConfig>,
        backlog_capacity: usize,
        sink_slot: Arc<InboundSinkSlot>,
    ) -> Result<(Self, oneshot::Receiver<&'static str>), &'static str> {
        let encoder = SipPayloadCodec::from_negotiated(&runtime.negotiated, runtime.payload_type)
            .map_err(|_| "sip-codec-reconfigure-failed")?;
        let framer = InboundPcmFramer::new(&runtime.negotiated);
        let (failure, failure_rx) = oneshot::channel();
        Ok((
            Self {
                runtime,
                codec_updates,
                encoder,
                framer,
                stream_id,
                frames_in_tx: Some(frames_in_tx),
                playout_policy,
                playout: None,
                ready: Vec::with_capacity(READY_BURST),
                outbox: VecDeque::new(),
                backlog: VecDeque::new(),
                backlog_capacity,
                retry_at: None,
                failure: Some(failure),
                reported: Instant::now(),
                sink_slot,
                count_only_enabled: rvoip_rtp_core::executor::count_only_sink_enabled(),
                sink: None,
                sink_closed: false,
                direct_frames: DirectSinkCounters::new(),
                sink_scratch: Vec::new(),
                sink_count_only: false,
            },
            failure_rx,
        ))
    }

    fn failed(&self) -> bool {
        self.frames_in_tx.is_none()
    }

    /// The task returned `reason`.
    fn fail(&mut self, reason: &'static str) -> InlineAudioDelivery {
        if let Some(failure) = self.failure.take() {
            let _ = failure.send(reason);
        }
        self.frames_in_tx = None;
        self.outbox.clear();
        self.backlog.clear();
        self.retry_at = None;
        self.sink = None;
        InlineAudioDelivery::Closed
    }

    /// The task's `biased` codec branch: apply a committed codec generation
    /// before the next frame.
    fn poll_codec(&mut self) -> Result<(), &'static str> {
        match self.codec_updates.has_changed() {
            Ok(false) => return Ok(()),
            Ok(true) => {}
            Err(_) => return Err("sip-codec-update-channel-closed"),
        }
        let Some(updated) = self.codec_updates.borrow_and_update().clone() else {
            return Ok(());
        };
        self.encoder = SipPayloadCodec::from_negotiated(&updated.negotiated, updated.payload_type)
            .map_err(|_| "sip-codec-reconfigure-failed")?;
        self.runtime = updated;
        self.playout = self.playout_policy.map(PlayoutBuffer::new);
        self.framer = InboundPcmFramer::new(&self.runtime.negotiated);
        Ok(())
    }

    /// Adopt an offered sink, only with nothing parked for `frames_in`, so
    /// every sink frame follows every channel frame.
    fn adopt_sink(&mut self) {
        if self.sink.is_some() || !self.outbox.is_empty() || !self.sink_slot.is_dirty() {
            return;
        }
        if let Some(sink) = self.sink_slot.take() {
            // Decided once per sink, never per frame.
            self.sink_count_only = self.count_only_enabled && !sink.wants_payload();
            rvoip_rtp_core::executor::record_direct_sink_active(self.sink_count_only);
            self.sink = Some(sink);
        }
    }

    /// One turn of the task's inner loop after its `select!`: accept the
    /// frame, pop due frames, encode them into the outbox (or, with a direct
    /// sink, straight into the sink).
    fn turn(&mut self, frame: Option<AudioFrame>, now: Instant) {
        self.adopt_sink();
        if let Some(frame) = frame {
            match self.playout.as_mut() {
                Some(buffer) => buffer.push(frame, now),
                None => self.ready.push(frame),
            }
        }
        if let Some(buffer) = self.playout.as_mut() {
            while let Some(frame) = buffer.pop_due(now) {
                self.ready.push(frame);
                if self.ready.len() >= READY_BURST {
                    break;
                }
            }
        }
        self.report_quality();
        let mut ready = std::mem::take(&mut self.ready);
        for frame in ready.drain(..) {
            if self.framer.is_pass_through() {
                self.emit(frame);
            } else {
                for framed in self.framer.push(frame) {
                    self.emit(framed);
                }
            }
        }
        self.ready = ready;
    }

    /// Encode one framed frame into the outbox or the sink.
    fn emit(&mut self, mut frame: AudioFrame) {
        if self.sink_closed {
            // The task returned at the failed send; later frames are dropped.
            recycle_pcm_buffer(std::mem::take(&mut frame.samples));
            return;
        }
        // A G.711 frame bound for the direct sink is encoded into the reused
        // scratch buffer (the bytes `encode_graph_frames` produces: one
        // payload with the frame's own timestamp) and lent to the sink.
        if self.outbox.is_empty() {
            if let (Some(sink), SipPayloadCodec::G711(codec)) = (self.sink.as_mut(), &self.encoder)
            {
                // A count-only sink gets the frame without the encode;
                // `encoded_len` keeps the encoder's answer.
                let encoded = if self.sink_count_only {
                    codec.encoded_len(&frame).map(|_| NO_PAYLOAD)
                } else {
                    codec
                        .encode_into(&frame, &mut self.sink_scratch)
                        .map(|()| &self.sink_scratch[..])
                };
                match encoded {
                    Ok(payload) => {
                        let view = InboundFrameView::new(
                            StreamKind::Audio,
                            payload,
                            frame.timestamp,
                            Some(self.runtime.payload_type),
                        );
                        match sink.on_frame(view) {
                            InboundFrameDelivery::Accepted if self.sink_count_only => {
                                self.direct_frames.count_only_frame()
                            }
                            InboundFrameDelivery::Accepted => self.direct_frames.frame(),
                            InboundFrameDelivery::Closed => self.sink_closed = true,
                        }
                    }
                    Err(error) => {
                        tracing::trace!(target: "rvoip_sip", error = %error, "SipMediaStream: audio encode failed");
                    }
                }
                recycle_pcm_buffer(std::mem::take(&mut frame.samples));
                return;
            }
        }
        let encoded_frames = match self.encoder.encode_graph_frames(&frame) {
            Ok(frames) => frames,
            Err(error) => {
                tracing::trace!(target: "rvoip_sip", error = %error, "SipMediaStream: audio encode failed");
                return;
            }
        };
        recycle_pcm_buffer(std::mem::take(&mut frame.samples));
        for (encoded, timestamp_rtp) in encoded_frames {
            if self.sink_closed {
                continue;
            }
            if self.outbox.is_empty() {
                if let Some(sink) = self.sink.as_mut() {
                    let view = InboundFrameView::new(
                        StreamKind::Audio,
                        &encoded,
                        timestamp_rtp,
                        Some(self.runtime.payload_type),
                    );
                    match sink.on_frame(view) {
                        InboundFrameDelivery::Accepted => self.direct_frames.frame(),
                        InboundFrameDelivery::Closed => self.sink_closed = true,
                    }
                    continue;
                }
            }
            self.outbox.push_back(MediaFrame {
                stream_id: self.stream_id.clone(),
                kind: StreamKind::Audio,
                payload: Bytes::from(encoded),
                timestamp_rtp,
                captured_at: Utc::now(),
                payload_type: Some(self.runtime.payload_type),
            });
        }
    }

    fn report_quality(&mut self) {
        let Some(buffer) = self.playout.as_ref() else {
            return;
        };
        if self.reported.elapsed() >= Duration::from_secs(10) {
            self.reported = Instant::now();
            let stats = buffer.stats();
            if stats.frames_concealed > 0 || stats.frames_late > 0 || stats.frames_catch_up > 0 {
                tracing::debug!(
                    target: "rvoip_sip",
                    emitted = stats.frames_emitted,
                    concealed = stats.frames_concealed,
                    late = stats.frames_late,
                    catch_up = stats.frames_catch_up,
                    depth = stats.depth,
                    "SipMediaStream playout quality"
                );
            }
        }
    }

    /// The task's `frames_in_tx.send(frame).await`, one frame at a time in
    /// order. A full channel leaves the rest parked in the outbox.
    fn flush_outbox(&mut self, now: Instant) -> Result<(), &'static str> {
        if self.sink_closed {
            // The direct consumer is gone: the task's send to a closed
            // `frames_in`.
            return Err("inbound-consumer-closed");
        }
        let Some(frames_in) = self.frames_in_tx.as_ref() else {
            return Err("inbound-consumer-closed");
        };
        while let Some(frame) = self.outbox.pop_front() {
            match frames_in.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(frame)) => {
                    self.outbox.push_front(frame);
                    self.retry_at = Some(now + BACKPRESSURE_RETRY);
                    return Ok(());
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    return Err("inbound-consumer-closed");
                }
            }
        }
        self.retry_at = None;
        Ok(())
    }

    /// Resume as the task does when `frames_in` frees up: finish the parked
    /// sends, then take queued frames one per turn, codec changes first.
    fn drain(&mut self, now: Instant) -> Result<(), &'static str> {
        loop {
            self.flush_outbox(now)?;
            if !self.outbox.is_empty() {
                return Ok(());
            }
            self.poll_codec()?;
            let Some(frame) = self.backlog.pop_front() else {
                return Ok(());
            };
            self.turn(Some(frame), now);
        }
    }

    fn playout_due(&self, now: Instant) -> bool {
        self.playout
            .as_ref()
            .and_then(PlayoutBuffer::next_deadline)
            .is_some_and(|deadline| deadline <= now)
    }

    fn service(&mut self, now: Instant) -> Result<(), &'static str> {
        self.drain(now)?;
        // The task's playout timer branch, armed only while not parked in
        // `send`.
        if self.outbox.is_empty() && self.playout_due(now) {
            self.turn(None, now);
            self.flush_outbox(now)?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn parked(&self) -> (usize, usize) {
        (self.outbox.len(), self.backlog.len())
    }

    #[cfg(test)]
    pub(super) fn has_direct_sink(&self) -> bool {
        self.sink.is_some()
    }
}

impl InlineAudioConsumer for FusedInboundPump {
    fn on_installed(&mut self, now: Instant) {
        self.playout = self.playout_policy.map(PlayoutBuffer::new);
        self.reported = now;
    }

    fn on_frame(&mut self, frame: AudioFrame, now: Instant) -> InlineAudioDelivery {
        if self.failed() {
            return InlineAudioDelivery::Closed;
        }
        if let Err(reason) = self.drain(now) {
            return self.fail(reason);
        }
        if self.outbox.is_empty() && self.backlog.is_empty() {
            self.turn(Some(frame), now);
            if let Err(reason) = self.flush_outbox(now) {
                return self.fail(reason);
            }
            return InlineAudioDelivery::Accepted;
        }
        if self.backlog.len() < self.backlog_capacity {
            self.backlog.push_back(frame);
            return InlineAudioDelivery::Accepted;
        }
        rvoip_rtp_core::executor::record_fused_backlog_drop();
        // The dropped frame's PCM goes back to the pool.
        recycle_pcm_buffer(frame.samples);
        InlineAudioDelivery::Full
    }

    fn next_deadline(&self) -> Option<Instant> {
        if self.failed() {
            return None;
        }
        if !self.outbox.is_empty() {
            return self.retry_at;
        }
        self.playout.as_ref().and_then(PlayoutBuffer::next_deadline)
    }

    fn on_deadline(&mut self, now: Instant) -> InlineAudioDelivery {
        if self.failed() {
            return InlineAudioDelivery::Closed;
        }
        match self.service(now) {
            Ok(()) => InlineAudioDelivery::Accepted,
            Err(reason) => self.fail(reason),
        }
    }

    fn on_detach(&mut self, now: Instant) {
        if !self.failed() {
            if let Err(reason) = self.service(now) {
                let _ = self.fail(reason);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! v1/v2 equivalence for the fused inbound pump: the same decoded-frame
    //! script is fed to the `run_inbound_pump` task (real time, through the
    //! audio channel) and to the fused pump (virtual time, driven the way the
    //! socket owner drives it). `frames_in` (or the direct sink) must carry
    //! identical frames.
    use super::*;
    use rvoip_rtp_core::executor::{
        override_media_executor_config_on_this_thread, MediaExecutorConfig,
    };
    use std::net::SocketAddr;

    fn runtime(codec: &str, payload_type: u8) -> SipMediaCodecRuntime {
        SipMediaCodecRuntime {
            negotiated: crate::session_store::state::NegotiatedConfig {
                local_addr: SocketAddr::from(([127, 0, 0, 1], 10_000)),
                remote_addr: SocketAddr::from(([127, 0, 0, 1], 20_000)),
                codec: codec.to_string(),
                sample_rate: 8_000,
                channels: 1,
                fmtp: None,
            },
            payload_type,
        }
    }

    fn pcmu() -> SipMediaCodecRuntime {
        runtime("PCMU", 0)
    }

    /// Fixed depth so real-time scheduling noise cannot change the target.
    fn playout() -> PlayoutConfig {
        PlayoutConfig {
            target_depth_frames: 2,
            max_depth_frames: 10,
            max_consecutive_concealed: 5,
            adaptive: false,
        }
    }

    fn frame(index: u32) -> AudioFrame {
        let samples = (0..160_i32)
            .map(|n| ((index as i32 * 389 + n * 97) % 4000 - 2000) as i16)
            .collect();
        AudioFrame::new(samples, 8_000, 1, index * 160)
    }

    /// (milliseconds from the start, frame index)
    type Script = [(u64, u32)];
    type Output = Vec<(u32, Vec<u8>, Option<u8>)>;

    fn observed(frame: &MediaFrame) -> (u32, Vec<u8>, Option<u8>) {
        (
            frame.timestamp_rtp,
            frame.payload.to_vec(),
            frame.payload_type,
        )
    }

    async fn run_v1(script: &Script, config: Option<PlayoutConfig>, wanted: usize) -> Output {
        let (audio_tx, audio_rx) = mpsc::channel(128);
        let (frames_tx, mut frames_rx) = mpsc::channel(256);
        let (codec_tx, codec_rx) = watch::channel(Some(pcmu()));
        let subscriber = crate::types::AudioFrameSubscriber::new(crate::SessionId::new(), audio_rx);
        let pump = tokio::spawn(super::super::run_inbound_pump(
            subscriber,
            pcmu(),
            codec_rx,
            StreamId::new(),
            frames_tx,
            config,
            None,
        ));
        let start = tokio::time::Instant::now();
        for &(at, index) in script {
            tokio::time::sleep_until(start + Duration::from_millis(at)).await;
            audio_tx.send(frame(index)).await.unwrap();
        }
        let mut output = Vec::new();
        while output.len() < wanted {
            let frame = tokio::time::timeout(Duration::from_secs(2), frames_rx.recv())
                .await
                .expect("the pump task emits")
                .expect("open");
            output.push(observed(&frame));
        }
        pump.abort();
        drop(codec_tx);
        output
    }

    type Fused = (
        FusedInboundPump,
        oneshot::Receiver<&'static str>,
        watch::Sender<Option<SipMediaCodecRuntime>>,
    );

    fn fused(
        frames_in: mpsc::Sender<MediaFrame>,
        config: Option<PlayoutConfig>,
        backlog: usize,
    ) -> Fused {
        fused_with_slot(frames_in, config, backlog, Arc::new(InboundSinkSlot::new()))
    }

    fn fused_with_slot(
        frames_in: mpsc::Sender<MediaFrame>,
        config: Option<PlayoutConfig>,
        backlog: usize,
        slot: Arc<InboundSinkSlot>,
    ) -> Fused {
        let (codec_tx, codec_rx) = watch::channel(Some(pcmu()));
        let (pump, stopped) = FusedInboundPump::new(
            pcmu(),
            codec_rx,
            StreamId::new(),
            frames_in,
            config,
            backlog,
            slot,
        )
        .unwrap();
        (pump, stopped, codec_tx)
    }

    /// A direct sink that records what it is handed. Reports `Closed` once
    /// `open` is cleared.
    struct RecordingSink {
        seen: Arc<Mutex<Output>>,
        open: Arc<AtomicBool>,
    }

    impl InboundFrameSink for RecordingSink {
        fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery {
            if !self.open.load(Ordering::Acquire) {
                return InboundFrameDelivery::Closed;
            }
            assert_eq!(frame.kind, StreamKind::Audio);
            self.seen.lock().unwrap().push((
                frame.timestamp_rtp,
                frame.payload.to_vec(),
                frame.payload_type,
            ));
            InboundFrameDelivery::Accepted
        }
    }

    fn recording_sink() -> (
        Box<dyn InboundFrameSink>,
        Arc<Mutex<Output>>,
        Arc<AtomicBool>,
    ) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let open = Arc::new(AtomicBool::new(true));
        (
            Box::new(RecordingSink {
                seen: Arc::clone(&seen),
                open: Arc::clone(&open),
            }),
            seen,
            open,
        )
    }

    /// Serve the fused pump's deadlines up to `target`, as the owner timer
    /// would.
    fn advance(pump: &mut FusedInboundPump, mut now: Instant, target: Instant) -> Instant {
        while let Some(deadline) = pump.next_deadline() {
            if deadline > target {
                break;
            }
            now = now.max(deadline);
            assert_eq!(pump.on_deadline(now), InlineAudioDelivery::Accepted);
        }
        target.max(now)
    }

    /// Feed `script` and serve deadlines until `done` says enough was seen.
    fn drive_script(pump: &mut FusedInboundPump, script: &Script, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        pump.on_installed(start);
        let mut now = start;
        for &(at, index) in script {
            now = advance(pump, now, start + Duration::from_millis(at));
            assert_eq!(
                pump.on_frame(frame(index), now),
                InlineAudioDelivery::Accepted
            );
        }
        for _ in 0..10_000 {
            if done() {
                return;
            }
            let deadline = pump
                .next_deadline()
                .expect("a primed playout keeps a deadline");
            now = now.max(deadline);
            assert_eq!(pump.on_deadline(now), InlineAudioDelivery::Accepted);
        }
        panic!("the fused pump did not emit enough frames");
    }

    fn run_fused(script: &Script, config: Option<PlayoutConfig>, wanted: usize) -> Output {
        let (frames_tx, mut frames_rx) = mpsc::channel(256);
        let (mut pump, _stopped, _codec) = fused(frames_tx, config, 128);
        let mut output = Vec::new();
        drive_script(&mut pump, script, || {
            while let Ok(frame) = frames_rx.try_recv() {
                output.push(observed(&frame));
            }
            output.len() >= wanted
        });
        output.truncate(wanted);
        output
    }

    /// `run_fused` with a direct sink offered before the first frame: the
    /// sink must see what `frames_in` would have carried, and `frames_in`
    /// must stay empty.
    fn run_fused_with_sink(
        script: &Script,
        config: Option<PlayoutConfig>,
        wanted: usize,
    ) -> Output {
        let (frames_tx, mut frames_rx) = mpsc::channel(256);
        let slot = Arc::new(InboundSinkSlot::new());
        let (sink, seen, _open) = recording_sink();
        assert!(slot.offer(sink).is_ok());
        let (mut pump, _stopped, _codec) = fused_with_slot(frames_tx, config, 128, slot);
        let seen_for_check = Arc::clone(&seen);
        drive_script(&mut pump, script, || {
            seen_for_check.lock().unwrap().len() >= wanted
        });
        assert!(pump.has_direct_sink());
        assert!(frames_rx.try_recv().is_err(), "nothing crossed frames_in");
        let mut output = seen.lock().unwrap().clone();
        output.truncate(wanted);
        output
    }

    async fn assert_equivalent(script: &Script, config: Option<PlayoutConfig>, expected: &[u32]) {
        let wanted = expected.len();
        let v1 = run_v1(script, config, wanted).await;
        let v2 = run_fused(script, config, wanted);
        let timestamps: Vec<u32> = v2.iter().map(|(timestamp, _, _)| *timestamp).collect();
        assert_eq!(
            timestamps,
            expected.iter().map(|index| index * 160).collect::<Vec<_>>()
        );
        assert_eq!(v1, v2, "{script:?}");
        assert!(v2
            .iter()
            .all(|(_, payload, pt)| payload.len() == 160 && *pt == Some(0)));
    }

    /// Frame `i` arrives at `20·(i−1)` ms, except frame 2 at 10 ms. Frame 2
    /// primes the playout, so its clock runs 10 ms off the arrival grid and
    /// no arrival ties with a playout deadline: real-time scheduling noise in
    /// the task run cannot reorder an arrival and a tick.
    fn paced(indices: impl IntoIterator<Item = u32>) -> Vec<(u64, u32)> {
        indices
            .into_iter()
            .map(|index| {
                (
                    if index == 2 {
                        10
                    } else {
                        u64::from(index - 1) * 20
                    },
                    index,
                )
            })
            .collect()
    }

    /// A script, its playout policy and how many frames it must emit.
    type Case = (Vec<(u64, u32)>, Option<PlayoutConfig>, usize);

    fn scripts() -> Vec<Case> {
        let mut reordered = paced([1, 2, 3, 6, 7, 8]);
        reordered.push((61, 5));
        reordered.push((62, 4));
        reordered.sort();
        let mut late = paced([1, 2, 3, 4, 6, 7, 8, 9, 10]);
        late.push((200, 5));
        late.sort();
        let burst: Vec<(u64, u32)> = (1..=6).map(|index| (0, index)).collect();
        vec![
            (paced(1..=12), Some(playout()), 12),
            (reordered, Some(playout()), 8),
            (paced([1, 2, 3, 4, 6, 7, 8, 9, 10]), Some(playout()), 10),
            (late, Some(playout()), 10),
            (burst, Some(playout()), 6),
            (vec![(0, 1), (20, 3), (40, 2), (60, 4)], None, 4),
        ]
    }

    #[tokio::test]
    async fn in_order_stream_is_identical() {
        assert_equivalent(
            &paced(1..=12),
            Some(playout()),
            &(1..=12).collect::<Vec<_>>(),
        )
        .await;
    }

    #[tokio::test]
    async fn reordered_frames_play_in_media_order_identically() {
        let mut script = paced([1, 2, 3, 6, 7, 8]);
        script.push((61, 5));
        script.push((62, 4));
        script.sort();
        assert_equivalent(&script, Some(playout()), &(1..=8).collect::<Vec<_>>()).await;
    }

    #[tokio::test]
    async fn a_lost_frame_is_concealed_identically() {
        let script = paced([1, 2, 3, 4, 6, 7, 8, 9, 10]);
        assert_equivalent(&script, Some(playout()), &(1..=10).collect::<Vec<_>>()).await;
    }

    #[tokio::test]
    async fn a_late_frame_is_dropped_and_its_slot_concealed_identically() {
        // Frame 5's playout slot is 120 ms; it arrives at 200 ms.
        let mut script = paced([1, 2, 3, 4, 6, 7, 8, 9, 10]);
        script.push((200, 5));
        script.sort();
        assert_equivalent(&script, Some(playout()), &(1..=10).collect::<Vec<_>>()).await;
    }

    #[tokio::test]
    async fn a_burst_drains_through_catch_up_identically() {
        let script: Vec<(u64, u32)> = (1..=6).map(|index| (0, index)).collect();
        assert_equivalent(&script, Some(playout()), &(1..=6).collect::<Vec<_>>()).await;
    }

    #[tokio::test]
    async fn disabled_playout_forwards_in_arrival_order_identically() {
        let script = [(0, 1), (20, 3), (40, 2), (60, 4)];
        assert_equivalent(&script, None, &[1, 3, 2, 4]).await;
    }

    #[test]
    fn full_frames_in_parks_like_an_awaited_send_and_resumes_in_order() {
        let (frames_tx, mut frames_rx) = mpsc::channel(1);
        let (mut pump, _stopped, _codec) = fused(frames_tx, None, 2);
        let now = Instant::now();
        pump.on_installed(now);
        // 1 is sent; 2 parks in the outbox (the task: awaiting `send`).
        assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.on_frame(frame(2), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.parked(), (1, 0));
        assert_eq!(pump.next_deadline(), Some(now + BACKPRESSURE_RETRY));
        // 3 and 4 queue (the task: the audio channel); 5 overflows
        // (media-core's `try_send` reports Full and drops it).
        assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.on_frame(frame(4), now), InlineAudioDelivery::Accepted);
        let drops = rvoip_rtp_core::executor::inline_executor_counters().fused_backlog_drops;
        assert_eq!(pump.on_frame(frame(5), now), InlineAudioDelivery::Full);
        assert!(rvoip_rtp_core::executor::inline_executor_counters().fused_backlog_drops > drops);
        assert_eq!(pump.parked(), (1, 2));
        // A retry with no room changes nothing.
        let retry = now + BACKPRESSURE_RETRY;
        assert_eq!(pump.on_deadline(retry), InlineAudioDelivery::Accepted);
        assert_eq!(pump.parked(), (1, 2));
        let mut received = Vec::new();
        let mut at = retry;
        for _ in 0..8 {
            if let Ok(frame) = frames_rx.try_recv() {
                received.push(frame.timestamp_rtp / 160);
            }
            at += BACKPRESSURE_RETRY;
            assert_eq!(pump.on_deadline(at), InlineAudioDelivery::Accepted);
        }
        while let Ok(frame) = frames_rx.try_recv() {
            received.push(frame.timestamp_rtp / 160);
        }
        assert_eq!(received, vec![1, 2, 3, 4]);
        assert_eq!(pump.parked(), (0, 0));
        assert_eq!(pump.next_deadline(), None, "nothing parked, no playout");
    }

    #[test]
    fn a_closed_consumer_reports_the_task_reason_once() {
        let (frames_tx, frames_rx) = mpsc::channel(4);
        let (mut pump, mut stopped, _codec) = fused(frames_tx, Some(playout()), 8);
        let now = Instant::now();
        pump.on_installed(now);
        drop(frames_rx);
        // Two frames prime the playout; the third drains one to a closed
        // channel.
        assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.on_frame(frame(2), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Closed);
        assert_eq!(stopped.try_recv(), Ok("inbound-consumer-closed"));
        assert_eq!(pump.on_frame(frame(4), now), InlineAudioDelivery::Closed);
        assert_eq!(pump.next_deadline(), None);
    }

    #[test]
    fn a_codec_change_applies_before_the_next_frame_and_resets_playout() {
        let (frames_tx, mut frames_rx) = mpsc::channel(8);
        let (mut pump, _stopped, codec) = fused(frames_tx, None, 8);
        let now = Instant::now();
        pump.on_installed(now);
        assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
        codec.send_replace(Some(runtime("PCMA", 8)));
        // The change is applied when the pump next drains, before the frame.
        assert_eq!(pump.on_frame(frame(2), now), InlineAudioDelivery::Accepted);
        let first = frames_rx.try_recv().unwrap();
        let second = frames_rx.try_recv().unwrap();
        assert_eq!(first.payload_type, Some(0));
        assert_eq!(second.payload_type, Some(8));
        assert_ne!(first.payload, second.payload);
        drop(codec);
        assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Closed);
    }

    #[tokio::test]
    async fn the_idle_task_returns_the_fused_reason_or_carries_on_after_detach() {
        // Failure: the task reports the fused pump's reason as its own.
        let (_audio_tx, audio_rx) = mpsc::channel(4);
        let (frames_tx, _frames_rx) = mpsc::channel(4);
        let (_codec_tx, codec_rx) = watch::channel(Some(pcmu()));
        let (stop_tx, stop_rx) = oneshot::channel();
        let pump = tokio::spawn(super::super::run_inbound_pump(
            crate::types::AudioFrameSubscriber::new(crate::SessionId::new(), audio_rx),
            pcmu(),
            codec_rx,
            StreamId::new(),
            frames_tx,
            Some(playout()),
            Some(stop_rx),
        ));
        stop_tx.send("inbound-consumer-closed").unwrap();
        assert_eq!(pump.await.unwrap(), "inbound-consumer-closed");

        // Detach: the task keeps serving its channel, building its playout
        // only now, and still ends when the audio source closes.
        let (audio_tx, audio_rx) = mpsc::channel(4);
        let (frames_tx, mut frames_rx) = mpsc::channel(4);
        let (_codec_tx, codec_rx) = watch::channel(Some(pcmu()));
        let (stop_tx, stop_rx) = oneshot::channel::<&'static str>();
        let pump = tokio::spawn(super::super::run_inbound_pump(
            crate::types::AudioFrameSubscriber::new(crate::SessionId::new(), audio_rx),
            pcmu(),
            codec_rx,
            StreamId::new(),
            frames_tx,
            None,
            Some(stop_rx),
        ));
        drop(stop_tx);
        audio_tx.send(frame(7)).await.unwrap();
        let forwarded = tokio::time::timeout(Duration::from_secs(2), frames_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(forwarded.timestamp_rtp, 7 * 160);
        drop(audio_tx);
        assert_eq!(pump.await.unwrap(), "sip-audio-source-closed");
    }

    #[tokio::test]
    async fn a_direct_sink_sees_exactly_the_frames_frames_in_would() {
        for (script, config, wanted) in scripts() {
            let channel = run_fused(&script, config, wanted);
            let direct = run_fused_with_sink(&script, config, wanted);
            assert_eq!(channel, direct, "{script:?}");
            // And the task through the audio channel, as the other tests.
            assert_eq!(run_v1(&script, config, wanted).await, direct, "{script:?}");
        }
    }

    #[test]
    fn a_sink_offered_while_frames_are_parked_is_adopted_after_they_drain() {
        let (frames_tx, mut frames_rx) = mpsc::channel(1);
        let slot = Arc::new(InboundSinkSlot::new());
        let (mut pump, _stopped, _codec) = fused_with_slot(frames_tx, None, 8, Arc::clone(&slot));
        let now = Instant::now();
        pump.on_installed(now);
        // 1 is sent; 2 parks in the outbox (frames_in is full).
        assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.on_frame(frame(2), now), InlineAudioDelivery::Accepted);
        assert_eq!(pump.parked(), (1, 0));
        let (sink, seen, _open) = recording_sink();
        assert!(slot.offer(sink).is_ok());
        let (second, _, _) = recording_sink();
        assert!(slot.offer(second).is_err(), "one sink per stream");
        // 3 waits behind the parked frame; the sink is not adopted yet.
        assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Accepted);
        assert!(!pump.has_direct_sink());
        assert_eq!(frames_rx.try_recv().unwrap().timestamp_rtp, 160);
        let retry = now + BACKPRESSURE_RETRY;
        assert_eq!(pump.on_deadline(retry), InlineAudioDelivery::Accepted);
        // 2 left through frames_in; 3 (and 4) went to the sink.
        assert_eq!(frames_rx.try_recv().unwrap().timestamp_rtp, 2 * 160);
        assert!(pump.has_direct_sink());
        assert_eq!(
            pump.on_frame(frame(4), retry),
            InlineAudioDelivery::Accepted
        );
        let direct: Vec<u32> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|(ts, _, _)| ts / 160)
            .collect();
        assert_eq!(direct, vec![3, 4]);
        assert!(frames_rx.try_recv().is_err());
        assert_eq!(pump.parked(), (0, 0));
    }

    #[test]
    fn a_closed_direct_sink_ends_the_pump_like_a_closed_frames_in() {
        let (frames_tx, _frames_rx) = mpsc::channel(4);
        let slot = Arc::new(InboundSinkSlot::new());
        let (sink, seen, open) = recording_sink();
        assert!(slot.offer(sink).is_ok());
        let (mut pump, mut stopped, _codec) = fused_with_slot(frames_tx, None, 8, slot);
        let now = Instant::now();
        pump.on_installed(now);
        assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
        open.store(false, Ordering::Release);
        assert_eq!(pump.on_frame(frame(2), now), InlineAudioDelivery::Closed);
        assert_eq!(stopped.try_recv(), Ok("inbound-consumer-closed"));
        assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Closed);
        assert_eq!(pump.next_deadline(), None);
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    // ---- Count-only sink and the allocation-free steady state ----

    /// A sink that only tallies frames, as a recording counter does.
    struct TallySink {
        frames: Arc<std::sync::atomic::AtomicU64>,
        payload_bytes: Arc<std::sync::atomic::AtomicU64>,
        wants_payload: bool,
    }

    impl InboundFrameSink for TallySink {
        fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery {
            self.frames.fetch_add(1, Ordering::Relaxed);
            self.payload_bytes
                .fetch_add(frame.payload.len() as u64, Ordering::Relaxed);
            InboundFrameDelivery::Accepted
        }

        fn wants_payload(&self) -> bool {
            self.wants_payload
        }
    }

    /// What a count-only sink was lent: timestamp, payload length, payload
    /// type.
    type CountedFrames = Vec<(u32, usize, Option<u8>)>;

    /// A count-only sink that also records what it is lent.
    struct CountOnlyRecorder {
        seen: Arc<Mutex<CountedFrames>>,
    }

    impl InboundFrameSink for CountOnlyRecorder {
        fn on_frame(&mut self, frame: InboundFrameView<'_>) -> InboundFrameDelivery {
            assert_eq!(frame.kind, StreamKind::Audio);
            self.seen.lock().unwrap().push((
                frame.timestamp_rtp,
                frame.payload.len(),
                frame.payload_type,
            ));
            InboundFrameDelivery::Accepted
        }

        fn wants_payload(&self) -> bool {
            false
        }
    }

    /// `run_fused_with_sink` with a sink that does not want payloads, built
    /// under v2 (count-only on): what it was lent, in order.
    fn run_fused_count_only(
        script: &Script,
        config: Option<PlayoutConfig>,
        wanted: usize,
    ) -> Vec<(u32, usize, Option<u8>)> {
        let (frames_tx, mut frames_rx) = mpsc::channel(256);
        let slot = Arc::new(InboundSinkSlot::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        assert!(slot
            .offer(Box::new(CountOnlyRecorder {
                seen: Arc::clone(&seen)
            }))
            .is_ok());
        let (mut pump, _stopped, _codec) = fused_with_slot(frames_tx, config, 128, slot);
        let seen_for_check = Arc::clone(&seen);
        drive_script(&mut pump, script, || {
            seen_for_check.lock().unwrap().len() >= wanted
        });
        assert!(pump.has_direct_sink());
        assert!(
            pump.sink_count_only,
            "a payload-free sink is count-only under v2"
        );
        assert!(frames_rx.try_recv().is_err(), "nothing crossed frames_in");
        drop(pump);
        let mut output = seen.lock().unwrap().clone();
        output.truncate(wanted);
        output
    }

    /// A count-only sink is lent exactly the frames (count, order,
    /// timestamps, payload type) the encoding paths carry; only the payload
    /// is empty. Reorder, loss/concealment, late drop, catch-up burst and
    /// disabled playout.
    #[tokio::test]
    async fn a_count_only_sink_sees_the_frames_of_the_encoding_path_without_payloads() {
        for (script, config, wanted) in scripts() {
            let channel = run_fused(&script, config, wanted);
            assert_eq!(run_v1(&script, config, wanted).await, channel, "{script:?}");
            let counted = {
                let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
                let before = rvoip_rtp_core::executor::inline_executor_counters();
                let counted = run_fused_count_only(&script, config, wanted);
                let after = rvoip_rtp_core::executor::inline_executor_counters();
                assert!(after.count_only_sinks > before.count_only_sinks);
                assert!(after.count_only_frames >= before.count_only_frames + wanted as u64);
                counted
            };
            let expected: Vec<(u32, usize, Option<u8>)> = channel
                .iter()
                .map(|(timestamp, _, payload_type)| (*timestamp, 0, *payload_type))
                .collect();
            assert_eq!(counted, expected, "{script:?}");
        }
    }

    /// Under v1 (or with the switch off) a payload-free sink still gets the
    /// encoded payloads: the re-encode is skipped only when asked for.
    #[test]
    fn count_only_needs_its_switch() {
        for config in [
            MediaExecutorConfig::v1(),
            MediaExecutorConfig::v2().with_count_only_sink(false),
        ] {
            let _config = override_media_executor_config_on_this_thread(config);
            let (frames_tx, _frames_rx) = mpsc::channel(4);
            let slot = Arc::new(InboundSinkSlot::new());
            let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
            assert!(slot
                .offer(Box::new(TallySink {
                    frames: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                    payload_bytes: Arc::clone(&bytes),
                    wants_payload: false,
                }))
                .is_ok());
            let (mut pump, _stopped, _codec) = fused_with_slot(frames_tx, None, 8, slot);
            let now = Instant::now();
            pump.on_installed(now);
            assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
            assert!(!pump.sink_count_only);
            assert_eq!(bytes.load(Ordering::Relaxed), 160);
        }
    }

    /// An empty decoded frame encodes to an empty payload, so the encoding
    /// sink sees it with no bytes; the count-only sink counts it the same
    /// way.
    #[test]
    fn an_empty_frame_is_counted_by_the_count_only_sink_as_by_the_encoder() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        let mut tallies = Vec::new();
        for wants_payload in [true, false] {
            let (frames_tx, _frames_rx) = mpsc::channel(4);
            let slot = Arc::new(InboundSinkSlot::new());
            let frames = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
            assert!(slot
                .offer(Box::new(TallySink {
                    frames: Arc::clone(&frames),
                    payload_bytes: Arc::clone(&bytes),
                    wants_payload,
                }))
                .is_ok());
            let (mut pump, _stopped, _codec) = fused_with_slot(frames_tx, None, 8, slot);
            let now = Instant::now();
            pump.on_installed(now);
            assert_eq!(pump.on_frame(frame(1), now), InlineAudioDelivery::Accepted);
            let empty = AudioFrame::new(Vec::new(), 8_000, 1, 2 * 160);
            assert_eq!(pump.on_frame(empty, now), InlineAudioDelivery::Accepted);
            assert_eq!(pump.on_frame(frame(3), now), InlineAudioDelivery::Accepted);
            assert_eq!(pump.sink_count_only, !wants_payload);
            tallies.push((
                frames.load(Ordering::Relaxed),
                bytes.load(Ordering::Relaxed),
            ));
        }
        assert_eq!(
            tallies,
            vec![(3, 320), (3, 0)],
            "same frames, no payload when count-only"
        );
    }

    /// Feed the prepared frames (popped from the back) whose index is at
    /// most `through`, frame i arriving at 20·(i−1) ms, and serve the playout
    /// deadlines in between, as the owner does.
    fn drive(
        pump: &mut FusedInboundPump,
        frames: &mut Vec<AudioFrame>,
        start: Instant,
        mut now: Instant,
        through: u32,
    ) -> Instant {
        while frames
            .last()
            .is_some_and(|frame| frame.timestamp <= through * 160)
        {
            let frame = frames.pop().expect("checked above");
            let index = frame.timestamp / 160;
            now = advance(
                pump,
                now,
                start + Duration::from_millis(u64::from(index - 1) * 20),
            );
            assert_eq!(pump.on_frame(frame, now), InlineAudioDelivery::Accepted);
        }
        now
    }

    /// After warm-up, the fused pump hands frames to a direct sink —
    /// count-only, or encoding into its reused scratch — without allocating
    /// a PCM or payload buffer: concealment of a lost frame takes its PCM
    /// from the pool (no pool miss on this thread), the encode scratch and
    /// the ready list keep their storage, and a count-only sink never
    /// touches the scratch at all.
    #[test]
    fn steady_state_direct_sink_frames_reuse_every_buffer() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        use rvoip_media_core::pcm_pool::{
            pcm_pool_len_on_this_thread, pcm_pool_misses_on_this_thread,
        };
        const LAST: u32 = 600;
        // One loss in the warm-up (so every first use happens there) and one
        // in the measured stretch.
        const LOST: [u32; 2] = [150, 450];
        for count_only in [true, false] {
            let (frames_tx, _frames_rx) = mpsc::channel(4);
            let slot = Arc::new(InboundSinkSlot::new());
            let tally = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
            assert!(slot
                .offer(Box::new(TallySink {
                    frames: Arc::clone(&tally),
                    payload_bytes: Arc::clone(&bytes),
                    wants_payload: !count_only,
                }))
                .is_ok());
            let (mut pump, _stopped, _codec) =
                fused_with_slot(frames_tx, Some(playout()), 128, slot);
            let mut frames: Vec<AudioFrame> = (1..=LAST)
                .rev()
                .filter(|index| !LOST.contains(index))
                .map(frame)
                .collect();
            let start = Instant::now();
            pump.on_installed(start);
            let now = drive(&mut pump, &mut frames, start, start, 300);
            assert_eq!(pump.sink_count_only, count_only);
            let concealed = |pump: &FusedInboundPump| {
                pump.playout
                    .as_ref()
                    .map_or(0, |playout| playout.stats().frames_concealed)
            };
            let concealed_before = concealed(&pump);
            assert!(concealed_before >= 1, "the warm-up loss was concealed");
            let misses = pcm_pool_misses_on_this_thread();
            let pooled = pcm_pool_len_on_this_thread();
            let scratch = (pump.sink_scratch.as_ptr(), pump.sink_scratch.capacity());
            let ready = (pump.ready.as_ptr(), pump.ready.capacity());

            drive(&mut pump, &mut frames, start, now, LAST);

            assert!(
                concealed(&pump) > concealed_before,
                "the measured loss was concealed"
            );
            assert_eq!(
                pcm_pool_misses_on_this_thread(),
                misses,
                "count_only={count_only}: concealment PCM came from the pool"
            );
            // v2's counter_pool_cleanup (#359) keeps up to 1024 buffers, so
            // the test's input frames join the pool once played out: no
            // pooled buffer may be lost, and the pool may grow.
            assert!(
                pcm_pool_len_on_this_thread() >= pooled,
                "every pooled buffer came back: {} < {pooled}",
                pcm_pool_len_on_this_thread()
            );
            assert_eq!(
                (pump.sink_scratch.as_ptr(), pump.sink_scratch.capacity()),
                scratch
            );
            assert_eq!((pump.ready.as_ptr(), pump.ready.capacity()), ready);
            if count_only {
                assert_eq!(pump.sink_scratch.capacity(), 0, "nothing was ever encoded");
            }
            let tally = tally.load(Ordering::Relaxed);
            assert!(
                tally >= u64::from(LAST) - 10,
                "frames reached the sink: {tally}"
            );
            let bytes = bytes.load(Ordering::Relaxed);
            if count_only {
                assert_eq!(bytes, 0, "no payloads");
            } else {
                assert_eq!(bytes, tally * 160);
            }
        }
    }
}
