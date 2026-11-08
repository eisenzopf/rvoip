//! Media executor v2: the media layer's half of the inline receive path.
//!
//! When `MediaExecutorConfig::inline_decode` is active, [`InlineAudioSink`]
//! runs inside the socket owner's receive loop (see
//! `rvoip_rtp_core::executor`). For each packet it performs exactly what the
//! v1 RTP event handler (`spawn_rtp_event_handler_with_inline` in `mod.rs`)
//! does for `PacketReceived` followed by one turn of its loop top: the
//! payload-type check, the codec-generation and source checks of the
//! [`SourceTimeline`], optional quality diagnostics, the [`RtpPlayoutBuffer`]
//! reorder hold with bounded concealment, the source-timeline mapping of every
//! decoded and concealed frame, G.711 decode, the paced
//! [`DecodedPlayoutQueue`], and delivery to the audio-frame callback channel.
//! Its reorder and pacing deadlines are served by the owner loop's single
//! timer instead of the handler's own timer.
//!
//! Differences from v1 are mechanical only:
//! - per-dialog control state (codec generation, media direction, audio
//!   callback) is read from one owner-held [`DialogInlineLease`] instead of
//!   String-keyed `DashMap` lookups per packet; the controller keeps the maps
//!   as the source of truth and re-syncs the lease after each change;
//! - an in-order packet is decoded straight from the owner's receive buffer;
//!   only a packet that must be held for reordering is copied into `Bytes`;
//! - decoded and concealed PCM uses the per-thread pool
//!   ([`crate::pcm_pool`]) when it is enabled.
//!
//! Only stateless G.711 is decoded inline. Any other codec, at start or after
//! a re-INVITE, and any bridge that needs `PacketReceived` demotes the dialog
//! to the v1 handler, which is always spawned and stays subscribed (it serves
//! DTMF throughout). On demotion the sink delivers its paced frames, hands
//! back the packets it holds for reordering and hands its source timeline to
//! the v1 handler, so decoded timestamps continue on the same call timeline.
//!
//! Whoever reads the audio-frame channel may install an
//! [`InlineAudioConsumer`] that runs the same work on the owner. Frames then
//! go from the pacing queue straight into it, under the lease lock the sink
//! already takes per delivery, instead of crossing the channel and waking the
//! consumer's task; the consumer's own deadlines join the owner's single
//! timer. The channel stays registered and the consumer's task stays alive,
//! so demotion, callback replacement and teardown fall back to exactly the v1
//! route.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use rvoip_rtp_core::executor::{
    inline_decode_enabled, record_inline_fallback, InlineFallbackReason, InlineMediaSink,
    InlineRtpPacket, InlineSinkVerdict,
};
use rvoip_rtp_core::RtpSession;

use super::codec_runtime::DialogCodecRuntime;
use super::source_timeline::SourceTimeline;
use super::{
    concealment_sample_count, record_transient_allocation, DecodedPlayoutQueue,
    MediaSessionController, RtpDeliveryState, RtpPlayoutBuffer, RtpPlayoutItem,
    RTP_REORDER_MAX_PACKETS,
};
use crate::diagnostics;
use crate::pcm_pool;
use crate::types::{AudioFrame, DialogId, MediaDirection};

static FUSED_CONSUMERS: AtomicU64 = AtomicU64::new(0);
static FUSED_FRAMES: AtomicU64 = AtomicU64::new(0);

/// How many fused frames a sink counts locally before publishing them.
const FLUSH_EVERY: u64 = 64;

/// Inline receive totals of the media layer for this process since start.
/// Each field is individually atomic; the set is not a transaction. The
/// packet-level totals (inline packets, sinks, fallbacks) are in
/// `rvoip_rtp_core::executor::inline_executor_counters`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct InlineRxCounters {
    /// Decoded-audio consumers fused onto a socket owner.
    pub fused_consumers: u64,
    /// Decoded frames handed to a fused consumer on the owner instead of the
    /// audio-frame channel. Published in batches per sink.
    pub fused_frames: u64,
}

/// The media layer's inline receive totals since start.
pub fn inline_rx_counters() -> InlineRxCounters {
    InlineRxCounters {
        fused_consumers: FUSED_CONSUMERS.load(Ordering::Relaxed),
        fused_frames: FUSED_FRAMES.load(Ordering::Relaxed),
    }
}

/// Fused frames counted by one sink, published in batches.
#[derive(Default)]
struct FusedFrameCounters {
    pending: u64,
}

impl FusedFrameCounters {
    fn frame(&mut self) {
        self.pending += 1;
        if self.pending >= FLUSH_EVERY {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending > 0 {
            FUSED_FRAMES.fetch_add(self.pending, Ordering::Relaxed);
            self.pending = 0;
        }
    }
}

impl Drop for FusedFrameCounters {
    fn drop(&mut self) {
        self.flush();
    }
}

/// A decoded-audio consumer fused onto the socket owner.
///
/// It replaces, for one dialog, the task that would otherwise read the
/// audio-frame callback channel: frames the inline sink would `try_send` to
/// that channel are handed to [`Self::on_frame`] instead, on the owner, and
/// the consumer's own timing joins the owner's single timer through
/// [`Self::next_deadline`]. Every call runs under the dialog's lease lock
/// inside the socket receive loop, so implementations must not block or
/// await.
pub trait InlineAudioConsumer: Send + 'static {
    /// Called once, under the lease lock, when the consumer is fused and
    /// before any frame reaches it.
    fn on_installed(&mut self, now: Instant);

    /// One decoded, paced frame that v1 would have sent on the channel.
    fn on_frame(&mut self, frame: AudioFrame, now: Instant) -> InlineAudioDelivery;

    /// The earliest instant the consumer needs service without a new frame.
    fn next_deadline(&self) -> Option<Instant>;

    /// The deadline from [`Self::next_deadline`] has passed.
    fn on_deadline(&mut self, now: Instant) -> InlineAudioDelivery;

    /// The consumer leaves the owner while the call goes on (demotion): emit
    /// whatever is due now. Later frames reach the consumer's own task
    /// through the channel.
    fn on_detach(&mut self, now: Instant);
}

/// What a fused consumer did with a frame or deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InlineAudioDelivery {
    /// Taken (v1: the channel accepted the frame).
    Accepted,
    /// The consumer is backed up and dropped the frame (v1: channel full).
    Full,
    /// The consumer stopped. It is detached; later frames take the channel,
    /// exactly where v1 frames would go after the consumer task ended.
    Closed,
}

/// Keeps a fused consumer installed. Dropping it (the consumer's task ended)
/// detaches the consumer, if it is still the installed one, and drops it.
#[must_use = "dropping the guard detaches the fused consumer"]
pub struct InlineAudioConsumerGuard {
    lease: Weak<DialogInlineLease>,
    id: u64,
}

impl std::fmt::Debug for InlineAudioConsumerGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InlineAudioConsumerGuard")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl Drop for InlineAudioConsumerGuard {
    fn drop(&mut self) {
        let Some(lease) = self.lease.upgrade() else {
            return;
        };
        let detached = {
            let mut state = lease.state.lock();
            if state
                .consumer
                .as_ref()
                .is_some_and(|installed| installed.id == self.id)
            {
                lease.detach_consumer_locked(&mut state)
            } else {
                None
            }
        };
        drop(detached);
    }
}

struct FusedConsumer {
    id: u64,
    consumer: Box<dyn InlineAudioConsumer>,
}

static NEXT_CONSUMER_ID: AtomicU64 = AtomicU64::new(1);

struct LeaseState {
    codec: Option<Arc<DialogCodecRuntime>>,
    receive_enabled: bool,
    audio: Option<mpsc::Sender<AudioFrame>>,
    /// The fused consumer, bound to the `audio` channel it was installed for.
    consumer: Option<FusedConsumer>,
    /// No consumer may be fused any more (demoted or retired).
    consumer_refused: bool,
}

/// What a demoted sink hands to the v1 handler so the stream continues on
/// the same call timeline.
pub(super) struct V1Handoff {
    pub(super) source_timeline: SourceTimeline,
    pub(super) receive_codec: Option<Arc<DialogCodecRuntime>>,
}

/// Per-dialog control state the inline sink reads instead of the
/// controller's String-keyed maps. Written only at control-plane events.
pub(super) struct DialogInlineLease {
    state: parking_lot::Mutex<LeaseState>,
    /// Bumped, under `state`, whenever the codec generation changes. The sink
    /// compares it once per packet and locks only on a change.
    codec_generation: AtomicU64,
    /// Bumped, under `state`, whenever the fused consumer is installed or
    /// detached. The sink compares it before trusting its cached consumer
    /// deadline and locks only on a change.
    consumer_epoch: AtomicU64,
    demote: AtomicBool,
    inline_stopped: AtomicBool,
    retired: AtomicBool,
    /// Set once, by a demoted sink, before any packet reaches the v1 handler.
    handoff: parking_lot::Mutex<Option<V1Handoff>>,
    handoff_ready: AtomicBool,
}

impl DialogInlineLease {
    pub(super) fn new() -> Self {
        Self {
            state: parking_lot::Mutex::new(LeaseState {
                codec: None,
                receive_enabled: true,
                audio: None,
                consumer: None,
                consumer_refused: false,
            }),
            codec_generation: AtomicU64::new(0),
            consumer_epoch: AtomicU64::new(0),
            demote: AtomicBool::new(false),
            inline_stopped: AtomicBool::new(false),
            retired: AtomicBool::new(false),
            handoff: parking_lot::Mutex::new(None),
            handoff_ready: AtomicBool::new(false),
        }
    }

    /// Take the fused consumer out of the lease. The caller drops it after
    /// releasing the lock.
    fn detach_consumer_locked(&self, state: &mut LeaseState) -> Option<FusedConsumer> {
        let detached = state.consumer.take();
        if detached.is_some() {
            self.consumer_epoch.fetch_add(1, Ordering::Release);
        }
        detached
    }

    /// Re-read the dialog's control state from the controller maps under the
    /// lease lock. Every mutation of those maps is followed by a resync, so
    /// concurrent mutations converge on the maps' final state.
    fn resync(
        &self,
        dialog_id: &DialogId,
        codec_runtimes: &DashMap<DialogId, Arc<DialogCodecRuntime>>,
        media_directions: &DashMap<DialogId, MediaDirection>,
        audio_frame_callbacks: &DashMap<DialogId, mpsc::Sender<AudioFrame>>,
    ) {
        let mut state = self.state.lock();
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        let codec = codec_runtimes
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()));
        let changed = match (&state.codec, &codec) {
            (Some(previous), Some(next)) => !Arc::ptr_eq(previous, next),
            (None, None) => false,
            _ => true,
        };
        state.codec = codec;
        if changed {
            self.codec_generation.fetch_add(1, Ordering::Release);
        }
        state.receive_enabled = media_directions
            .get(dialog_id)
            .map(|direction| {
                matches!(
                    *direction,
                    MediaDirection::SendRecv | MediaDirection::RecvOnly
                )
            })
            .unwrap_or(true);
        let audio = audio_frame_callbacks
            .get(dialog_id)
            .map(|entry| entry.value().clone());
        // A fused consumer stands in for the reader of one channel. When the
        // callback is removed or replaced, v1 stops feeding that reader, so
        // the consumer leaves the owner (its task stays, idle, as v1's would).
        let same_channel = match (&state.audio, &audio) {
            (Some(current), Some(next)) => current.same_channel(next),
            _ => false,
        };
        let detached = if same_channel {
            None
        } else {
            self.detach_consumer_locked(&mut state)
        };
        state.audio = audio;
        drop(state);
        if detached.is_some() {
            record_inline_fallback(InlineFallbackReason::ConsumerDetached);
        }
    }

    /// The dialog's maps were cleared. Drop the callback sender now so the
    /// consumer observes closure exactly when v1 would.
    fn retire(&self) {
        let mut state = self.state.lock();
        self.retired.store(true, Ordering::Release);
        state.audio = None;
        state.receive_enabled = true;
        state.consumer_refused = true;
        if state.codec.take().is_some() {
            self.codec_generation.fetch_add(1, Ordering::Release);
        }
        // Teardown: the consumer goes with the channel, as v1's reader does.
        let detached = self.detach_consumer_locked(&mut state);
        drop(state);
        drop(detached);
        self.handoff.lock().take();
    }

    fn codec_snapshot(&self) -> (Option<Arc<DialogCodecRuntime>>, u64) {
        let state = self.state.lock();
        (
            state.codec.clone(),
            self.codec_generation.load(Ordering::Acquire),
        )
    }

    fn request_demote(&self) {
        self.demote.store(true, Ordering::Release);
    }

    fn demote_requested(&self) -> bool {
        self.demote.load(Ordering::Acquire)
    }

    fn mark_inline_stopped(&self) {
        self.inline_stopped.store(true, Ordering::Release);
    }

    /// The inline sink observed a closed audio channel: the v1 handler must
    /// leave its loop exactly as it would have on that delivery.
    pub(super) fn inline_stopped(&self) -> bool {
        self.inline_stopped.load(Ordering::Acquire)
    }

    fn store_v1_handoff(&self, handoff: V1Handoff) {
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        *self.handoff.lock() = Some(handoff);
        self.handoff_ready.store(true, Ordering::Release);
    }

    /// What a demoted sink left for the v1 handler, once. One Acquire load
    /// while there is nothing to take.
    pub(super) fn take_v1_handoff(&self) -> Option<V1Handoff> {
        if !self.handoff_ready.load(Ordering::Acquire) {
            return None;
        }
        if !self.handoff_ready.swap(false, Ordering::AcqRel) {
            return None;
        }
        self.handoff.lock().take()
    }

    #[cfg(test)]
    pub(super) fn is_retired(&self) -> bool {
        self.retired.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn has_audio_sender(&self) -> bool {
        self.state.lock().audio.is_some()
    }

    #[cfg(test)]
    pub(super) fn has_consumer(&self) -> bool {
        self.state.lock().consumer.is_some()
    }
}

/// Fuse `consumer` onto the owner of `lease`'s dialog.
///
/// Refused (the consumer is handed back and its task keeps reading the
/// channel) unless the dialog is still inline, no consumer is installed, and
/// the registered callback is `channel`. Frames already in `channel` are
/// drained through `backlog` into the consumer first, under the lease lock;
/// the owner delivers under the same lock, so no frame can slip into the
/// channel between the drain and the hand-over and order is kept.
fn install_consumer(
    lease: &Arc<DialogInlineLease>,
    channel: &mpsc::Sender<AudioFrame>,
    mut consumer: Box<dyn InlineAudioConsumer>,
    backlog: &mut dyn FnMut() -> Option<AudioFrame>,
) -> Result<InlineAudioConsumerGuard, Box<dyn InlineAudioConsumer>> {
    let mut state = lease.state.lock();
    let accepts = !state.consumer_refused
        && state.consumer.is_none()
        && !lease.retired.load(Ordering::Acquire)
        && !lease.demote_requested()
        && !lease.inline_stopped()
        && state
            .audio
            .as_ref()
            .is_some_and(|registered| registered.same_channel(channel));
    if !accepts {
        drop(state);
        record_inline_fallback(InlineFallbackReason::ConsumerRefused);
        return Err(consumer);
    }
    let now = Instant::now();
    consumer.on_installed(now);
    while let Some(frame) = backlog() {
        if consumer.on_frame(frame, now) == InlineAudioDelivery::Closed {
            drop(state);
            record_inline_fallback(InlineFallbackReason::ConsumerRefused);
            return Err(consumer);
        }
    }
    let id = NEXT_CONSUMER_ID.fetch_add(1, Ordering::Relaxed);
    state.consumer = Some(FusedConsumer { id, consumer });
    lease.consumer_epoch.fetch_add(1, Ordering::Release);
    drop(state);
    FUSED_CONSUMERS.fetch_add(1, Ordering::Relaxed);
    Ok(InlineAudioConsumerGuard {
        lease: Arc::downgrade(lease),
        id,
    })
}

/// The sink's view of the fused consumer.
struct FusedView {
    /// The consumer's next deadline as of the sink's last call into it.
    deadline: Option<Instant>,
    /// Lease consumer epoch `deadline` was read at.
    epoch: u64,
    counters: FusedFrameCounters,
}

/// The v1 handler's optional receive-quality diagnostics state.
#[derive(Default)]
struct RxQuality {
    last_sequence_number: Option<u16>,
    last_rtp_timestamp: Option<u32>,
    last_rtp_arrival: Option<Instant>,
    jitter_ns: f64,
}

impl RxQuality {
    fn observe(&mut self, sequence: u16, timestamp: u32, arrival: Instant, clock_rate: u32) {
        let sequence_gap_packets = if let Some(previous) = self.last_sequence_number {
            let expected = previous.wrapping_add(1);
            if sequence == expected {
                0
            } else {
                let gap = sequence.wrapping_sub(expected);
                if gap < 32_768 {
                    u64::from(gap)
                } else {
                    0
                }
            }
        } else {
            0
        };
        let interarrival_gap = self
            .last_rtp_arrival
            .map(|previous| arrival.duration_since(previous));
        if let (Some(previous_arrival), Some(previous_timestamp)) =
            (self.last_rtp_arrival, self.last_rtp_timestamp)
        {
            let arrival_delta = arrival.duration_since(previous_arrival).as_nanos() as f64;
            let rtp_delta_samples = timestamp.wrapping_sub(previous_timestamp) as f64;
            let rtp_delta_ns = rtp_delta_samples * 1_000_000_000_f64 / f64::from(clock_rate);
            let transit_delta = (arrival_delta - rtp_delta_ns).abs();
            self.jitter_ns += (transit_delta - self.jitter_ns) / 16.0;
        }
        diagnostics::record_audio_rx_packet(
            sequence_gap_packets,
            interarrival_gap,
            Some(Duration::from_nanos(
                self.jitter_ns.max(0.0).min(u64::MAX as f64) as u64,
            )),
        );
        self.last_sequence_number = Some(sequence);
        self.last_rtp_timestamp = Some(timestamp);
        self.last_rtp_arrival = Some(arrival);
    }
}

/// The v1 RTP event handler's per-dialog state machine, run inline on the
/// socket owner.
pub(super) struct InlineAudioSink {
    dialog_id: DialogId,
    lease: Arc<DialogInlineLease>,
    /// The codec generation in the lease, refreshed only after it changed
    /// (v1 reads it from the controller map on every packet and loop turn).
    codec: Option<Arc<DialogCodecRuntime>>,
    codec_generation: u64,
    /// The generation the last media packet was accepted under (v1's
    /// `receive_codec`): a change resets the source timeline and the reorder
    /// window, because buffered packets belong to the previous codec.
    receive_codec: Option<Arc<DialogCodecRuntime>>,
    source_timeline: SourceTimeline,
    /// SSRC the quality diagnostics state belongs to (v1's `metrics_ssrc`).
    metrics_ssrc: Option<u32>,
    rtp_playout: RtpPlayoutBuffer,
    decoded_playout: DecodedPlayoutQueue,
    delivery: RtpDeliveryState,
    quality: RxQuality,
    rtp_count: u64,
    collect_audio_quality: bool,
    skip_audio_frame_delivery: bool,
    /// The audio channel closed: v1 leaves its loop here.
    stopped: bool,
    fused: FusedView,
    /// Reused for the reorder buffer's released items (v1 builds a `Vec` per
    /// release; same items, same order).
    playout_items: Vec<RtpPlayoutItem>,
}

impl InlineAudioSink {
    pub(super) fn new(
        dialog_id: DialogId,
        lease: Arc<DialogInlineLease>,
        collect_audio_quality: bool,
        skip_audio_frame_delivery: bool,
    ) -> Self {
        let (codec, codec_generation) = lease.codec_snapshot();
        let consumer_epoch = lease.consumer_epoch.load(Ordering::Acquire);
        Self {
            dialog_id,
            lease,
            codec,
            codec_generation,
            receive_codec: None,
            source_timeline: SourceTimeline::default(),
            metrics_ssrc: None,
            rtp_playout: RtpPlayoutBuffer::new(),
            decoded_playout: DecodedPlayoutQueue::new(),
            delivery: RtpDeliveryState::default(),
            quality: RxQuality::default(),
            rtp_count: 0,
            collect_audio_quality,
            skip_audio_frame_delivery,
            stopped: false,
            fused: FusedView {
                deadline: None,
                epoch: consumer_epoch,
                counters: FusedFrameCounters::default(),
            },
            playout_items: Vec::new(),
        }
    }

    /// One Acquire load per call; locks only after a codec change.
    fn refresh_codec(&mut self) {
        if self.lease.codec_generation.load(Ordering::Acquire) != self.codec_generation {
            let (codec, generation) = self.lease.codec_snapshot();
            self.codec = codec;
            self.codec_generation = generation;
        }
    }

    /// Demotion checks shared by packets and deadlines.
    fn must_demote(&mut self) -> Option<InlineFallbackReason> {
        if self.lease.demote_requested() {
            return Some(InlineFallbackReason::Demoted);
        }
        self.refresh_codec();
        if self
            .codec
            .as_ref()
            .is_some_and(|codec| !codec.decodes_inline())
        {
            return Some(InlineFallbackReason::CodecChanged);
        }
        None
    }

    /// The v1 `PacketReceived` arm.
    fn packet_arrived(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) {
        self.rtp_count += 1;
        let Some(codec) = self.codec.as_ref() else {
            debug!("No codec runtime for dialog {}", self.dialog_id);
            return;
        };
        if packet.payload_type != codec.format.payload_type {
            debug!(
                "Ignoring unnegotiated payload type {} for dialog {} (expected {})",
                packet.payload_type, self.dialog_id, codec.format.payload_type
            );
            return;
        }
        match self.receive_codec.as_ref() {
            Some(previous) if Arc::ptr_eq(previous, codec) => {}
            previous => {
                if previous.is_some() {
                    // Packets buffered under the previous codec cannot be
                    // decoded by this one. Frames already decoded are valid
                    // audio and still play.
                    self.source_timeline = SourceTimeline::default();
                    self.rtp_playout = RtpPlayoutBuffer::new();
                    self.metrics_ssrc = None;
                }
                self.receive_codec = Some(Arc::clone(codec));
            }
        }
        let arrived_at = now;
        if !self.source_timeline.should_buffer(
            packet.ssrc,
            packet.sequence_number,
            packet.timestamp,
            arrived_at,
            codec.format.clock_rate,
        ) {
            return;
        }
        if self.metrics_ssrc != Some(packet.ssrc) {
            self.quality = RxQuality::default();
            self.metrics_ssrc = Some(packet.ssrc);
        }
        let rtp_count = self.rtp_count;
        if rtp_count % 10 == 0
            || rtp_count == 100
            || rtp_count == 101
            || rtp_count > 100 && rtp_count < 110
        {
            trace!(
                "📦 Received RTP packet #{} for dialog {}: PT={}, seq={}, ts={}, payload_size={}",
                rtp_count,
                self.dialog_id,
                packet.payload_type,
                packet.sequence_number,
                packet.timestamp,
                packet.payload.len()
            );
        }
        if self.collect_audio_quality {
            self.quality.observe(
                packet.sequence_number,
                packet.timestamp,
                now,
                codec.format.clock_rate,
            );
        }

        if self
            .rtp_playout
            .releases_immediately(packet.ssrc, packet.sequence_number)
        {
            // The in-order packet never leaves the owner's receive buffer.
            self.rtp_playout
                .release_in_order(packet.sequence_number, packet.timestamp, arrived_at);
            if let Some(frame) = decode_inline(
                codec,
                &mut self.source_timeline,
                packet.ssrc,
                packet.sequence_number,
                packet.timestamp,
                packet.payload,
                arrived_at,
                &self.dialog_id,
            ) {
                push_decoded(
                    &mut self.decoded_playout,
                    frame,
                    arrived_at,
                    &self.dialog_id,
                );
            }
        } else {
            self.rtp_playout
                .push_into(packet.to_packet(), arrived_at, &mut self.playout_items);
            enqueue_inline(
                self.playout_items.drain(..),
                codec,
                &self.dialog_id,
                &mut self.decoded_playout,
                &mut self.source_timeline,
                arrived_at,
            );
        }
        // A frame that is already due leaves on arrival.
        if deliver_due_inline(
            arrived_at,
            &mut self.decoded_playout,
            &self.dialog_id,
            &self.lease,
            self.collect_audio_quality,
            self.skip_audio_frame_delivery,
            &mut self.delivery,
            &mut self.fused,
        ) {
            self.stop();
        }
    }

    /// One turn of the v1 handler's loop top.
    fn service(&mut self, now: Instant) {
        self.rtp_playout.tick_into(now, &mut self.playout_items);
        if !self.playout_items.is_empty() {
            match self.codec.as_ref() {
                Some(codec) => enqueue_inline(
                    self.playout_items.drain(..),
                    codec,
                    &self.dialog_id,
                    &mut self.decoded_playout,
                    &mut self.source_timeline,
                    now,
                ),
                // v1 drops the released items without a codec.
                None => self.playout_items.clear(),
            }
        }
        if deliver_due_inline(
            now,
            &mut self.decoded_playout,
            &self.dialog_id,
            &self.lease,
            self.collect_audio_quality,
            self.skip_audio_frame_delivery,
            &mut self.delivery,
            &mut self.fused,
        ) {
            self.stop();
        }
    }

    /// The fused consumer's deadline, refreshed only after the lease's
    /// consumer changed (one Acquire load otherwise).
    fn consumer_deadline(&mut self) -> Option<Instant> {
        if self.lease.consumer_epoch.load(Ordering::Acquire) != self.fused.epoch {
            let state = self.lease.state.lock();
            self.fused.epoch = self.lease.consumer_epoch.load(Ordering::Acquire);
            self.fused.deadline = state
                .consumer
                .as_ref()
                .and_then(|installed| installed.consumer.next_deadline());
        }
        self.fused.deadline
    }

    /// Serve the fused consumer's own deadline (its playout clock), the work
    /// its task's timer did in v1. Locks only when that deadline is due.
    fn service_consumer(&mut self, now: Instant) {
        if !self.fused.deadline.is_some_and(|deadline| deadline <= now) {
            return;
        }
        let mut guard = self.lease.state.lock();
        let state: &mut LeaseState = &mut guard;
        let closed = match state.consumer.as_mut() {
            Some(installed) => {
                let delivery = installed.consumer.on_deadline(now);
                self.fused.deadline = installed.consumer.next_deadline();
                delivery == InlineAudioDelivery::Closed
            }
            None => {
                self.fused.deadline = None;
                false
            }
        };
        let detached = if closed {
            self.fused.deadline = None;
            self.lease.detach_consumer_locked(state)
        } else {
            None
        };
        self.fused.epoch = self.lease.consumer_epoch.load(Ordering::Acquire);
        drop(guard);
        if detached.is_some() {
            record_inline_fallback(InlineFallbackReason::ConsumerDetached);
            drop(detached);
        }
    }

    fn stop(&mut self) {
        self.stopped = true;
        self.lease.mark_inline_stopped();
        // v1 drops this state when its handler task returns.
        self.rtp_playout = RtpPlayoutBuffer::new();
        for frame in self.decoded_playout.pending.drain(..) {
            pcm_pool::recycle_pcm_buffer(frame.samples);
        }
        self.decoded_playout = DecodedPlayoutQueue::new();
    }

    /// Leave the inline path: paced frames go to the consumer now (the v1
    /// handler starts with an empty pacing queue), held RTP is handed back
    /// for republication, oldest first, and the source timeline goes to the
    /// v1 handler, which continues it.
    fn demote(&mut self, reason: InlineFallbackReason, now: Instant) -> InlineSinkVerdict {
        info!(
            dialog_id = %self.dialog_id,
            reason = reason.as_str(),
            "media executor v2 demoting dialog to the v1 receive handler"
        );
        let held = if self.stopped {
            Vec::new()
        } else {
            while let Some(frame) = self.decoded_playout.pending.pop_front() {
                if deliver_frame_inline(
                    frame,
                    &self.dialog_id,
                    &self.lease,
                    self.collect_audio_quality,
                    self.skip_audio_frame_delivery,
                    &mut self.delivery,
                    &mut self.fused,
                    now,
                ) {
                    break;
                }
            }
            self.decoded_playout = DecodedPlayoutQueue::new();
            self.rtp_playout.take_pending_in_order()
        };
        self.lease.store_v1_handoff(V1Handoff {
            source_timeline: std::mem::take(&mut self.source_timeline),
            receive_codec: self.receive_codec.take(),
        });
        // The v1 handler delivers to the channel from here on, so a fused
        // consumer goes back to its own task after emitting what is due.
        let detached = {
            let mut state = self.lease.state.lock();
            state.consumer_refused = true;
            self.lease.detach_consumer_locked(&mut state)
        };
        if let Some(mut detached) = detached {
            detached.consumer.on_detach(now);
            record_inline_fallback(InlineFallbackReason::ConsumerDetached);
        }
        self.fused.deadline = None;
        self.fused.counters.flush();
        self.stopped = true;
        InlineSinkVerdict::Demote { reason, held }
    }
}

impl Drop for InlineAudioSink {
    /// Only a live sink feeds a fused consumer. Once it is gone (owner task
    /// ended, demoted, or refused by the session after media had already
    /// been broadcast) the consumer goes back to its own task and no other
    /// may be fused.
    fn drop(&mut self) {
        let detached = {
            let mut state = self.lease.state.lock();
            state.consumer_refused = true;
            self.lease.detach_consumer_locked(&mut state)
        };
        drop(detached);
    }
}

impl InlineMediaSink for InlineAudioSink {
    fn on_packet(&mut self, packet: &InlineRtpPacket<'_>, now: Instant) -> InlineSinkVerdict {
        if let Some(reason) = self.must_demote() {
            return self.demote(reason, now);
        }
        if self.stopped {
            return InlineSinkVerdict::Continue;
        }
        self.packet_arrived(packet, now);
        if !self.stopped {
            self.service(now);
            self.service_consumer(now);
        }
        InlineSinkVerdict::Continue
    }

    fn next_deadline(&mut self, now: Instant) -> Option<Instant> {
        if self.stopped {
            return None;
        }
        let own = match (
            self.rtp_playout.next_deadline(now),
            self.decoded_playout.next_deadline(),
        ) {
            (Some(reorder), Some(audio)) => Some(reorder.min(audio)),
            (reorder, audio) => reorder.or(audio),
        };
        match (own, self.consumer_deadline()) {
            (Some(own), Some(consumer)) => Some(own.min(consumer)),
            (own, consumer) => own.or(consumer),
        }
    }

    fn on_deadline(&mut self, now: Instant) -> InlineSinkVerdict {
        if let Some(reason) = self.must_demote() {
            return self.demote(reason, now);
        }
        if !self.stopped {
            self.service(now);
            self.service_consumer(now);
        }
        InlineSinkVerdict::Continue
    }
}

/// The packet half of `enqueue_rtp_playout_items` for stateless G.711: the
/// same source-timeline admission, decode at the mapped timestamp, and
/// commit of the decoded duration.
#[allow(clippy::too_many_arguments)]
fn decode_inline(
    codec: &DialogCodecRuntime,
    timeline: &mut SourceTimeline,
    ssrc: u32,
    sequence_number: u16,
    timestamp: u32,
    payload: &[u8],
    arrived_at: Instant,
    dialog_id: &DialogId,
) -> Option<AudioFrame> {
    let accepted = timeline.prepare(
        ssrc,
        sequence_number,
        timestamp,
        arrived_at,
        codec.format.clock_rate,
    )?;
    // `None`: unreachable, a generation that is not G.711 demotes first.
    match codec.decode_g711_now(payload, accepted.timestamp)? {
        Ok(frame) => {
            let ticks = (frame.samples_per_channel() as u64 * u64::from(codec.format.clock_rate)
                / u64::from(frame.sample_rate.max(1))) as u32;
            timeline.commit(accepted, arrived_at, ticks);
            Some(frame)
        }
        Err(error) => {
            warn!(
                "Failed to decode {} for dialog {}: {}",
                codec.format.name, dialog_id, error
            );
            None
        }
    }
}

/// Synchronous twin of `enqueue_rtp_playout_items` for stateless G.711.
fn enqueue_inline(
    items: impl Iterator<Item = RtpPlayoutItem>,
    codec: &DialogCodecRuntime,
    dialog_id: &DialogId,
    decoded_playout: &mut DecodedPlayoutQueue,
    timeline: &mut SourceTimeline,
    arrived_at: Instant,
) {
    for item in items {
        let audio_frame = match item {
            RtpPlayoutItem::Packet(packet) => {
                let Some(frame) = decode_inline(
                    codec,
                    timeline,
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    packet.header.timestamp,
                    &packet.payload,
                    arrived_at,
                    dialog_id,
                ) else {
                    continue;
                };
                frame
            }
            RtpPlayoutItem::Gap {
                sequence_number,
                timestamp,
                samples_per_channel,
            } => {
                let Some(sample_count) = concealment_sample_count(
                    samples_per_channel,
                    codec.format.clock_rate,
                    codec.format.channels,
                ) else {
                    warn!(
                        %dialog_id,
                        sequence_number,
                        timestamp,
                        samples_per_channel,
                        clock_rate = codec.format.clock_rate,
                        channels = codec.format.channels,
                        "refusing an unbounded RTP concealment frame; received media continues"
                    );
                    continue;
                };
                warn!(
                    %dialog_id,
                    sequence_number,
                    timestamp,
                    samples_per_channel,
                    "concealing confirmed missing RTP audio packet"
                );
                AudioFrame::new(
                    pcm_pool::take_zeroed_pcm_buffer(sample_count),
                    codec.format.clock_rate,
                    codec.format.channels,
                    timeline.map_gap(timestamp),
                )
            }
        };
        push_decoded(decoded_playout, audio_frame, arrived_at, dialog_id);
    }
}

fn push_decoded(
    decoded_playout: &mut DecodedPlayoutQueue,
    frame: AudioFrame,
    arrived_at: Instant,
    dialog_id: &DialogId,
) {
    if !decoded_playout.push(frame, arrived_at) {
        decoded_playout.dropped_frames += 1;
        if decoded_playout.dropped_frames == 1 {
            warn!(
                %dialog_id,
                capacity_frames = RTP_REORDER_MAX_PACKETS,
                "decoded playout queue overflow; dropping frames until playout catches up"
            );
        }
    }
}

/// `deliver_due_audio_frame` with the lease instead of the String-keyed maps.
#[allow(clippy::too_many_arguments)]
fn deliver_due_inline(
    now: Instant,
    decoded_playout: &mut DecodedPlayoutQueue,
    dialog_id: &DialogId,
    lease: &DialogInlineLease,
    collect_audio_quality: bool,
    skip_audio_frame_delivery: bool,
    state: &mut RtpDeliveryState,
    fused: &mut FusedView,
) -> bool {
    let Some(audio_frame) = decoded_playout.pop_due(now) else {
        return false;
    };
    deliver_frame_inline(
        audio_frame,
        dialog_id,
        lease,
        collect_audio_quality,
        skip_audio_frame_delivery,
        state,
        fused,
        now,
    )
}

/// Outcome of one delivery, fused or over the channel.
enum Delivered {
    Sent,
    Full,
    Closed,
}

/// Returns true when the audio channel is closed (v1 leaves its loop).
///
/// With a fused consumer installed the frame goes to it, on the owner, in
/// place of the channel's `try_send`; every gate before that point (media
/// direction, quality diagnostics, the perf skip flag, a registered callback)
/// is unchanged. A frame that is not delivered gives its PCM back to the
/// pool.
#[allow(clippy::too_many_arguments)]
fn deliver_frame_inline(
    audio_frame: AudioFrame,
    dialog_id: &DialogId,
    lease: &DialogInlineLease,
    collect_audio_quality: bool,
    skip_audio_frame_delivery: bool,
    state: &mut RtpDeliveryState,
    fused: &mut FusedView,
    now: Instant,
) -> bool {
    let decoded_len = audio_frame.samples.len();
    let mut guard = lease.state.lock();
    if !guard.receive_enabled {
        drop(guard);
        debug!(
            "Dropping received RTP audio for dialog {} due to media direction",
            dialog_id
        );
        pcm_pool::recycle_pcm_buffer(audio_frame.samples);
        return false;
    }
    if collect_audio_quality {
        diagnostics::record_audio_rx_decoded_frame();
    }
    if skip_audio_frame_delivery {
        drop(guard);
        state.decoded_audio_frame_count += 1;
        record_transient_allocation(
            "media_core.audio.rx.decoded_without_frame_delivery",
            decoded_len * std::mem::size_of::<i16>(),
        );
        pcm_pool::recycle_pcm_buffer(audio_frame.samples);
        return false;
    }
    if guard.audio.is_none() {
        drop(guard);
        if !state.logged_missing_audio_callback {
            info!(
                "⚠️ No audio frame callback registered yet for dialog {}",
                dialog_id
            );
            state.logged_missing_audio_callback = true;
        }
        pcm_pool::recycle_pcm_buffer(audio_frame.samples);
        return false;
    }
    state.decoded_audio_frame_count += 1;
    record_transient_allocation(
        "media_core.audio.rx.audio_frame.samples_vec",
        audio_frame.samples.capacity() * std::mem::size_of::<i16>(),
    );
    let lease_state: &mut LeaseState = &mut guard;
    let (sent, consumer_closed) = match lease_state.consumer.as_mut() {
        Some(installed) => {
            let delivery = installed.consumer.on_frame(audio_frame, now);
            fused.deadline = installed.consumer.next_deadline();
            match delivery {
                InlineAudioDelivery::Accepted => {
                    fused.counters.frame();
                    (Delivered::Sent, false)
                }
                InlineAudioDelivery::Full => (Delivered::Full, false),
                // The consumer took this frame and stopped; later frames use
                // the channel, as they would after v1's reader returned.
                InlineAudioDelivery::Closed => (Delivered::Sent, true),
            }
        }
        None => {
            fused.deadline = None;
            let sent = match lease_state.audio.as_ref() {
                Some(sender) => match sender.try_send(audio_frame) {
                    Ok(()) => Delivered::Sent,
                    Err(mpsc::error::TrySendError::Full(refused)) => {
                        pcm_pool::recycle_pcm_buffer(refused.samples);
                        Delivered::Full
                    }
                    Err(mpsc::error::TrySendError::Closed(refused)) => {
                        pcm_pool::recycle_pcm_buffer(refused.samples);
                        Delivered::Closed
                    }
                },
                None => unreachable!("checked above while holding the lease lock"),
            };
            (sent, false)
        }
    };
    let detached = if consumer_closed {
        fused.deadline = None;
        lease.detach_consumer_locked(lease_state)
    } else {
        None
    };
    fused.epoch = lease.consumer_epoch.load(Ordering::Acquire);
    drop(guard);
    if detached.is_some() {
        record_inline_fallback(InlineFallbackReason::ConsumerDetached);
        drop(detached);
    }
    match sent {
        Delivered::Sent => {
            if collect_audio_quality {
                let delivered_at = Instant::now();
                let delivery_gap = state
                    .last_delivered_at
                    .map(|previous| delivered_at.duration_since(previous));
                diagnostics::record_audio_rx_delivered_frame(delivery_gap);
                state.last_delivered_at = Some(delivered_at);
            }
            if state.decoded_audio_frame_count % 10 == 0
                || state.decoded_audio_frame_count == 100
                || state.decoded_audio_frame_count == 101
            {
                info!(
                    "✅ Sent decoded audio frame #{} to callback for dialog {}",
                    state.decoded_audio_frame_count, dialog_id
                );
            }
            false
        }
        Delivered::Full => {
            warn!(
                "Audio frame buffer full for dialog {} at frame #{}, dropping frame",
                dialog_id, state.decoded_audio_frame_count
            );
            false
        }
        Delivered::Closed => {
            error!(
                "❌ Audio frame channel closed for dialog {} at frame #{} - THIS IS THE 100-FRAME BUG!",
                dialog_id, state.decoded_audio_frame_count
            );
            true
        }
    }
}

impl MediaSessionController {
    /// Media executor v2: offer the inline sink for a new dialog on its RTP
    /// session, before media can flow. Returns the lease the controller must
    /// publish once the dialog's maps are populated.
    pub(super) fn offer_inline_rx(
        &self,
        dialog_id: &DialogId,
        rtp_session: &RtpSession,
        codec: &DialogCodecRuntime,
    ) -> Option<Arc<DialogInlineLease>> {
        if !inline_decode_enabled() || !rtp_session.inline_rx_enabled() {
            return None;
        }
        if !codec.decodes_inline() {
            record_inline_fallback(InlineFallbackReason::Codec);
            return None;
        }
        let lease = Arc::new(DialogInlineLease::new());
        let sink = InlineAudioSink::new(
            dialog_id.clone(),
            Arc::clone(&lease),
            diagnostics::audio_quality_enabled(),
            super::perf_skip_audio_frame_delivery(),
        );
        match rtp_session.install_inline_media_sink(Box::new(sink)) {
            Ok(()) => Some(lease),
            Err(_refused) => {
                record_inline_fallback(InlineFallbackReason::Refused);
                None
            }
        }
    }

    /// Make the lease reachable by control-plane updates and load its state.
    pub(super) fn publish_inline_lease(
        &self,
        dialog_id: &DialogId,
        lease: &Arc<DialogInlineLease>,
    ) {
        self.inline_leases
            .insert(dialog_id.clone(), Arc::clone(lease));
        lease.resync(
            dialog_id,
            &self.codec_runtimes,
            &self.media_directions,
            &self.audio_frame_callbacks,
        );
    }

    /// Call after any change to `codec_runtimes`, `media_directions` or
    /// `audio_frame_callbacks` for this dialog. No-op without a lease.
    pub(super) fn resync_inline_lease(&self, dialog_id: &DialogId) {
        let lease = self
            .inline_leases
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()));
        if let Some(lease) = lease {
            lease.resync(
                dialog_id,
                &self.codec_runtimes,
                &self.media_directions,
                &self.audio_frame_callbacks,
            );
        }
    }

    pub(super) fn retire_inline_lease(&self, dialog_id: &DialogId) {
        if let Some((_, lease)) = self.inline_leases.remove(dialog_id) {
            lease.retire();
        }
    }

    /// Fuse the reader of `dialog_id`'s audio callback channel onto the
    /// dialog's socket owner (media executor v2).
    ///
    /// `channel` must be the sender currently registered with
    /// [`Self::set_audio_frame_callback`]; frames already queued in it are
    /// drained through `backlog` (the reader's `try_recv`) into the consumer
    /// first, so order is kept. On success the owner hands every frame it
    /// would have sent on `channel` to the consumer until the returned guard
    /// is dropped, the callback is removed or replaced, the dialog is
    /// demoted, the consumer reports `Closed`, or the dialog is torn down; in
    /// each case later frames take the channel again.
    ///
    /// Refused (the consumer is handed back, and the reader keeps reading the
    /// channel) when the dialog does not decode inline (v1, inline decode
    /// switched off, a codec other than G.711), or once the dialog was
    /// stopped, demoted or rebound.
    pub fn install_inline_audio_consumer(
        &self,
        dialog_id: &DialogId,
        channel: &mpsc::Sender<AudioFrame>,
        consumer: Box<dyn InlineAudioConsumer>,
        backlog: &mut dyn FnMut() -> Option<AudioFrame>,
    ) -> Result<InlineAudioConsumerGuard, Box<dyn InlineAudioConsumer>> {
        let lease = self
            .inline_leases
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()));
        match lease {
            Some(lease) => install_consumer(&lease, channel, consumer, backlog),
            // Not inline: the reason was counted when the sink was offered.
            None => Err(consumer),
        }
    }

    /// A consumer of `RtpSessionEvent::PacketReceived` is attaching to this
    /// dialog's RTP session (the in-tree bridge does this itself): move the
    /// dialog off the inline receive path before its next packet. Packets
    /// the inline sink still holds for reordering, and every later packet,
    /// are then published as `PacketReceived` again. No-op for a dialog that
    /// does not decode inline.
    pub fn request_inline_demotion(&self, dialog_id: &DialogId) {
        let lease = self
            .inline_leases
            .get(dialog_id)
            .map(|entry| Arc::clone(entry.value()));
        if let Some(lease) = lease {
            lease.request_demote();
        }
    }
}

#[cfg(test)]
mod tests {
    //! v1/v2 equivalence for the media layer's half of executor v2: the same
    //! packet script is fed to the v1 RTP event handler (real time, through
    //! its broadcast queue) and to the inline sink (virtual time, the way the
    //! socket owner drives it). Both must hand the consumer identical frames:
    //! timestamps (including the source timeline's epoch normalisation),
    //! samples, concealment, reorder and duplicate handling.
    use super::*;
    use crate::relay::controller::codec_runtime::resolve_codec;
    use crate::relay::controller::MediaConfig;
    use bytes::Bytes;
    use rvoip_rtp_core::executor::{
        inline_executor_counters, override_media_executor_config_on_this_thread,
        MediaExecutorConfig,
    };
    use rvoip_rtp_core::session::RtpSessionEvent;
    use rvoip_rtp_core::RtpPacket;
    use std::collections::HashMap;
    use std::net::SocketAddr;

    const SSRC: u32 = 0x0e40_0001;
    const OTHER_SSRC: u32 = 0x0e40_0002;

    #[derive(Clone, Copy, Debug)]
    struct Rtp {
        pt: u8,
        seq: u16,
        ts: u32,
        ssrc: u32,
    }

    #[derive(Clone, Copy, Debug)]
    enum Step {
        /// PCMU, `SSRC`, timestamp `seq * 160`.
        Packet(u16),
        /// Any packet.
        Raw(Rtp),
        WaitMs(u64),
        /// A re-INVITE committed a new codec generation (same codec).
        NewCodec,
    }
    use Step::{NewCodec, Packet, Raw, WaitMs};

    impl Step {
        fn rtp(self) -> Option<Rtp> {
            match self {
                Packet(seq) => Some(Rtp {
                    pt: 0,
                    seq,
                    ts: timestamp(seq),
                    ssrc: SSRC,
                }),
                Raw(rtp) => Some(rtp),
                WaitMs(_) | NewCodec => None,
            }
        }
    }

    type Frames = Vec<(u32, Vec<i16>)>;

    fn timestamp(seq: u16) -> u32 {
        u32::from(seq).wrapping_mul(160)
    }

    /// A second source with its own, unrelated RTP clock.
    fn other_source(seq: u16) -> Step {
        Raw(Rtp {
            pt: 0,
            seq,
            ts: 7_000_000_u32.wrapping_add(u32::from(seq) * 160),
            ssrc: OTHER_SSRC,
        })
    }

    fn payload_byte(seq: u16) -> u8 {
        (seq as u8) ^ 0x55
    }

    fn pcmu() -> Arc<DialogCodecRuntime> {
        let format = resolve_codec(&MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        })
        .unwrap();
        Arc::new(DialogCodecRuntime::new(format).unwrap())
    }

    fn pcmu_config() -> MediaConfig {
        MediaConfig {
            local_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: HashMap::new(),
        }
    }

    /// The controller maps a lease mirrors, kept so a test can change them.
    struct LeaseMaps {
        dialog: DialogId,
        codecs: DashMap<DialogId, Arc<DialogCodecRuntime>>,
        directions: DashMap<DialogId, MediaDirection>,
        callbacks: DashMap<DialogId, mpsc::Sender<AudioFrame>>,
        lease: Arc<DialogInlineLease>,
    }

    impl LeaseMaps {
        fn new(
            dialog: &DialogId,
            direction: Option<MediaDirection>,
            audio: mpsc::Sender<AudioFrame>,
        ) -> Self {
            let maps = Self {
                dialog: dialog.clone(),
                codecs: DashMap::new(),
                directions: DashMap::new(),
                callbacks: DashMap::new(),
                lease: Arc::new(DialogInlineLease::new()),
            };
            maps.codecs.insert(dialog.clone(), pcmu());
            if let Some(direction) = direction {
                maps.directions.insert(dialog.clone(), direction);
            }
            maps.callbacks.insert(dialog.clone(), audio);
            maps.resync();
            maps
        }

        fn resync(&self) {
            self.lease.resync(
                &self.dialog,
                &self.codecs,
                &self.directions,
                &self.callbacks,
            );
        }
    }

    fn lease_with(
        dialog: &DialogId,
        direction: Option<MediaDirection>,
        audio: mpsc::Sender<AudioFrame>,
    ) -> Arc<DialogInlineLease> {
        Arc::clone(&LeaseMaps::new(dialog, direction, audio).lease)
    }

    /// Serve every owner deadline up to `target`, as the socket owner's
    /// timer would, and return the new virtual time.
    fn advance(sink: &mut InlineAudioSink, mut now: Instant, target: Instant) -> Instant {
        for _ in 0..10_000 {
            match sink.next_deadline(now) {
                Some(deadline) if deadline <= target => {
                    now = now.max(deadline);
                    assert!(matches!(sink.on_deadline(now), InlineSinkVerdict::Continue));
                }
                _ => return target,
            }
        }
        panic!("inline deadlines did not converge");
    }

    fn inline_rtp(rtp: Rtp, payload: &[u8]) -> InlineRtpPacket<'_> {
        InlineRtpPacket {
            payload_type: rtp.pt,
            sequence_number: rtp.seq,
            timestamp: rtp.ts,
            marker: false,
            ssrc: rtp.ssrc,
            padding_size: 0,
            payload,
            source: SocketAddr::from(([127, 0, 0, 1], 40_000)),
        }
    }

    fn inline_packet(seq: u16, payload: &[u8]) -> InlineRtpPacket<'_> {
        inline_rtp(Packet(seq).rtp().unwrap(), payload)
    }

    fn run_inline(script: &[Step]) -> Frames {
        let dialog = DialogId::new("inline-equivalence");
        let (audio_tx, mut audio_rx) = mpsc::channel(256);
        let maps = LeaseMaps::new(&dialog, None, audio_tx);
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&maps.lease), false, false);
        let mut now = Instant::now();
        let mut payload = [0u8; 160];
        for step in script {
            match *step {
                WaitMs(ms) => now = advance(&mut sink, now, now + Duration::from_millis(ms)),
                NewCodec => {
                    maps.codecs.insert(maps.dialog.clone(), pcmu());
                    maps.resync();
                }
                packet => {
                    let rtp = packet.rtp().expect("a packet step");
                    payload.fill(payload_byte(rtp.seq));
                    let verdict = sink.on_packet(&inline_rtp(rtp, &payload), now);
                    assert!(matches!(verdict, InlineSinkVerdict::Continue));
                    let _ = sink.next_deadline(now);
                }
            }
        }
        advance(&mut sink, now, now + Duration::from_secs(2));
        drop(sink);
        let mut frames = Vec::new();
        while let Ok(frame) = audio_rx.try_recv() {
            frames.push((frame.timestamp, frame.samples));
        }
        frames
    }

    async fn run_v1(script: &[Step]) -> Frames {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("v1-equivalence");
        let (audio_tx, mut audio_rx) = mpsc::channel(256);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .unwrap();
        controller.codec_runtimes.insert(dialog.clone(), pcmu());
        let (events, receiver) = tokio::sync::broadcast::channel(256);
        controller.spawn_rtp_event_handler(dialog.clone(), receiver, 0);
        for step in script {
            match *step {
                WaitMs(ms) => tokio::time::sleep(Duration::from_millis(ms)).await,
                NewCodec => {
                    controller.codec_runtimes.insert(dialog.clone(), pcmu());
                }
                packet => {
                    let rtp = packet.rtp().expect("a packet step");
                    events
                        .send(RtpSessionEvent::PacketReceived(
                            RtpPacket::new_with_payload(
                                rtp.pt,
                                rtp.seq,
                                rtp.ts,
                                rtp.ssrc,
                                Bytes::from(vec![payload_byte(rtp.seq); 160]),
                            ),
                        ))
                        .unwrap();
                }
            }
        }
        let mut frames = Vec::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(Duration::from_millis(400), audio_rx.recv()).await
        {
            frames.push((frame.timestamp, frame.samples));
        }
        drop(events);
        controller.cleanup_per_dialog_side_state(&dialog);
        frames
    }

    async fn assert_same_frames(script: &[Step], expected_timestamps: &[u32]) -> Frames {
        let v1 = run_v1(script).await;
        let v2 = run_inline(script);
        let timestamps: Vec<u32> = v2.iter().map(|(timestamp, _)| *timestamp).collect();
        assert_eq!(timestamps, expected_timestamps, "{script:?}");
        assert_eq!(v1, v2, "{script:?}");
        v2
    }

    /// Where the source timeline places a packet by the receive time it
    /// waited (a clock step across a held gap), the v1 handler runs in real
    /// time and the inline sink in virtual time, so v1 may place it later by
    /// its scheduling delay. Everything else must match exactly.
    async fn assert_same_frames_within(script: &[Step], expected_timestamps: &[u32]) -> Frames {
        const SLACK_TICKS: u32 = 80; // 10 ms at 8 kHz
        let v1 = run_v1(script).await;
        let v2 = run_inline(script);
        let timestamps: Vec<u32> = v2.iter().map(|(timestamp, _)| *timestamp).collect();
        assert_eq!(timestamps, expected_timestamps, "{script:?}");
        assert_eq!(v1.len(), v2.len(), "{script:?}");
        for ((v1_timestamp, v1_samples), (v2_timestamp, v2_samples)) in v1.iter().zip(&v2) {
            assert_eq!(v1_samples, v2_samples, "{script:?}");
            assert!(
                v1_timestamp.wrapping_sub(*v2_timestamp) <= SLACK_TICKS,
                "{script:?}: v1 {v1_timestamp} vs inline {v2_timestamp}"
            );
        }
        v2
    }

    fn is_silence(frames: &Frames, frame_timestamp: u32) -> bool {
        frames
            .iter()
            .find(|(timestamp, _)| *timestamp == frame_timestamp)
            .expect("frame present")
            .1
            .iter()
            .all(|sample| *sample == 0)
    }

    #[tokio::test]
    async fn in_order_stream_is_identical_and_decoded_from_the_owner_buffer() {
        let script: Vec<Step> = (1..=12).map(Packet).collect();
        let expected: Vec<u32> = (1..=12).map(timestamp).collect();
        assert_same_frames(&script, &expected).await;
    }

    #[tokio::test]
    async fn reordered_packets_are_released_in_order_identically() {
        let script = [
            Packet(1),
            Packet(2),
            Packet(4),
            Packet(3),
            Packet(5),
            Packet(7),
            Packet(6),
        ];
        let expected: Vec<u32> = (1..=7).map(timestamp).collect();
        assert_same_frames(&script, &expected).await;
    }

    #[tokio::test]
    async fn loss_is_concealed_after_the_same_hold_with_the_same_bounds() {
        // 3 is lost and concealed after the reorder hold; 9..=19 is a gap
        // beyond the concealment bound and is skipped without a frame.
        let script = [
            Packet(1),
            Packet(2),
            Packet(4),
            Packet(5),
            WaitMs(120),
            Packet(6),
            Packet(7),
            Packet(8),
            Packet(20),
            WaitMs(120),
            Packet(21),
        ];
        let expected = [1, 2, 3, 4, 5, 6, 7, 8, 20, 21].map(timestamp);
        let frames = assert_same_frames(&script, &expected).await;
        assert!(is_silence(&frames, timestamp(3)));
    }

    #[tokio::test]
    async fn consecutive_losses_are_concealed_after_one_hold_identically() {
        // Three lost packets after one hold: every one is concealed in the
        // same pass, then the held packets follow.
        let script = [
            Packet(1),
            Packet(2),
            Packet(6),
            Packet(7),
            WaitMs(120),
            Packet(8),
        ];
        let expected: Vec<u32> = (1..=8).map(timestamp).collect();
        let frames = assert_same_frames(&script, &expected).await;
        for lost in 3..=5 {
            assert!(is_silence(&frames, timestamp(lost)), "{lost}");
        }
    }

    #[tokio::test]
    async fn duplicates_and_late_replays_are_dropped_identically() {
        let script = [
            Packet(1),
            Packet(2),
            Packet(2),
            Packet(3),
            Packet(1),
            Packet(4),
            Packet(4),
        ];
        let expected = [1, 2, 3, 4].map(timestamp);
        assert_same_frames(&script, &expected).await;
    }

    #[tokio::test]
    async fn a_backward_clock_step_stays_on_the_call_timeline_identically() {
        // 4 carries a timestamp from before the stream on the same SSRC: the
        // source timeline re-anchors it one frame after 3, and the reorder
        // window does not learn a frame size from the step.
        let script = [
            Packet(1),
            Packet(2),
            Packet(3),
            Raw(Rtp {
                pt: 0,
                seq: 4,
                ts: 100,
                ssrc: SSRC,
            }),
            Raw(Rtp {
                pt: 0,
                seq: 5,
                ts: 260,
                ssrc: SSRC,
            }),
        ];
        let expected: Vec<u32> = (1..=5).map(timestamp).collect();
        assert_same_frames(&script, &expected).await;
    }

    #[tokio::test]
    async fn a_backward_clock_step_across_a_loss_neither_sizes_a_gap_nor_leaves_the_timeline() {
        // 4 is lost and 5 steps the clock backward. The gap is concealed with
        // the learned frame size, and 5 is placed by the time it waited.
        let script = [
            Packet(1),
            Packet(2),
            Packet(3),
            Raw(Rtp {
                pt: 0,
                seq: 5,
                ts: 100,
                ssrc: SSRC,
            }),
            WaitMs(120),
            Raw(Rtp {
                pt: 0,
                seq: 6,
                ts: 260,
                ssrc: SSRC,
            }),
        ];
        // 5 waited one reorder hold (45 ms = 360 ticks) after 3 was decoded.
        let frames = assert_same_frames_within(&script, &[160, 320, 480, 640, 840, 1000]).await;
        assert!(is_silence(&frames, 640));
    }

    #[tokio::test]
    async fn a_backward_step_before_the_cadence_is_learned_allocates_nothing_large() {
        // seq 100 / ts 16000 then seq 102 / ts 0: the gap cannot be sized
        // from a backward clock, so nothing is concealed, and the timeline
        // places 102 by the time it waited (one hold, 360 ticks).
        let script = [
            Raw(Rtp {
                pt: 0,
                seq: 100,
                ts: 16_000,
                ssrc: SSRC,
            }),
            Raw(Rtp {
                pt: 0,
                seq: 102,
                ts: 0,
                ssrc: SSRC,
            }),
            WaitMs(120),
        ];
        let frames = assert_same_frames_within(&script, &[16_000, 16_360]).await;
        assert!(frames.iter().all(|(_, samples)| samples.len() == 160));
    }

    #[tokio::test]
    async fn a_replacement_source_is_normalised_onto_the_call_timeline_identically() {
        // A second SSRC with an unrelated clock takes over: its first packet
        // is probation, the second is placed one frame after the last frame
        // of the first source, and a late packet of the retired source is
        // refused. Without the source timeline the inline path would emit
        // the new source's raw epoch.
        let script = [
            Packet(1),
            Packet(2),
            Packet(3),
            other_source(900),
            other_source(901),
            other_source(902),
            Packet(4),
            other_source(903),
        ];
        let expected = [4, 5, 6].map(timestamp);
        let mut expected_all: Vec<u32> = (1..=3).map(timestamp).collect();
        expected_all.extend(expected);
        assert_same_frames(&script, &expected_all).await;
    }

    #[tokio::test]
    async fn a_new_codec_generation_restarts_the_timeline_identically() {
        // After a re-INVITE commits a new codec generation, packets buffered
        // under the old one are dropped and the timeline starts again from
        // the new source's own clock, exactly as v1 does.
        let script = [
            Packet(1),
            Packet(2),
            other_source(900),
            other_source(901),
            WaitMs(5),
            NewCodec,
            other_source(902),
            other_source(903),
        ];
        let raw = |seq: u16| 7_000_000_u32 + u32::from(seq) * 160;
        let expected = [timestamp(1), timestamp(2), timestamp(3), raw(902), raw(903)];
        assert_same_frames(&script, &expected).await;
    }

    #[tokio::test]
    async fn comfort_noise_and_other_payload_types_are_ignored_identically() {
        // A CN packet (PT 13) and an unnegotiated payload type use sequence
        // numbers of the stream; both paths ignore them before the reorder
        // window, which then conceals their slots.
        let script = [
            Packet(1),
            Packet(2),
            Raw(Rtp {
                pt: 13,
                seq: 3,
                ts: timestamp(3),
                ssrc: SSRC,
            }),
            Packet(4),
            Raw(Rtp {
                pt: 96,
                seq: 5,
                ts: timestamp(5),
                ssrc: SSRC,
            }),
            Packet(6),
            WaitMs(120),
            Packet(7),
        ];
        let expected: Vec<u32> = (1..=7).map(timestamp).collect();
        let frames = assert_same_frames(&script, &expected).await;
        assert!(is_silence(&frames, timestamp(3)));
        assert!(is_silence(&frames, timestamp(5)));
    }

    #[tokio::test]
    async fn a_reorder_batch_drains_instead_of_becoming_standing_latency_identically() {
        // One loss, then live packets every 20 ms: both paths release the
        // same frames (the catch-up schedule itself is pinned by the
        // playout queue's own tests).
        let mut script = vec![Packet(1), Packet(2), Packet(3), Packet(5)];
        for seq in 6..=14 {
            script.push(WaitMs(20));
            script.push(Packet(seq));
        }
        let expected: Vec<u32> = (1..=14).map(timestamp).collect();
        assert_same_frames(&script, &expected).await;
    }

    #[test]
    fn fast_path_state_matches_push_for_in_order_packets() {
        // Interleave fast-path and push on twin buffers; their observable
        // state and output must never diverge.
        let mut fast = RtpPlayoutBuffer::new();
        let mut slow = RtpPlayoutBuffer::new();
        let start = Instant::now();
        let packet = |seq: u16| {
            RtpPacket::new_with_payload(
                0,
                seq,
                timestamp(seq),
                SSRC,
                Bytes::from_static(&[0xff; 4]),
            )
        };
        let sequences = |items: Vec<RtpPlayoutItem>| -> Vec<u16> {
            items
                .into_iter()
                .map(|item| match item {
                    RtpPlayoutItem::Packet(packet) => packet.header.sequence_number,
                    RtpPlayoutItem::Gap {
                        sequence_number, ..
                    } => sequence_number,
                })
                .collect()
        };
        for (index, seq) in [7_u16, 8, 9, 11, 10, 12, 13, 13, 14]
            .into_iter()
            .enumerate()
        {
            let now = start + Duration::from_millis(20 * index as u64);
            let slow_items = sequences(slow.push(packet(seq), now));
            let fast_items = if fast.releases_immediately(SSRC, seq) {
                fast.release_in_order(seq, timestamp(seq), now);
                vec![seq]
            } else {
                sequences(fast.push(packet(seq), now))
            };
            assert_eq!(fast_items, slow_items, "seq {seq}");
            assert_eq!(fast.expected_sequence, slow.expected_sequence);
            assert_eq!(fast.last_timestamp, slow.last_timestamp);
            assert_eq!(fast.timestamp_step, slow.timestamp_step);
            assert_eq!(fast.gap_started_at, slow.gap_started_at);
            assert_eq!(fast.startup_started_at, slow.startup_started_at);
            assert_eq!(fast.pending.len(), slow.pending.len());
        }
    }

    #[test]
    fn demotion_flushes_paced_frames_and_hands_back_held_packets_in_order() {
        let dialog = DialogId::new("inline-demote");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx);
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let now = Instant::now();
        let mut payload = [0u8; 160];
        for seq in [1_u16, 2, 5, 4] {
            payload.fill(payload_byte(seq));
            assert!(matches!(
                sink.on_packet(&inline_packet(seq, &payload), now),
                InlineSinkVerdict::Continue
            ));
        }
        lease.request_demote();
        payload.fill(payload_byte(6));
        let InlineSinkVerdict::Demote { reason, held } =
            sink.on_packet(&inline_packet(6, &payload), now)
        else {
            panic!("demotion requested");
        };
        assert_eq!(reason, InlineFallbackReason::Demoted);
        let held: Vec<u16> = held
            .iter()
            .map(|packet| packet.header.sequence_number)
            .collect();
        assert_eq!(held, vec![4, 5], "3 is still missing, so 4 and 5 were held");
        let delivered: Vec<u32> = std::iter::from_fn(|| audio_rx.try_recv().ok())
            .map(|frame| frame.timestamp)
            .collect();
        assert_eq!(delivered, vec![timestamp(1), timestamp(2)]);
        assert_eq!(sink.next_deadline(now), None, "a demoted sink arms nothing");
        assert!(
            lease.take_v1_handoff().is_some(),
            "the timeline is handed over"
        );
        assert!(lease.take_v1_handoff().is_none(), "once");
    }

    #[tokio::test]
    async fn after_demotion_the_v1_handler_continues_the_inline_source_timeline() {
        // The inline sink re-anchored a replacement source; after demotion
        // the v1 handler must keep placing that source on the same call
        // timeline rather than restart from its raw clock.
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-demote-timeline");
        let (audio_tx, mut audio_rx) = mpsc::channel(64);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .unwrap();
        controller.codec_runtimes.insert(dialog.clone(), pcmu());
        let lease = Arc::new(DialogInlineLease::new());
        controller.publish_inline_lease(&dialog, &lease);
        let (events, receiver) = tokio::sync::broadcast::channel(64);
        controller.spawn_rtp_event_handler_with_inline(
            dialog.clone(),
            receiver,
            0,
            Some(Arc::clone(&lease)),
        );

        let mut sink = InlineAudioSink::new(dialog.clone(), Arc::clone(&lease), false, false);
        let now = Instant::now();
        let mut payload = [0u8; 160];
        for step in [Packet(1), Packet(2), other_source(900), other_source(901)] {
            let rtp = step.rtp().unwrap();
            payload.fill(payload_byte(rtp.seq));
            // Back to back, so the replacement is placed one frame after the
            // first source rather than by elapsed time.
            assert!(matches!(
                sink.on_packet(&inline_rtp(rtp, &payload), now),
                InlineSinkVerdict::Continue
            ));
        }
        controller.request_inline_demotion(&dialog);
        let next = other_source(902).rtp().unwrap();
        payload.fill(payload_byte(next.seq));
        let InlineSinkVerdict::Demote { held, .. } =
            sink.on_packet(&inline_rtp(next, &payload), now)
        else {
            panic!("demotion requested");
        };
        assert!(held.is_empty());
        // What the session publishes after a demotion on a packet: the held
        // packets, then the current one.
        events
            .send(RtpSessionEvent::PacketReceived(
                RtpPacket::new_with_payload(
                    next.pt,
                    next.seq,
                    next.ts,
                    next.ssrc,
                    Bytes::from(vec![payload_byte(next.seq); 160]),
                ),
            ))
            .unwrap();
        let mut timestamps = Vec::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(Duration::from_millis(400), audio_rx.recv()).await
        {
            timestamps.push(frame.timestamp);
        }
        assert_eq!(timestamps, [1, 2, 3, 4].map(timestamp));
        drop(sink);
        drop(events);
        controller.cleanup_per_dialog_side_state(&dialog);
    }

    #[test]
    fn closed_consumer_stops_the_sink_and_releases_the_v1_handler() {
        let dialog = DialogId::new("inline-closed");
        let (audio_tx, audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx);
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        drop(audio_rx);
        let payload = [payload_byte(1); 160];
        assert!(matches!(
            sink.on_packet(&inline_packet(1, &payload), Instant::now()),
            InlineSinkVerdict::Continue
        ));
        assert!(lease.inline_stopped());
        assert_eq!(sink.next_deadline(Instant::now()), None);
    }

    #[test]
    fn receive_disabled_direction_drops_frames_like_v1() {
        let dialog = DialogId::new("inline-inactive");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, Some(MediaDirection::SendOnly), audio_tx);
        let mut sink = InlineAudioSink::new(dialog, lease, false, false);
        let payload = [payload_byte(1); 160];
        let _ = sink.on_packet(&inline_packet(1, &payload), Instant::now());
        assert!(audio_rx.try_recv().is_err());
    }

    async fn start_pcmu(
        controller: &MediaSessionController,
        dialog: &DialogId,
        config: MediaExecutorConfig,
    ) {
        let _config = override_media_executor_config_on_this_thread(config);
        controller
            .start_media(dialog.clone(), pcmu_config())
            .await
            .unwrap();
    }

    async fn peer_to(
        controller: &MediaSessionController,
        dialog: &DialogId,
    ) -> (tokio::net::UdpSocket, SocketAddr) {
        let port = controller
            .get_session_info(dialog)
            .await
            .and_then(|info| info.rtp_port)
            .expect("rtp port");
        (
            tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap(),
            SocketAddr::from(([127, 0, 0, 1], port)),
        )
    }

    async fn send_rtp(
        peer: &tokio::net::UdpSocket,
        destination: SocketAddr,
        rtp: Rtp,
        payload: Vec<u8>,
    ) {
        let packet =
            RtpPacket::new_with_payload(rtp.pt, rtp.seq, rtp.ts, rtp.ssrc, Bytes::from(payload));
        peer.send_to(&packet.serialize().unwrap(), destination)
            .await
            .unwrap();
    }

    /// A real dialog on loopback: audio with a lost packet, comfort noise and
    /// an RFC 4733 digit (start, continuation, three end retransmits) on the
    /// same SSRC. Returns the decoded frames, the digits delivered and
    /// whether the dialog ran inline.
    async fn real_dialog_run(config: MediaExecutorConfig) -> (Frames, Vec<char>, bool) {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-real-equivalence");
        start_pcmu(&controller, &dialog, config).await;
        let inline = controller.inline_leases.contains_key(&dialog);
        let (audio_tx, mut audio_rx) = mpsc::channel(64);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .unwrap();
        let (dtmf_tx, mut dtmf_rx) = mpsc::channel(8);
        controller
            .set_dtmf_callback(dialog.clone(), dtmf_tx)
            .await
            .unwrap();
        let (peer, destination) = peer_to(&controller, &dialog).await;
        let audio = |seq: u16| Packet(seq).rtp().unwrap();
        for seq in 1_u16..=3 {
            send_rtp(&peer, destination, audio(seq), vec![payload_byte(seq); 160]).await;
        }
        let digit_ts = timestamp(4);
        for (seq, end, duration) in [
            (4_u16, false, 160_u16),
            (5, false, 320),
            (6, true, 480),
            (7, true, 480),
            (8, true, 480),
        ] {
            let flags = if end { 0x80 | 10 } else { 10 };
            let body = vec![5, flags, (duration >> 8) as u8, duration as u8];
            let rtp = Rtp {
                pt: 101,
                seq,
                ts: digit_ts,
                ssrc: SSRC,
            };
            send_rtp(&peer, destination, rtp, body).await;
        }
        let cn = Rtp {
            pt: 13,
            seq: 9,
            ts: timestamp(9),
            ssrc: SSRC,
        };
        send_rtp(&peer, destination, cn, vec![40]).await;
        for seq in [10_u16, 12, 11, 13] {
            send_rtp(&peer, destination, audio(seq), vec![payload_byte(seq); 160]).await;
        }
        let mut frames = Vec::new();
        while let Ok(Some(frame)) =
            tokio::time::timeout(Duration::from_millis(500), audio_rx.recv()).await
        {
            frames.push((frame.timestamp, frame.samples));
        }
        let mut digits = Vec::new();
        while let Ok(notification) = dtmf_rx.try_recv() {
            digits.push(notification.digit);
        }
        controller.stop_media(&dialog).await.unwrap();
        (frames, digits, inline)
    }

    #[tokio::test]
    async fn a_real_dialog_delivers_identical_audio_and_dtmf_on_v1_and_v2() {
        let before = inline_executor_counters();
        let (v1_frames, v1_digits, v1_inline) = real_dialog_run(MediaExecutorConfig::v1()).await;
        let (v2_frames, v2_digits, v2_inline) = real_dialog_run(MediaExecutorConfig::v2()).await;
        assert!(!v1_inline && v2_inline);
        assert_eq!(v1_digits, ['5'], "one digit per tone on v1");
        assert_eq!(
            v2_digits, v1_digits,
            "DTMF keeps the broadcast to the v1 handler"
        );
        let timestamps: Vec<u32> = v2_frames.iter().map(|(timestamp, _)| *timestamp).collect();
        assert_eq!(timestamps, (1..=13).map(timestamp).collect::<Vec<_>>());
        assert_eq!(v2_frames, v1_frames);
        assert!(inline_executor_counters().inline_sinks > before.inline_sinks);
    }

    /// Hot call state changes where the socket owner keeps its per-packet
    /// state and how it checks its fences, never what reaches the playout:
    /// a real v2 dialog decodes and plays out the same frames and digits with
    /// it on as off.
    #[tokio::test]
    async fn a_real_dialog_plays_out_identically_with_hot_state_on_and_off() {
        let (off_frames, off_digits, off_inline) =
            real_dialog_run(MediaExecutorConfig::v2().with_hot_state(false)).await;
        let (on_frames, on_digits, on_inline) = real_dialog_run(MediaExecutorConfig::v2()).await;
        assert!(off_inline && on_inline);
        assert_eq!(on_digits, off_digits);
        assert_eq!(on_frames, off_frames);
        let timestamps: Vec<u32> = on_frames.iter().map(|(timestamp, _)| *timestamp).collect();
        assert_eq!(timestamps, (1..=13).map(timestamp).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn v2_without_inline_decode_keeps_the_v1_handler() {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-decode-off");
        start_pcmu(
            &controller,
            &dialog,
            MediaExecutorConfig::v2().with_inline_decode(false),
        )
        .await;
        assert!(controller.inline_leases.is_empty(), "no sink offered");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .unwrap();
        let (peer, destination) = peer_to(&controller, &dialog).await;
        for seq in 1_u16..=3 {
            send_rtp(
                &peer,
                destination,
                Packet(seq).rtp().unwrap(),
                vec![payload_byte(seq); 160],
            )
            .await;
        }
        for seq in 1_u16..=3 {
            let frame = tokio::time::timeout(Duration::from_secs(2), audio_rx.recv())
                .await
                .expect("decoded by the v1 handler")
                .expect("open channel");
            assert_eq!(frame.timestamp, timestamp(seq));
        }
        controller.stop_media(&dialog).await.unwrap();
    }

    #[tokio::test]
    async fn real_dialog_runs_inline_and_teardown_releases_everything() {
        let before = inline_executor_counters();
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-teardown");
        start_pcmu(&controller, &dialog, MediaExecutorConfig::v2()).await;
        let lease = controller
            .inline_leases
            .get(&dialog)
            .map(|entry| Arc::clone(entry.value()))
            .expect("v2 publishes an inline lease");
        let (audio_tx, mut audio_rx) = mpsc::channel(64);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx)
            .await
            .unwrap();
        assert!(lease.has_audio_sender());
        #[cfg(feature = "perf-diagnostics")]
        assert_eq!(
            controller.diagnostic_counts()["inline_rx_leases"],
            serde_json::json!(1)
        );

        let (peer, destination) = peer_to(&controller, &dialog).await;
        for seq in 1_u16..=3 {
            send_rtp(
                &peer,
                destination,
                Packet(seq).rtp().unwrap(),
                vec![payload_byte(seq); 160],
            )
            .await;
        }
        for seq in 1_u16..=3 {
            let frame = tokio::time::timeout(Duration::from_secs(2), audio_rx.recv())
                .await
                .expect("inline frame")
                .expect("open channel");
            assert_eq!(frame.timestamp, timestamp(seq));
        }

        controller.stop_media(&dialog).await.unwrap();
        assert!(controller.inline_leases.is_empty());
        assert!(lease.is_retired());
        assert!(!lease.has_audio_sender());
        // Every sender is gone: the controller map entry, the lease, and the
        // sink (dropped with the joined owner task).
        assert!(
            tokio::time::timeout(Duration::from_secs(1), audio_rx.recv())
                .await
                .expect("closure is observed promptly")
                .is_none()
        );
        // The v1 handler holds the last other lease reference until its
        // event broadcast closes.
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&lease) > 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("all lease references released");
        let after = inline_executor_counters();
        assert!(after.inline_packets >= before.inline_packets + 3);
        assert!(after.inline_sinks > before.inline_sinks);
    }

    // ---- Fused decoded-audio consumer ----

    #[derive(Default)]
    struct ConsumerProbe {
        frames: parking_lot::Mutex<Vec<(u32, Vec<i16>)>>,
        installed: AtomicBool,
        detached: AtomicBool,
        dropped: AtomicBool,
        ticks: AtomicU64,
    }

    /// Records every frame; optionally asks for one deadline and stops
    /// (`Closed`) after a number of frames.
    struct ProbeConsumer {
        probe: Arc<ConsumerProbe>,
        deadline: Option<Instant>,
        close_after: Option<usize>,
    }

    impl Drop for ProbeConsumer {
        fn drop(&mut self) {
            self.probe.dropped.store(true, Ordering::Release);
        }
    }

    impl InlineAudioConsumer for ProbeConsumer {
        fn on_installed(&mut self, _now: Instant) {
            self.probe.installed.store(true, Ordering::Release);
        }

        fn on_frame(&mut self, frame: AudioFrame, _now: Instant) -> InlineAudioDelivery {
            let mut frames = self.probe.frames.lock();
            frames.push((frame.timestamp, frame.samples));
            if self.close_after == Some(frames.len()) {
                return InlineAudioDelivery::Closed;
            }
            InlineAudioDelivery::Accepted
        }

        fn next_deadline(&self) -> Option<Instant> {
            self.deadline
        }

        fn on_deadline(&mut self, now: Instant) -> InlineAudioDelivery {
            if self.deadline.is_some_and(|deadline| deadline <= now) {
                self.deadline = None;
                self.probe.ticks.fetch_add(1, Ordering::AcqRel);
            }
            InlineAudioDelivery::Accepted
        }

        fn on_detach(&mut self, _now: Instant) {
            self.probe.detached.store(true, Ordering::Release);
        }
    }

    fn probe_consumer(
        deadline: Option<Instant>,
        close_after: Option<usize>,
    ) -> (Box<ProbeConsumer>, Arc<ConsumerProbe>) {
        let probe = Arc::new(ConsumerProbe::default());
        (
            Box::new(ProbeConsumer {
                probe: Arc::clone(&probe),
                deadline,
                close_after,
            }),
            probe,
        )
    }

    fn no_backlog() -> Option<AudioFrame> {
        None
    }

    /// `run_inline` with a consumer fused onto the sink: what the consumer
    /// sees, and whether anything leaked onto the channel.
    fn run_inline_fused(script: &[Step]) -> (Frames, bool) {
        let dialog = DialogId::new("inline-fused");
        let (audio_tx, mut audio_rx) = mpsc::channel(256);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let (consumer, probe) = probe_consumer(None, None);
        let guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("an inline dialog accepts a consumer"));
        let mut now = Instant::now();
        let mut payload = [0u8; 160];
        for step in script {
            match *step {
                WaitMs(ms) => now = advance(&mut sink, now, now + Duration::from_millis(ms)),
                NewCodec => unreachable!("not used with a fused consumer"),
                packet => {
                    let rtp = packet.rtp().expect("a packet step");
                    payload.fill(payload_byte(rtp.seq));
                    let verdict = sink.on_packet(&inline_rtp(rtp, &payload), now);
                    assert!(matches!(verdict, InlineSinkVerdict::Continue));
                    let _ = sink.next_deadline(now);
                }
            }
        }
        advance(&mut sink, now, now + Duration::from_secs(2));
        let leaked = audio_rx.try_recv().is_ok();
        drop(sink);
        drop(guard);
        let frames = std::mem::take(&mut *probe.frames.lock());
        (frames, leaked)
    }

    #[test]
    fn fused_consumer_sees_exactly_the_frames_the_channel_would() {
        let scripts: Vec<Vec<Step>> = vec![
            (1..=12).map(Packet).collect(),
            vec![
                Packet(1),
                Packet(2),
                Packet(4),
                Packet(3),
                Packet(5),
                Packet(7),
                Packet(6),
            ],
            vec![
                Packet(1),
                Packet(2),
                Packet(4),
                Packet(5),
                WaitMs(120),
                Packet(6),
                Packet(7),
                Packet(8),
                Packet(20),
                WaitMs(120),
                Packet(21),
            ],
            vec![
                Packet(1),
                Packet(2),
                Packet(2),
                Packet(3),
                Packet(1),
                Packet(4),
                Packet(4),
            ],
            vec![
                Packet(1),
                Packet(2),
                other_source(900),
                other_source(901),
                other_source(902),
            ],
        ];
        for script in scripts {
            let channel = run_inline(&script);
            let (fused, leaked) = run_inline_fused(&script);
            assert!(!channel.is_empty());
            assert_eq!(fused, channel, "{script:?}");
            assert!(
                !leaked,
                "no frame may reach the channel while a consumer is fused"
            );
        }
    }

    #[test]
    fn consumer_deadline_joins_the_owner_timer_and_is_served_once() {
        let dialog = DialogId::new("inline-consumer-deadline");
        let (audio_tx, _audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let now = Instant::now();
        assert_eq!(sink.next_deadline(now), None);
        let due = now + Duration::from_millis(7);
        let (consumer, probe) = probe_consumer(Some(due), None);
        let _guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        assert!(probe.installed.load(Ordering::Acquire));
        // The install bumped the consumer epoch: the sink reads the new
        // deadline without being told.
        assert_eq!(sink.next_deadline(now), Some(due));
        assert!(matches!(sink.on_deadline(due), InlineSinkVerdict::Continue));
        assert_eq!(probe.ticks.load(Ordering::Acquire), 1);
        assert_eq!(sink.next_deadline(due), None);
        // Decoded frames keep reaching the consumer on the owner.
        let payload = [payload_byte(1); 160];
        let _ = sink.on_packet(&inline_packet(1, &payload), due);
        assert_eq!(probe.frames.lock().len(), 1);
    }

    #[test]
    fn install_drains_the_channel_backlog_first_and_is_refused_when_not_inline() {
        let dialog = DialogId::new("inline-install");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let _sink = InlineAudioSink::new(dialog.clone(), Arc::clone(&lease), false, false);

        // A different channel than the registered callback is refused.
        let (other_tx, _other_rx) = mpsc::channel(1);
        let (consumer, probe) = probe_consumer(None, None);
        assert!(install_consumer(&lease, &other_tx, consumer, &mut no_backlog).is_err());
        assert!(!probe.installed.load(Ordering::Acquire));

        // Frames already queued for the reader move into the consumer first,
        // in order.
        for seq in [1_u16, 2, 3] {
            audio_tx
                .try_send(AudioFrame::new(
                    vec![seq as i16; 160],
                    8000,
                    1,
                    timestamp(seq),
                ))
                .unwrap();
        }
        let mut backlog = || audio_rx.try_recv().ok();
        let (consumer, probe) = probe_consumer(None, None);
        let guard = install_consumer(&lease, &audio_tx, consumer, &mut backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        let seen: Vec<u32> = probe.frames.lock().iter().map(|(ts, _)| *ts).collect();
        assert_eq!(seen, [1, 2, 3].map(timestamp));
        assert!(lease.has_consumer());

        // One consumer at a time.
        let (second, _) = probe_consumer(None, None);
        assert!(install_consumer(&lease, &audio_tx, second, &mut no_backlog).is_err());

        // The guard detaches its own consumer.
        drop(guard);
        assert!(!lease.has_consumer());
        assert!(probe.dropped.load(Ordering::Acquire));

        // Demotion refuses later installs.
        lease.request_demote();
        let (late, _) = probe_consumer(None, None);
        assert!(install_consumer(&lease, &audio_tx, late, &mut no_backlog).is_err());
    }

    #[test]
    fn callback_replacement_sink_drop_and_retire_detach_the_consumer() {
        let dialog = DialogId::new("inline-detach");
        let (audio_tx, _audio_rx) = mpsc::channel(16);
        let maps = LeaseMaps::new(&dialog, None, audio_tx.clone());
        let lease = Arc::clone(&maps.lease);
        let sink = InlineAudioSink::new(dialog.clone(), Arc::clone(&lease), false, false);

        // Re-syncing with the same channel keeps the consumer.
        let (consumer, probe) = probe_consumer(None, None);
        let _guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        maps.resync();
        assert!(lease.has_consumer());

        // A replaced callback detaches it: v1 would stop feeding its reader.
        let (replacement_tx, _replacement_rx) = mpsc::channel(16);
        maps.callbacks
            .insert(dialog.clone(), replacement_tx.clone());
        maps.resync();
        assert!(!lease.has_consumer());
        assert!(probe.dropped.load(Ordering::Acquire));

        // Dropping the sink detaches and refuses any later consumer.
        let (consumer, probe) = probe_consumer(None, None);
        let _guard = install_consumer(&lease, &replacement_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        drop(sink);
        assert!(probe.dropped.load(Ordering::Acquire));
        let (late, _) = probe_consumer(None, None);
        assert!(install_consumer(&lease, &replacement_tx, late, &mut no_backlog).is_err());

        // Retirement detaches too.
        let other = DialogId::new("inline-retire");
        let (other_tx, _other_rx) = mpsc::channel(16);
        let other_lease = lease_with(&other, None, other_tx.clone());
        let _other_sink = InlineAudioSink::new(other, Arc::clone(&other_lease), false, false);
        let (consumer, probe) = probe_consumer(None, None);
        let _guard = install_consumer(&other_lease, &other_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        other_lease.retire();
        assert!(!other_lease.has_consumer());
        assert!(probe.dropped.load(Ordering::Acquire));
    }

    #[test]
    fn a_closed_consumer_hands_later_frames_back_to_the_channel() {
        let dialog = DialogId::new("inline-consumer-closed");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let (consumer, probe) = probe_consumer(None, Some(2));
        let _guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        let now = Instant::now();
        let mut payload = [0u8; 160];
        for seq in 1_u16..=4 {
            payload.fill(payload_byte(seq));
            let _ = sink.on_packet(&inline_packet(seq, &payload), now);
        }
        assert_eq!(probe.frames.lock().len(), 2);
        assert!(probe.dropped.load(Ordering::Acquire));
        assert!(!lease.has_consumer());
        let channel: Vec<u32> = std::iter::from_fn(|| audio_rx.try_recv().ok())
            .map(|frame| frame.timestamp)
            .collect();
        assert_eq!(channel, [3, 4].map(timestamp));
    }

    #[test]
    fn demotion_flushes_paced_frames_into_the_consumer_then_detaches_it() {
        let dialog = DialogId::new("inline-demote-fused");
        let (audio_tx, mut audio_rx) = mpsc::channel(16);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let (consumer, probe) = probe_consumer(None, None);
        let _guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("accepted"));
        let now = Instant::now();
        let mut payload = [0u8; 160];
        for seq in [1_u16, 2, 5, 4] {
            payload.fill(payload_byte(seq));
            let _ = sink.on_packet(&inline_packet(seq, &payload), now);
        }
        lease.request_demote();
        payload.fill(payload_byte(6));
        let InlineSinkVerdict::Demote { held, .. } =
            sink.on_packet(&inline_packet(6, &payload), now)
        else {
            panic!("demotion requested");
        };
        assert_eq!(held.len(), 2);
        let seen: Vec<u32> = probe.frames.lock().iter().map(|(ts, _)| *ts).collect();
        assert_eq!(seen, [1, 2].map(timestamp));
        assert!(probe.detached.load(Ordering::Acquire), "on_detach ran");
        assert!(probe.dropped.load(Ordering::Acquire));
        assert!(!lease.has_consumer());
        assert!(audio_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn real_dialog_feeds_a_fused_consumer_and_teardown_releases_it() {
        let before = inline_rx_counters();
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-fused-teardown");
        start_pcmu(&controller, &dialog, MediaExecutorConfig::v2()).await;
        let lease = controller
            .inline_leases
            .get(&dialog)
            .map(|entry| Arc::clone(entry.value()))
            .expect("v2 publishes an inline lease");
        let (audio_tx, mut audio_rx) = mpsc::channel(64);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx.clone())
            .await
            .unwrap();
        let (consumer, probe) = probe_consumer(None, None);
        let mut backlog = || audio_rx.try_recv().ok();
        let guard = controller
            .install_inline_audio_consumer(&dialog, &audio_tx, consumer, &mut backlog)
            .unwrap_or_else(|_| panic!("an inline G.711 dialog accepts a consumer"));
        drop(audio_tx);

        let (peer, destination) = peer_to(&controller, &dialog).await;
        for seq in 1_u16..=3 {
            send_rtp(
                &peer,
                destination,
                Packet(seq).rtp().unwrap(),
                vec![payload_byte(seq); 160],
            )
            .await;
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while probe.frames.lock().len() < 3 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the owner fed the fused consumer");
        let seen: Vec<u32> = probe.frames.lock().iter().map(|(ts, _)| *ts).collect();
        assert_eq!(seen, [1, 2, 3].map(timestamp));
        assert!(audio_rx.try_recv().is_err(), "nothing crossed the channel");

        controller.stop_media(&dialog).await.unwrap();
        assert!(
            probe.dropped.load(Ordering::Acquire),
            "teardown drops the consumer"
        );
        assert!(!lease.has_consumer());
        drop(guard);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), audio_rx.recv())
                .await
                .expect("closure is observed promptly")
                .is_none(),
            "every channel sender is released"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&lease) > 1 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("all lease references released");
        let after = inline_rx_counters();
        assert!(after.fused_consumers > before.fused_consumers);
        assert!(after.fused_frames >= before.fused_frames + 3);
        assert_eq!(controller.inline_leases.len(), 0);
    }

    #[tokio::test]
    async fn install_is_refused_for_a_dialog_that_does_not_decode_inline() {
        let controller = MediaSessionController::new();
        let dialog = DialogId::new("inline-refused-v1");
        start_pcmu(&controller, &dialog, MediaExecutorConfig::v1()).await;
        assert!(controller.inline_leases.is_empty());
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        controller
            .set_audio_frame_callback(dialog.clone(), audio_tx.clone())
            .await
            .unwrap();
        let (consumer, probe) = probe_consumer(None, None);
        assert!(controller
            .install_inline_audio_consumer(&dialog, &audio_tx, consumer, &mut no_backlog)
            .is_err());
        assert!(!probe.installed.load(Ordering::Acquire));
        controller.request_inline_demotion(&dialog);
        controller.stop_media(&dialog).await.unwrap();
    }

    // ---- Lane tick: service on lane ticks ----

    /// Frames a consumer saw: (timestamp, samples, when it saw them).
    type TimedFrames = Vec<(u32, Vec<i16>, Instant)>;

    /// Records every frame with the instant it reached the consumer.
    struct TimingConsumer {
        frames: Arc<parking_lot::Mutex<TimedFrames>>,
    }

    impl InlineAudioConsumer for TimingConsumer {
        fn on_installed(&mut self, _now: Instant) {}

        fn on_frame(&mut self, frame: AudioFrame, now: Instant) -> InlineAudioDelivery {
            self.frames
                .lock()
                .push((frame.timestamp, frame.samples, now));
            InlineAudioDelivery::Accepted
        }

        fn next_deadline(&self) -> Option<Instant> {
            None
        }

        fn on_deadline(&mut self, _now: Instant) -> InlineAudioDelivery {
            InlineAudioDelivery::Accepted
        }

        fn on_detach(&mut self, _now: Instant) {}
    }

    /// The lane thread's wake-up latency after a tick sleep, in the model.
    const TICK_WAKE: Duration = Duration::from_micros(60);

    /// Feed `arrivals` (sequence, exact arrival instant) to a sink with a
    /// fused consumer and serve its deadlines, as the socket owner does:
    /// exactly (`tick: None`), or on the ticks of a fake-clock grid (every
    /// arrival and deadline at the first boundary at or after it plus
    /// [`TICK_WAKE`]; packets served in one tick 1 µs apart, receives before
    /// deadlines). Returns what the consumer saw and when.
    fn run_on_ticks(
        arrivals: &[(u16, Instant)],
        tick: Option<rvoip_rtp_core::lane_tick::TickGrid>,
        anchored: bool,
    ) -> TimedFrames {
        let dialog = DialogId::new("lane-tick");
        let (audio_tx, _audio_rx) = mpsc::channel(256);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        sink.decoded_playout.anchored = anchored;
        let frames = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let consumer = Box::new(TimingConsumer {
            frames: Arc::clone(&frames),
        });
        let guard = install_consumer(&lease, &audio_tx, consumer, &mut no_backlog)
            .unwrap_or_else(|_| panic!("an inline dialog accepts a consumer"));
        let serve = |at: Instant| match tick {
            Some(grid) => grid.ceil(at) + TICK_WAKE,
            None => at,
        };
        let mut payload = [0u8; 160];
        let mut now = arrivals[0].1;
        let mut next = 0;
        for _ in 0..100_000 {
            let arrival = arrivals.get(next).map(|(_, at)| serve(*at));
            let deadline = sink.next_deadline(now).map(serve);
            match (arrival, deadline) {
                (None, None) => break,
                (Some(arrival), deadline)
                    if deadline.is_none_or(|deadline| arrival <= deadline) =>
                {
                    let mut offset = 0;
                    while let Some((seq, at)) = arrivals.get(next) {
                        if serve(*at) != arrival {
                            break;
                        }
                        now = now.max(arrival + Duration::from_micros(offset));
                        payload.fill(payload_byte(*seq));
                        let verdict = sink.on_packet(&inline_packet(*seq, &payload), now);
                        assert!(matches!(verdict, InlineSinkVerdict::Continue));
                        offset += 1;
                        next += 1;
                    }
                }
                (_, Some(deadline)) => {
                    now = now.max(deadline);
                    assert!(matches!(sink.on_deadline(now), InlineSinkVerdict::Continue));
                }
                (Some(_), None) => unreachable!("handled by the arrival arm"),
            }
        }
        assert_eq!(next, arrivals.len(), "every packet was fed");
        drop(sink);
        drop(guard);
        let seen = std::mem::take(&mut *frames.lock());
        seen
    }

    /// The inline receive path served on lane ticks hands the consumer
    /// exactly the frames (timestamps, samples, order) it hands it when every
    /// packet and deadline is served on time, for every quantum and two grid
    /// phases. Frames delivered on arrival (an in-order stream with jitter)
    /// are at most one quantum (plus the wake-up latency) late. Frames
    /// released by a loss or reorder hold and then paced are later by at most
    /// three quanta: the hold, and the pacing step after it, are relative
    /// deadlines re-quantized once each; nothing accumulates.
    #[test]
    fn tick_service_hands_the_consumer_the_same_frames_within_the_quantum() {
        let start = Instant::now();
        let at = |slot: u64, extra_us: u64| start + Duration::from_micros(20_000 * slot + extra_us);
        let in_order: Vec<(u16, Instant)> = (1..=60u16)
            .map(|seq| {
                let jitter = u64::from(seq % 4) * 900 + u64::from(seq % 3) * 130;
                (seq, at(u64::from(seq), jitter))
            })
            .collect();
        let lossy: Vec<(u16, Instant)> = (1..=60u16)
            .filter(|seq| *seq != 20 && *seq != 41)
            .map(|seq| (seq, at(u64::from(seq), 0)))
            .collect();
        let reordered: Vec<(u16, Instant)> = (1..=60u16)
            .map(|seq| match seq {
                30 => (31, at(30, 0)),
                31 => (30, at(30, 3_000)),
                _ => (seq, at(u64::from(seq), 0)),
            })
            .collect();
        let scripts = [
            ("in-order", &in_order, 1u32),
            ("lossy", &lossy, 3),
            ("reordered", &reordered, 3),
        ];
        let strip = |frames: &[(u32, Vec<i16>, Instant)]| {
            frames
                .iter()
                .map(|(timestamp, samples, _)| (*timestamp, samples.clone()))
                .collect::<Vec<_>>()
        };
        for (name, arrivals, quanta) in scripts {
            let exact = run_on_ticks(arrivals, None, false);
            assert!(exact.len() >= 58, "{name}: frames flowed ({})", exact.len());
            assert_eq!(
                exact,
                run_on_ticks(arrivals, None, true),
                "{name}: anchoring is a no-op on time"
            );
            for quantum_ms in rvoip_rtp_core::lane_tick::LANE_TICK_QUANTA_MS {
                let quantum = Duration::from_millis(quantum_ms);
                for phase_us in [700u64, 3_100] {
                    let grid = rvoip_rtp_core::lane_tick::TickGrid::new(
                        start - Duration::from_micros(phase_us),
                        quantum,
                    );
                    let ticked = run_on_ticks(arrivals, Some(grid), true);
                    assert_eq!(
                        strip(&ticked),
                        strip(&exact),
                        "{name} Q={quantum:?}: same frames"
                    );
                    let bound = quantum * quanta + TICK_WAKE * 2 + Duration::from_micros(100);
                    for ((timestamp, _, on_time), (_, _, served)) in exact.iter().zip(&ticked) {
                        assert!(
                            served >= on_time,
                            "{name} Q={quantum:?}: frame {timestamp} early"
                        );
                        let late = served.saturating_duration_since(*on_time);
                        assert!(
                            late <= bound,
                            "{name} Q={quantum:?}: frame {timestamp} {late:?} late"
                        );
                    }
                }
            }
        }
    }

    // ---- PCM pool and the allocation-free steady state ----

    /// A fused consumer that only counts frames and hands their PCM back to
    /// the pool, as a consumer that is done with a frame does.
    struct RecyclingConsumer {
        frames: Arc<AtomicU64>,
    }

    impl InlineAudioConsumer for RecyclingConsumer {
        fn on_installed(&mut self, _now: Instant) {}

        fn on_frame(&mut self, frame: AudioFrame, _now: Instant) -> InlineAudioDelivery {
            self.frames.fetch_add(1, Ordering::Relaxed);
            pcm_pool::recycle_pcm_buffer(frame.samples);
            InlineAudioDelivery::Accepted
        }

        fn next_deadline(&self) -> Option<Instant> {
            None
        }

        fn on_deadline(&mut self, _now: Instant) -> InlineAudioDelivery {
            InlineAudioDelivery::Accepted
        }

        fn on_detach(&mut self, _now: Instant) {}
    }

    /// Packets `sequences` in order, packet n arriving at 20·(n−1) ms, with
    /// the owner deadlines served in between.
    fn feed_in_order(
        sink: &mut InlineAudioSink,
        start: Instant,
        mut now: Instant,
        sequences: std::ops::RangeInclusive<u16>,
    ) -> Instant {
        let mut payload = [0u8; 160];
        for seq in sequences {
            now = advance(
                sink,
                now,
                start + Duration::from_millis(u64::from(seq - 1) * 20),
            );
            payload.fill(payload_byte(seq));
            let verdict = sink.on_packet(&inline_packet(seq, &payload), now);
            assert!(matches!(verdict, InlineSinkVerdict::Continue));
            let _ = sink.next_deadline(now);
        }
        now
    }

    /// After warm-up, an in-order stream goes receive buffer → pooled G.711
    /// decode → pacing → fused consumer → back to the pool without a single
    /// PCM allocation (no pool miss on this thread) and without the pacing
    /// queue or the reorder scratch growing.
    #[test]
    fn steady_state_inline_receive_reuses_every_pcm_buffer() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        let dialog = DialogId::new("inline-pool-steady");
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        let lease = lease_with(&dialog, None, audio_tx.clone());
        let mut sink = InlineAudioSink::new(dialog, Arc::clone(&lease), false, false);
        let frames = Arc::new(AtomicU64::new(0));
        let _guard = install_consumer(
            &lease,
            &audio_tx,
            Box::new(RecyclingConsumer {
                frames: Arc::clone(&frames),
            }),
            &mut no_backlog,
        )
        .unwrap_or_else(|_| panic!("an inline dialog accepts a consumer"));
        let start = Instant::now();
        let now = feed_in_order(&mut sink, start, start, 1..=200);
        let warmed = frames.load(Ordering::Relaxed);
        assert!(warmed >= 190, "frames flowed during warm-up: {warmed}");
        let misses = pcm_pool::pcm_pool_misses_on_this_thread();
        let pacing = sink.decoded_playout.pending.capacity();
        let scratch = sink.playout_items.capacity();
        feed_in_order(&mut sink, start, now, 201..=600);
        assert_eq!(
            pcm_pool::pcm_pool_misses_on_this_thread(),
            misses,
            "every decode reused a buffer the consumer handed back"
        );
        assert_eq!(sink.decoded_playout.pending.capacity(), pacing);
        assert_eq!(sink.playout_items.capacity(), scratch);
        assert!(frames.load(Ordering::Relaxed) >= warmed + 395);
    }

    /// The reorder buffer's released items reuse the sink's scratch: a
    /// reordered and a lossy stretch leave the scratch empty with its
    /// storage.
    #[test]
    fn reorder_scratch_is_drained_after_every_release() {
        let dialog = DialogId::new("inline-scratch");
        let (audio_tx, _audio_rx) = mpsc::channel(256);
        let lease = lease_with(&dialog, None, audio_tx);
        let mut sink = InlineAudioSink::new(dialog, lease, false, false);
        let mut now = Instant::now();
        let mut payload = [0u8; 160];
        for seq in [1u16, 2, 4, 3, 5, 7, 8, 9, 6, 10] {
            payload.fill(payload_byte(seq));
            let verdict = sink.on_packet(&inline_packet(seq, &payload), now);
            assert!(matches!(verdict, InlineSinkVerdict::Continue));
            assert!(sink.playout_items.is_empty(), "drained after packet {seq}");
            now = advance(&mut sink, now, now + Duration::from_millis(20));
            assert!(sink.playout_items.is_empty(), "drained after the deadlines");
        }
        assert!(
            sink.playout_items.capacity() > 0,
            "the scratch keeps its storage"
        );
    }

    /// A concealment frame takes its zeroed PCM from the pool, and a frame
    /// the pacing queue cannot hold goes back to it.
    #[test]
    fn gap_frames_come_from_and_overflow_returns_to_the_pool() {
        let _v2 = override_media_executor_config_on_this_thread(MediaExecutorConfig::v2());
        use crate::pcm_pool::{pcm_pool_len_on_this_thread, recycle_pcm_buffer, PCM_POOL_CAPACITY};
        recycle_pcm_buffer(vec![i16::MAX; 160]);
        let pooled = pcm_pool_len_on_this_thread();
        assert!(pooled >= 1);
        let codec = pcmu();
        let dialog = DialogId::new("inline-pool-gap");
        let now = Instant::now();
        let mut queue = DecodedPlayoutQueue::new();
        let mut timeline = SourceTimeline::default();
        enqueue_inline(
            std::iter::once(RtpPlayoutItem::Gap {
                sequence_number: 5,
                timestamp: 800,
                samples_per_channel: 160,
            }),
            &codec,
            &dialog,
            &mut queue,
            &mut timeline,
            now,
        );
        assert_eq!(
            pcm_pool_len_on_this_thread(),
            pooled - 1,
            "the gap frame came from the pool"
        );
        let gap = queue.pending.pop_front().expect("queued");
        assert_eq!(
            gap.samples,
            vec![0i16; 160],
            "a dirty pooled buffer is zeroed"
        );
        assert_eq!(
            gap.timestamp, 800,
            "no active source: the gap keeps its timestamp"
        );
        recycle_pcm_buffer(gap.samples);
        // Overflow: the refused frame's PCM goes back to the pool.
        for index in 0..RTP_REORDER_MAX_PACKETS {
            assert!(queue.push(
                AudioFrame::new(vec![1; 160], 8_000, 1, index as u32 * 160),
                now
            ));
        }
        let before = pcm_pool_len_on_this_thread();
        let full = before >= PCM_POOL_CAPACITY;
        assert!(!queue.push(AudioFrame::new(vec![2; 160], 8_000, 1, 99_999), now));
        if !full {
            assert_eq!(pcm_pool_len_on_this_thread(), before + 1);
        }
    }
}
