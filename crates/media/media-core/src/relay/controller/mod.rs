//! Media Session Controller for Session-Core Integration
//!
//! This module provides the high-level interface for session-core to control
//! media sessions. It manages the lifecycle of media sessions tied to SIP dialogs.
//!
//! ## Audio Muting
//!
//! The controller implements production-ready audio muting using silence-based
//! approach. When `set_audio_muted()` is called, the RTP stream continues but
//! audio samples are replaced with silence before encoding. This maintains:
//!
//! - Continuous RTP sequence numbers and timestamps
//! - NAT traversal and firewall state
//! - Compatibility with all SIP endpoints
//! - Instant mute/unmute without renegotiation
//!
//! Use `set_audio_muted()` and `is_audio_muted()` for muting functionality.

use dashmap::DashMap;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, trace, warn};

use crate::codec::mapping::CodecMapper;
use crate::diagnostics;
use crate::error::{Error, Result};
use crate::integration::{RtpBridge, RtpBridgeConfig, RtpEventCallback};
use crate::performance::{
    metrics::ConcurrentPerformanceMetrics,
    pool::{AudioFramePool, PoolConfig, RtpBufferPool},
    simd::SimdProcessor,
};
use crate::processing::audio::AudioMixer;
use crate::quality::QualityMonitor;
use crate::relay::controller::codec_detection::CodecDetector;
use crate::relay::controller::codec_fallback::CodecFallbackManager;
use crate::types::conference::{ConferenceMixingConfig, ConferenceMixingEvent};
use crate::types::{AudioFrame, DialogId, MediaDirection, MediaSessionId};

use rvoip_rtp_core as rtp_core;
use rvoip_rtp_core::transport::{
    AllocationStrategy, GlobalPortAllocator, PortAllocator, PortAllocatorConfig,
    SessionPortReservation, SymmetricRtpPolicy,
};
use rvoip_rtp_core::{
    RtpPacket, RtpSession, RtpSessionBufferConfig, RtpSessionConfig, RtpTransportBufferConfig,
};

mod codec_runtime;
mod inline_rx;
mod source_timeline;

const RTP_REORDER_HOLD: Duration = Duration::from_millis(45);
const RTP_REORDER_MAX_PACKETS: usize = 64;
const RTP_MAX_CONCEALMENT_PACKETS: u16 = 8;
/// Longest single concealment frame, in seconds of audio. Recovery work needs
/// a memory bound that does not depend on peer timestamps: one second is
/// generous for one lost packet, and a larger discontinuity keeps the media
/// that was received without synthesizing a giant frame for it.
const RTP_MAX_CONCEALMENT_SECONDS: usize = 1;
/// RTP timestamp differences at or beyond half the 32-bit space are backward
/// steps, not elapsed time.
const RTP_TIMESTAMP_HALF_RANGE: u32 = 1 << 31;

/// Interleaved sample count for one concealment frame, or `None` when the
/// frame would be empty or longer than [`RTP_MAX_CONCEALMENT_SECONDS`].
fn concealment_sample_count(
    samples_per_channel: usize,
    clock_rate: u32,
    channels: u8,
) -> Option<usize> {
    let maximum = usize::try_from(clock_rate)
        .ok()?
        .checked_mul(RTP_MAX_CONCEALMENT_SECONDS)?;
    if samples_per_channel == 0 || samples_per_channel > maximum || channels == 0 {
        return None;
    }
    samples_per_channel.checked_mul(usize::from(channels))
}

#[derive(Debug)]
struct PendingRtpPacket {
    packet: RtpPacket,
}

#[derive(Debug)]
enum RtpPlayoutItem {
    Packet(RtpPacket),
    Gap {
        sequence_number: u16,
        timestamp: u32,
        samples_per_channel: usize,
    },
}

/// Small per-dialog reorder window at the media decode boundary.
///
/// rtp-core currently exposes its jitter-buffer settings but does not connect
/// them to the event-driven session path. Keeping this stage here preserves RTP
/// sequence/timestamp information until packets are ordered, while still
/// delivering decoded PCM through the existing bounded callback channel.
struct RtpPlayoutBuffer {
    active_ssrc: Option<u32>,
    pending: BTreeMap<u16, PendingRtpPacket>,
    expected_sequence: Option<u16>,
    startup_started_at: Option<Instant>,
    gap_started_at: Option<Instant>,
    last_timestamp: Option<u32>,
    timestamp_step: Option<u32>,
}

impl RtpPlayoutBuffer {
    fn new() -> Self {
        Self {
            active_ssrc: None,
            pending: BTreeMap::new(),
            expected_sequence: None,
            startup_started_at: None,
            gap_started_at: None,
            last_timestamp: None,
            timestamp_step: None,
        }
    }

    fn push(&mut self, packet: RtpPacket, now: Instant) -> Vec<RtpPlayoutItem> {
        let mut ready = Vec::new();
        self.push_into(packet, now, &mut ready);
        ready
    }

    /// [`Self::push`], appending the released items to `ready`, so the
    /// inline receive path reuses one buffer instead of a `Vec` per release.
    fn push_into(&mut self, packet: RtpPacket, now: Instant, ready: &mut Vec<RtpPlayoutItem>) {
        let ssrc = packet.header.ssrc;
        if self.active_ssrc.is_some_and(|active| active != ssrc) {
            // A re-INVITE can replace a temporary media relay with the final
            // peer while keeping the dialog and payload type unchanged. RTP
            // sequence and timestamp spaces belong to an SSRC, so retaining
            // the former source's reorder state would classify the new stream
            // as late and discard it indefinitely.
            self.pending.clear();
            self.expected_sequence = None;
            self.startup_started_at = None;
            self.gap_started_at = None;
            self.last_timestamp = None;
            self.timestamp_step = None;
        }
        self.active_ssrc = Some(ssrc);
        let sequence = packet.header.sequence_number;
        if self.expected_sequence.is_some_and(|expected| {
            sequence != expected && sequence.wrapping_sub(expected) >= 32_768
        }) {
            return;
        }
        self.pending
            .entry(sequence)
            .or_insert(PendingRtpPacket { packet });
        self.startup_started_at.get_or_insert(now);

        self.tick_into(now, ready)
    }

    fn tick(&mut self, now: Instant) -> Vec<RtpPlayoutItem> {
        let mut ready = Vec::new();
        self.tick_into(now, &mut ready);
        ready
    }

    /// [`Self::tick`], appending the released items to `ready`.
    fn tick_into(&mut self, now: Instant, ready: &mut Vec<RtpPlayoutItem>) {
        if self.expected_sequence.is_none() {
            let Some(started_at) = self.startup_started_at else {
                return;
            };
            // Hold the stream start only while the pending packets show a
            // gap. An unbroken run carries no evidence of reordering, and
            // holding it would cost every call up to RTP_REORDER_HOLD of
            // first-frame latency, which an application that answers on the
            // first delivered early-media frame cannot afford.
            let startup_elapsed = now.saturating_duration_since(started_at);
            if !self.pending_is_contiguous()
                && startup_elapsed < RTP_REORDER_HOLD
                && self.pending.len() < RTP_REORDER_MAX_PACKETS
            {
                return;
            }
            self.expected_sequence = self.oldest_pending_sequence();
        }

        self.drain_ready_into(now, ready)
    }

    /// When a startup hold or a gap hold expires, if one is pending after
    /// `after`. An expired gap that cannot advance (for example, one with no
    /// timestamp progression) needs another packet to supply information;
    /// polling it again cannot help, so it arms nothing.
    fn next_deadline(&self, after: Instant) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        let started = if self.expected_sequence.is_none() {
            self.startup_started_at
        } else {
            self.gap_started_at
        }?;
        let deadline = started + RTP_REORDER_HOLD;
        (deadline > after).then_some(deadline)
    }

    /// True when the pending packets form one unbroken sequence run.
    fn pending_is_contiguous(&self) -> bool {
        let Some(oldest) = self.oldest_pending_sequence() else {
            return true;
        };
        (0..self.pending.len() as u16)
            .all(|offset| self.pending.contains_key(&oldest.wrapping_add(offset)))
    }

    fn oldest_pending_sequence(&self) -> Option<u16> {
        self.pending.keys().copied().find(|candidate| {
            self.pending
                .keys()
                .all(|other| other == candidate || other.wrapping_sub(*candidate) < 32_768)
        })
    }

    fn drain_ready_into(&mut self, now: Instant, ready: &mut Vec<RtpPlayoutItem>) {
        loop {
            let Some(expected) = self.expected_sequence else {
                break;
            };
            if let Some(buffered) = self.pending.remove(&expected) {
                self.observe_timestamp(buffered.packet.header.timestamp);
                self.expected_sequence = Some(expected.wrapping_add(1));
                self.gap_started_at = None;
                ready.push(RtpPlayoutItem::Packet(buffered.packet));
                continue;
            }

            let nearest = self
                .pending
                .iter()
                .filter_map(|(sequence, buffered)| {
                    let distance = sequence.wrapping_sub(expected);
                    (distance > 0 && distance < 32_768).then_some((distance, *sequence, buffered))
                })
                .min_by_key(|(distance, _, _)| *distance);
            let Some((distance, nearest_sequence, nearest_packet)) = nearest else {
                break;
            };
            let missing_since = *self.gap_started_at.get_or_insert(now);
            if now.saturating_duration_since(missing_since) < RTP_REORDER_HOLD
                && self.pending.len() < RTP_REORDER_MAX_PACKETS
            {
                break;
            }

            if distance > RTP_MAX_CONCEALMENT_PACKETS {
                self.expected_sequence = Some(nearest_sequence);
                self.gap_started_at = None;
                continue;
            }

            let Some(step) = self.timestamp_step.or_else(|| {
                self.last_timestamp.and_then(|previous| {
                    let elapsed = nearest_packet
                        .packet
                        .header
                        .timestamp
                        .wrapping_sub(previous);
                    // `distance` counts the missing packets after the last
                    // played one; the received successor spans one more
                    // interval than that.
                    (elapsed < RTP_TIMESTAMP_HALF_RANGE)
                        .then_some(elapsed / (u32::from(distance) + 1))
                })
            }) else {
                // A backward or discontinuous timestamp cannot size the gap,
                // and dividing it by the distance once asked for a ~2^32
                // sample frame. Resume at the received media and learn the
                // cadence again from what follows.
                self.expected_sequence = Some(nearest_sequence);
                self.gap_started_at = None;
                self.last_timestamp = None;
                continue;
            };
            if step == 0 {
                break;
            }
            let timestamp = self
                .last_timestamp
                .map_or(nearest_packet.packet.header.timestamp, |previous| {
                    previous.wrapping_add(step)
                });
            self.last_timestamp = Some(timestamp);
            self.timestamp_step = Some(step);
            self.expected_sequence = Some(expected.wrapping_add(1));
            // The hold is per gap, not per missing packet: the next missing
            // packet of the same gap has waited as long as this one, so it is
            // concealed in this pass. Restarting the hold here held every
            // packet behind an N-packet loss for N holds (five lost packets
            // kept the stream 225 ms late).
            ready.push(RtpPlayoutItem::Gap {
                sequence_number: expected,
                timestamp,
                samples_per_channel: step as usize,
            });
        }
    }

    fn observe_timestamp(&mut self, timestamp: u32) {
        if let Some(previous) = self.last_timestamp {
            let step = timestamp.wrapping_sub(previous);
            // A backward clock step on the same SSRC is not a frame size; a
            // concealment frame sized from it would be gigabytes of silence.
            if step > 0 && step < RTP_TIMESTAMP_HALF_RANGE {
                self.timestamp_step = Some(step);
            }
        }
        self.last_timestamp = Some(timestamp);
    }

    /// Inline receive fast path. True when [`Self::push`] of this packet
    /// would return exactly `[Packet(this)]` and leave nothing pending: the
    /// same SSRC, the expected sequence and an empty reorder window. The
    /// caller may then decode straight from its receive buffer and apply
    /// [`Self::release_in_order`] instead of building an owned packet.
    fn releases_immediately(&self, ssrc: u32, sequence: u16) -> bool {
        self.active_ssrc == Some(ssrc)
            && self.expected_sequence == Some(sequence)
            && self.pending.is_empty()
    }

    /// The exact state change `push` and `drain_ready_into` make for a
    /// packet that [`Self::releases_immediately`] accepted.
    fn release_in_order(&mut self, sequence: u16, timestamp: u32, now: Instant) {
        self.startup_started_at.get_or_insert(now);
        self.observe_timestamp(timestamp);
        self.expected_sequence = Some(sequence.wrapping_add(1));
        self.gap_started_at = None;
    }

    /// Inline receive demotion: hand back the packets held for reordering,
    /// oldest first, so the v1 handler can continue the stream.
    fn take_pending_in_order(&mut self) -> Vec<RtpPacket> {
        let base = self
            .expected_sequence
            .or_else(|| self.oldest_pending_sequence());
        let mut held: Vec<RtpPacket> = std::mem::take(&mut self.pending)
            .into_values()
            .map(|pending| pending.packet)
            .collect();
        if let Some(base) = base {
            held.sort_by_key(|packet| packet.header.sequence_number.wrapping_sub(base));
        }
        held
    }
}

/// Paces frames that the reorder window releases together (a held gap and the
/// packets queued behind it) one frame duration apart.
///
/// Pacing alone cannot drain a backlog while the stream keeps arriving: live
/// frames arrive one frame duration apart and the paced head leaves one frame
/// duration apart, so a batch of N frames would stay N - 1 frames deep, and
/// add that latency, for the rest of the call. A playout clock downstream
/// keeps its own schedule, so that standing delay turns every later frame
/// into a late frame there: one lost inbound packet silenced the caller's
/// audio for the rest of the call. A frame queued at a later instant than the
/// previous one while a backlog is waiting is therefore live media behind
/// paced media, and earns one extra delivery that does not move the paced
/// schedule. A batch queued at one instant is still paced; a backlog drains
/// at two frames per frame duration and the queue converges back to
/// delivering every frame on arrival.
struct DecodedPlayoutQueue {
    pending: VecDeque<AudioFrame>,
    next_delivery_at: Option<Instant>,
    /// When the newest queued frame was pushed.
    last_push_at: Option<Instant>,
    /// Deliveries owed to live arrivals that found a backlog. Never more
    /// than the frames queued ahead of them.
    catch_up: usize,
    /// Frames dropped on overflow. The first drop is logged; the rest are
    /// counted so a burst does not flood the log.
    dropped_frames: u64,
    /// Pace the next frame from the deadline just served rather than from
    /// the time it was served. On when the lane tick is active
    /// (`rvoip_rtp_core::lane_tick`), where deadlines are served up to one
    /// quantum late; identical to the other rule when a deadline is served
    /// on time.
    anchored: bool,
}

impl DecodedPlayoutQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            next_delivery_at: None,
            last_push_at: None,
            catch_up: 0,
            dropped_frames: 0,
            anchored: rvoip_rtp_core::lane_tick::lane_tick_pacing_enabled(),
        }
    }

    fn push(&mut self, frame: AudioFrame, now: Instant) -> bool {
        if self.pending.len() >= RTP_REORDER_MAX_PACKETS {
            // The dropped frame's PCM goes back to the pool (simply dropped,
            // as before, while the pool is off).
            crate::pcm_pool::recycle_pcm_buffer(frame.samples);
            return false;
        }
        if self.next_delivery_at.is_none() {
            self.next_delivery_at = Some(now);
        }
        if !self.pending.is_empty() && self.last_push_at.is_some_and(|last| now > last) {
            self.catch_up = (self.catch_up + 1).min(self.pending.len());
        }
        self.last_push_at = Some(now);
        self.pending.push_back(frame);
        true
    }

    fn pop_due(&mut self, now: Instant) -> Option<AudioFrame> {
        let deadline = self.next_delivery_at?;
        if self.catch_up > 0 {
            if let Some(frame) = self.pending.pop_front() {
                // Owed to a live arrival: the paced schedule stays as it is.
                self.catch_up -= 1;
                if self.pending.is_empty() {
                    self.next_delivery_at = None;
                    self.catch_up = 0;
                }
                return Some(frame);
            }
            self.catch_up = 0;
        }
        if now < deadline {
            return None;
        }
        let frame = self.pending.pop_front()?;
        let step = frame.duration.max(Duration::from_millis(1));
        // A lane tick serves a deadline up to one quantum late; pacing from
        // the served deadline keeps that from accumulating frame after
        // frame. A wake later than a whole frame (a stall) still restarts the
        // schedule from now, so a stall never becomes a burst.
        let base = if self.anchored && now.saturating_duration_since(deadline) < step {
            deadline
        } else {
            now
        };
        self.next_delivery_at = self.pending.front().map(|_| base + step);
        if self.pending.is_empty() {
            self.catch_up = 0;
        }
        Some(frame)
    }

    /// When the next frame is due, while any frame is queued. A delivery
    /// owed to a live arrival is due as soon as that arrival was queued.
    fn next_deadline(&self) -> Option<Instant> {
        if self.pending.is_empty() {
            return None;
        }
        if self.catch_up > 0 {
            return match (self.next_delivery_at, self.last_push_at) {
                (Some(paced), Some(owed)) => Some(paced.min(owed)),
                (paced, owed) => paced.or(owed),
            };
        }
        self.next_delivery_at
    }
}

#[cfg(test)]
mod rtp_playout_tests {
    use super::*;
    use bytes::Bytes;
    use rvoip_rtp_core::RtpHeader;

    fn packet(sequence: u16) -> RtpPacket {
        packet_for_ssrc(sequence, 0x0102_0304)
    }

    fn packet_for_ssrc(sequence: u16, ssrc: u32) -> RtpPacket {
        RtpPacket {
            header: RtpHeader::new(0, sequence, u32::from(sequence) * 160, ssrc),
            payload: Bytes::from(vec![0xff; 160]),
            padding_size: 0,
        }
    }

    #[test]
    fn ssrc_handoff_resets_the_sequence_and_timestamp_space() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let first = buffer.push(packet_for_ssrc(50_000, 0x1111_1111), start);
        assert_eq!(first.iter().map(sequence).collect::<Vec<_>>(), [50_000]);
        assert!(buffer.tick(start + Duration::from_millis(46)).is_empty());

        // A new SSRC resets the sequence space: 100 is not "late" relative to
        // 50_000, it is the first packet of a new stream and leaves at once.
        let handoff = buffer.push(
            packet_for_ssrc(100, 0x2222_2222),
            start + Duration::from_millis(100),
        );
        assert_eq!(handoff.iter().map(sequence).collect::<Vec<_>>(), [100]);
        assert!(buffer.tick(start + Duration::from_millis(146)).is_empty());
    }

    fn sequence(item: &RtpPlayoutItem) -> u16 {
        match item {
            RtpPlayoutItem::Packet(packet) => packet.header.sequence_number,
            RtpPlayoutItem::Gap {
                sequence_number, ..
            } => *sequence_number,
        }
    }

    #[test]
    fn reorder_deadline_exists_only_while_a_gap_can_wait() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        assert_eq!(buffer.next_deadline(start), None);
        buffer.push(packet(100), start);
        buffer.push(packet(101), start + Duration::from_millis(20));
        assert_eq!(
            buffer.next_deadline(start + Duration::from_millis(20)),
            None
        );
        let gap_at = start + Duration::from_millis(40);
        assert!(buffer.push(packet(103), gap_at).is_empty());
        assert_eq!(
            buffer.next_deadline(gap_at),
            Some(gap_at + RTP_REORDER_HOLD)
        );
        let ready = buffer.tick(gap_at + RTP_REORDER_HOLD);
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [102, 103]);
        assert_eq!(buffer.next_deadline(gap_at + RTP_REORDER_HOLD), None);
    }

    #[test]
    fn expired_gap_without_timestamp_progress_does_not_rearm_a_busy_timer() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let mut first = packet(100);
        first.header.timestamp = 0;
        buffer.push(first, start);
        let mut next = packet(102);
        next.header.timestamp = 0;
        assert!(buffer.push(next, start).is_empty());
        assert!(buffer.tick(start + RTP_REORDER_HOLD).is_empty());
        assert_eq!(buffer.next_deadline(start + RTP_REORDER_HOLD), None);
    }

    #[test]
    fn startup_releases_a_contiguous_first_packet_without_holding_it() {
        // The first packet of a stream carries no evidence of reordering, so
        // it is not held for RTP_REORDER_HOLD. An application that answers a
        // call on the first delivered early-media frame depends on this.
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let ready = buffer.push(packet(101), start);
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [101]);
        // A lower sequence arriving afterwards is late relative to a stream
        // that has already started, and is discarded rather than reordered.
        assert!(buffer
            .push(packet(100), start + Duration::from_millis(35))
            .is_empty());
        assert!(buffer.tick(start + Duration::from_millis(46)).is_empty());
    }

    #[test]
    fn startup_holds_only_once_a_gap_is_observed() {
        // Reorder protection is still in force from the first observed gap:
        // 103 arriving ahead of 102 waits, and both leave in order once 102
        // arrives inside the hold.
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        assert_eq!(
            buffer
                .push(packet(101), start)
                .iter()
                .map(sequence)
                .collect::<Vec<_>>(),
            [101]
        );
        assert!(buffer
            .push(packet(103), start + Duration::from_millis(20))
            .is_empty());
        let ready = buffer.push(packet(102), start + Duration::from_millis(35));
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [102, 103]);
    }

    #[test]
    fn active_window_waits_for_a_delayed_predecessor() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let mut initial = Vec::new();
        initial.extend(buffer.push(packet(100), start));
        initial.extend(buffer.push(packet(101), start + Duration::from_millis(20)));
        initial.extend(buffer.push(packet(102), start + Duration::from_millis(40)));
        assert_eq!(
            initial.iter().map(sequence).collect::<Vec<_>>(),
            [100, 101, 102]
        );
        assert!(buffer.tick(start + Duration::from_millis(46)).is_empty());
        assert!(buffer
            .push(packet(104), start + Duration::from_millis(80))
            .is_empty());
        let ready = buffer.push(packet(103), start + Duration::from_millis(95));
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [103, 104]);
    }

    #[test]
    fn confirmed_gap_advances_one_timestamp_sized_playout_slot() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        buffer.push(packet(100), start);
        buffer.push(packet(101), start + Duration::from_millis(20));
        buffer.push(packet(102), start + Duration::from_millis(40));
        buffer.tick(start + Duration::from_millis(46));
        assert!(buffer
            .push(packet(104), start + Duration::from_millis(80))
            .is_empty());
        let ready = buffer.tick(start + Duration::from_millis(126));
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [103, 104]);
        assert!(matches!(
            &ready[0],
            RtpPlayoutItem::Gap {
                timestamp,
                samples_per_channel: 160,
                ..
            } if *timestamp == 103 * 160
        ));
    }

    #[test]
    fn backward_clock_step_does_not_size_a_concealment_frame() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let at = |sequence: u16, timestamp: u32| {
            let mut packet = packet(sequence);
            packet.header.timestamp = timestamp;
            packet
        };
        buffer.push(at(100, 1_000_000), start);
        buffer.push(at(101, 1_000_160), start + Duration::from_millis(20));
        // Same SSRC resets its clock, then the next packet is lost.
        buffer.push(at(102, 0), start + Duration::from_millis(40));
        buffer.push(at(104, 320), start + Duration::from_millis(80));
        let ready = buffer.tick(start + Duration::from_millis(126));
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [103, 104]);
        assert!(matches!(
            &ready[0],
            RtpPlayoutItem::Gap {
                timestamp: 160,
                samples_per_channel: 160,
                ..
            }
        ));
    }

    #[test]
    fn backward_step_before_cadence_is_learned_does_not_size_a_gap() {
        // Only one packet has played when the gap is confirmed, so no
        // timestamp step is known yet and the gap is sized from the
        // successor. Its timestamp is behind the last played one (a same-SSRC
        // clock reset), and dividing that backward step by the distance asked
        // for a concealment frame of about 2^32 samples: an 8.6 GiB zeroed
        // allocation downstream. Inspect metadata only, so this test never
        // allocates the frame it guards against.
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let mut first = packet(100);
        first.header.timestamp = 16_000;
        assert_eq!(
            buffer
                .push(first, start)
                .iter()
                .map(sequence)
                .collect::<Vec<_>>(),
            [100]
        );
        let mut successor = packet(102);
        successor.header.timestamp = 0;
        let lost_at = start + Duration::from_millis(40);
        assert!(buffer.push(successor, lost_at).is_empty());
        let ready = buffer.tick(lost_at + RTP_REORDER_HOLD);
        for item in &ready {
            if let RtpPlayoutItem::Gap {
                samples_per_channel,
                ..
            } = item
            {
                assert!(
                    *samples_per_channel <= 8_000,
                    "unbounded concealment frame: {samples_per_channel} samples"
                );
            }
        }
        // Received media resumes at the successor. Nothing is synthesized for
        // a gap whose duration the timestamps cannot establish.
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [102]);
        assert!(matches!(&ready[0], RtpPlayoutItem::Packet(_)));
        // The cadence is learned again from the media that follows.
        let mut next = packet(103);
        next.header.timestamp = 160;
        assert_eq!(
            buffer
                .push(next, lost_at + Duration::from_millis(60))
                .iter()
                .map(sequence)
                .collect::<Vec<_>>(),
            [103]
        );
    }

    #[test]
    fn first_gap_infers_duration_from_missing_and_received_intervals() {
        // 101 is lost before any step is known. The received successor is two
        // intervals after the last played packet, not one, so the concealment
        // frame is one interval long and does not reuse the successor's
        // timestamp.
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        buffer.push(packet(100), start);
        buffer.push(packet(102), start);
        let released = buffer.tick(start + RTP_REORDER_HOLD);
        assert_eq!(
            released.iter().map(sequence).collect::<Vec<_>>(),
            [101, 102]
        );
        assert!(matches!(
            &released[0],
            RtpPlayoutItem::Gap {
                timestamp: 16_160,
                samples_per_channel: 160,
                ..
            }
        ));
    }

    #[test]
    fn forward_timestamp_wrap_preserves_normal_concealment() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let mut first = packet(100);
        first.header.timestamp = u32::MAX - 159;
        buffer.push(first, start);
        let mut successor = packet(102);
        successor.header.timestamp = 160;
        buffer.push(successor, start);
        let released = buffer.tick(start + RTP_REORDER_HOLD);
        assert!(matches!(
            &released[0],
            RtpPlayoutItem::Gap {
                timestamp: 0,
                samples_per_channel: 160,
                ..
            }
        ));
    }

    #[test]
    fn concealment_allocation_is_bounded_by_negotiated_audio_format() {
        assert_eq!(concealment_sample_count(160, 8_000, 1), Some(160));
        assert_eq!(concealment_sample_count(960, 48_000, 2), Some(1_920));
        assert_eq!(concealment_sample_count(8_000, 8_000, 1), Some(8_000));
        assert_eq!(concealment_sample_count(4_294_967_136, 8_000, 1), None);
        assert_eq!(concealment_sample_count(1_000_000_000, 48_000, 2), None);
        assert_eq!(concealment_sample_count(8_001, 8_000, 1), None);
        assert_eq!(concealment_sample_count(0, 8_000, 1), None);
        assert_eq!(concealment_sample_count(160, 8_000, 0), None);
    }

    #[tokio::test]
    async fn oversized_gap_does_not_prevent_following_real_g711_audio() {
        // A concealment request longer than the bound is refused without
        // allocating it, and the received packet behind it still plays.
        for codec in ["PCMU", "PCMA"] {
            let config = MediaConfig {
                local_addr: "127.0.0.1:0".parse().expect("local addr"),
                remote_addr: None,
                preferred_codec: Some(codec.to_string()),
                parameters: std::collections::HashMap::new(),
            };
            let format = codec_runtime::resolve_codec(&config).expect("G.711 resolves");
            let payload_type = format.payload_type;
            let runtime =
                Arc::new(codec_runtime::DialogCodecRuntime::new(format).expect("codec runtime"));
            let mut real = packet(11);
            real.header.payload_type = payload_type;
            let mut queue = DecodedPlayoutQueue::new();
            let now = Instant::now();
            let terminate = enqueue_rtp_playout_items(
                vec![
                    RtpPlayoutItem::Gap {
                        sequence_number: 10,
                        timestamp: 1_600,
                        samples_per_channel: 8_001,
                    },
                    RtpPlayoutItem::Packet(real),
                ],
                &runtime,
                &DialogId::new("bounded-concealment"),
                &mut queue,
                &mut source_timeline::SourceTimeline::default(),
                now,
            )
            .await;
            assert!(!terminate, "{codec}");
            assert_eq!(queue.pending.len(), 1, "{codec}: only received media");
            let frame = queue
                .pop_due(now)
                .expect("received media survives a refused concealment");
            assert_eq!(frame.samples.len(), 160, "{codec}");
            assert_eq!(frame.timestamp, 1_760, "{codec}");
        }
    }

    #[test]
    fn consecutive_losses_are_concealed_after_one_hold() {
        // 102, 103 and 104 are lost. One hold after the gap opened, the whole
        // gap is concealed and 105 leaves with it. Restarting the hold for
        // each missing packet made 105 wait three holds.
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        buffer.push(packet(100), start);
        buffer.push(packet(101), start + Duration::from_millis(20));
        let gap_at = start + Duration::from_millis(80);
        assert!(buffer.push(packet(105), gap_at).is_empty());
        let ready = buffer.tick(gap_at + RTP_REORDER_HOLD);
        assert_eq!(
            ready.iter().map(sequence).collect::<Vec<_>>(),
            [102, 103, 104, 105]
        );
        let concealed: Vec<u32> = ready
            .iter()
            .filter_map(|item| match item {
                RtpPlayoutItem::Gap { timestamp, .. } => Some(*timestamp),
                RtpPlayoutItem::Packet(_) => None,
            })
            .collect();
        assert_eq!(concealed, [102 * 160, 103 * 160, 104 * 160]);
        assert_eq!(buffer.next_deadline(gap_at + RTP_REORDER_HOLD), None);
        // The next gap waits its own hold again.
        let second_gap = gap_at + Duration::from_millis(100);
        assert!(buffer.push(packet(107), second_gap).is_empty());
        assert!(buffer
            .tick(second_gap + Duration::from_millis(44))
            .is_empty());
        assert_eq!(
            buffer
                .tick(second_gap + RTP_REORDER_HOLD)
                .iter()
                .map(sequence)
                .collect::<Vec<_>>(),
            [106, 107]
        );
    }

    #[test]
    fn duplicate_and_late_packets_do_not_reenter_playout() {
        let start = Instant::now();
        let mut buffer = RtpPlayoutBuffer::new();
        let ready = buffer.push(packet(100), start);
        assert_eq!(ready.iter().map(sequence).collect::<Vec<_>>(), [100]);
        assert!(buffer.tick(start + Duration::from_millis(46)).is_empty());
        assert!(buffer
            .push(packet(100), start + Duration::from_millis(50))
            .is_empty());
    }

    #[test]
    fn decoded_playout_paces_a_reordered_batch() {
        let start = Instant::now();
        let mut playout = DecodedPlayoutQueue::new();
        for timestamp in [0, 160, 320] {
            assert!(playout.push(AudioFrame::new(vec![0; 160], 8_000, 1, timestamp), start,));
        }
        assert_eq!(playout.next_deadline(), Some(start));
        assert_eq!(playout.pop_due(start).expect("first frame").timestamp, 0);
        assert_eq!(
            playout.next_deadline(),
            Some(start + Duration::from_millis(20))
        );
        assert!(playout.pop_due(start + Duration::from_millis(19)).is_none());
        assert_eq!(
            playout
                .pop_due(start + Duration::from_millis(20))
                .expect("second frame")
                .timestamp,
            160
        );
    }

    #[test]
    fn decoded_playout_does_not_keep_a_reorder_batch_as_standing_latency() {
        // One lost inbound packet. The reorder window holds the packets
        // behind it for RTP_REORDER_HOLD and releases the gap and those
        // packets together, while live packets keep arriving every 20 ms.
        // Driven the way the receive loop drives it (one due frame on every
        // packet and on every armed deadline), the batch must drain and every
        // later frame must again leave on arrival. Pacing alone kept the batch
        // queued for the rest of the call, so every later frame reached the
        // application's playout clock about 60 ms late.
        let start = Instant::now();
        let frame_interval = Duration::from_millis(20);
        let arrival = |sequence: u16| {
            start + frame_interval * u32::from(sequence - 100) + Duration::from_millis(5)
        };
        let decoded = |item: RtpPlayoutItem| match item {
            RtpPlayoutItem::Packet(packet) => {
                AudioFrame::new(vec![0; 160], 8_000, 1, packet.header.timestamp)
            }
            RtpPlayoutItem::Gap { timestamp, .. } => {
                AudioFrame::new(vec![0; 160], 8_000, 1, timestamp)
            }
        };
        let lost = 110_u16;
        let mut reorder = RtpPlayoutBuffer::new();
        let mut playout = DecodedPlayoutQueue::new();
        let mut delivered: Vec<(u32, Instant)> = Vec::new();
        let mut now = start;
        for sequence in 100_u16..200 {
            let next_arrival = arrival(sequence);
            // Serve every deadline armed before this arrival, as the timer would.
            for _ in 0..1_000 {
                let deadline = match (reorder.next_deadline(now), playout.next_deadline()) {
                    (Some(reorder_at), Some(audio_at)) => Some(reorder_at.min(audio_at)),
                    (reorder_at, audio_at) => reorder_at.or(audio_at),
                };
                let Some(deadline) = deadline.filter(|deadline| *deadline < next_arrival) else {
                    break;
                };
                now = now.max(deadline);
                for item in reorder.tick(now) {
                    assert!(playout.push(decoded(item), now));
                }
                if let Some(frame) = playout.pop_due(now) {
                    delivered.push((frame.timestamp, now));
                }
            }
            now = next_arrival;
            if sequence == lost {
                continue;
            }
            for item in reorder.push(packet(sequence), now) {
                assert!(playout.push(decoded(item), now));
            }
            if let Some(frame) = playout.pop_due(now) {
                delivered.push((frame.timestamp, now));
            }
        }

        // Every frame once, in order; the lost one as its concealment.
        let timestamps: Vec<u32> = delivered.iter().map(|(timestamp, _)| *timestamp).collect();
        let expected: Vec<u32> = (100_u32..200).map(|sequence| sequence * 160).collect();
        assert_eq!(timestamps, expected);
        // Once the batch is spent, frames leave on arrival again, so the
        // stream carries no standing delay into the playout clock.
        for (timestamp, at) in &delivered {
            let sequence = u16::try_from(*timestamp / 160).expect("sequence");
            if sequence >= lost + 10 {
                assert_eq!(
                    *at,
                    arrival(sequence),
                    "frame {sequence} left {:?} after it arrived",
                    at.saturating_duration_since(arrival(sequence))
                );
            }
        }
        assert!(playout.pending.is_empty());
    }

    #[test]
    fn decoded_playout_catch_up_never_reorders_or_outruns_the_queue() {
        // A live arrival behind a paced batch earns one delivery; a batch
        // queued at one instant earns none, and owed deliveries never exceed
        // the frames queued ahead.
        let start = Instant::now();
        let mut playout = DecodedPlayoutQueue::new();
        for timestamp in [0, 160, 320] {
            assert!(playout.push(AudioFrame::new(vec![0; 160], 8_000, 1, timestamp), start));
        }
        assert_eq!(playout.catch_up, 0, "one batch is paced, not caught up");
        assert_eq!(playout.pop_due(start).expect("head").timestamp, 0);
        let live = start + Duration::from_millis(5);
        assert!(playout.push(AudioFrame::new(vec![0; 160], 8_000, 1, 480), live));
        assert_eq!(
            playout.next_deadline(),
            Some(live),
            "an owed delivery is due now"
        );
        assert_eq!(playout.pop_due(live).expect("owed").timestamp, 160);
        assert!(
            playout.pop_due(live).is_none(),
            "the paced schedule (start + 20 ms) is not moved by the owed delivery"
        );
        assert_eq!(
            playout
                .pop_due(start + Duration::from_millis(20))
                .expect("paced")
                .timestamp,
            320
        );
        assert_eq!(
            playout
                .pop_due(start + Duration::from_millis(40))
                .expect("paced")
                .timestamp,
            480
        );
        assert!(playout.pending.is_empty());
        assert_eq!(playout.catch_up, 0);
        assert_eq!(playout.next_deadline(), None);
    }

    #[test]
    fn decoded_playout_does_not_invent_audio_across_a_talkspurt_pause() {
        let start = Instant::now();
        let mut playout = DecodedPlayoutQueue::new();
        assert!(playout.push(AudioFrame::new(vec![0; 160], 8_000, 1, 0), start));
        assert!(playout.pop_due(start).is_some());
        assert!(playout.pop_due(start + Duration::from_secs(5)).is_none());

        assert!(playout.push(
            AudioFrame::new(vec![1; 160], 8_000, 1, 40_000),
            start + Duration::from_secs(5),
        ));
        assert_eq!(
            playout
                .pop_due(start + Duration::from_secs(5))
                .expect("new talkspurt")
                .timestamp,
            40_000
        );
    }

    /// Lane tick: a paced batch whose deadlines are served on lane ticks
    /// (each at the first boundary of a fake-clock grid at or after it, plus
    /// the lane thread's wake-up latency) delivers the same frames in the
    /// same order. Anchored pacing keeps every delivery within one quantum
    /// plus that latency of the exact schedule; pacing from the wake time
    /// drifts by up to a quantum per frame.
    #[test]
    fn tick_served_pacing_stays_within_one_quantum_only_when_anchored() {
        let start = Instant::now();
        let wake = Duration::from_micros(60);
        let frames = 12u32;
        let deliver = |anchored: bool, quantum: Option<Duration>| -> Vec<(u32, Instant)> {
            let mut playout = DecodedPlayoutQueue::new();
            playout.anchored = anchored;
            for index in 0..frames {
                let samples = vec![index as i16; 160];
                assert!(playout.push(AudioFrame::new(samples, 8_000, 1, index * 160), start));
            }
            // A grid whose boundaries are 0.7 ms out of phase with the batch.
            let grid = quantum.map(|quantum| {
                rvoip_rtp_core::lane_tick::TickGrid::new(
                    start - Duration::from_micros(700),
                    quantum,
                )
            });
            let mut delivered = Vec::new();
            let mut now = start;
            for _ in 0..1_000 {
                while let Some(frame) = playout.pop_due(now) {
                    assert!(frame
                        .samples
                        .iter()
                        .all(|sample| *sample == (frame.timestamp / 160) as i16));
                    delivered.push((frame.timestamp, now));
                }
                let Some(deadline) = playout.next_deadline() else {
                    break;
                };
                now = match grid {
                    Some(grid) => grid.ceil(deadline) + wake,
                    None => deadline,
                };
            }
            delivered
        };
        let exact = deliver(false, None);
        assert_eq!(
            exact,
            deliver(true, None),
            "anchoring changes nothing on time"
        );
        let timestamps: Vec<u32> = (0..frames).map(|index| index * 160).collect();
        assert_eq!(
            exact
                .iter()
                .map(|(timestamp, _)| *timestamp)
                .collect::<Vec<_>>(),
            timestamps
        );
        for quantum_ms in rvoip_rtp_core::lane_tick::LANE_TICK_QUANTA_MS {
            let quantum = Duration::from_millis(quantum_ms);
            let ticked = deliver(true, Some(quantum));
            assert_eq!(
                ticked
                    .iter()
                    .map(|(timestamp, _)| *timestamp)
                    .collect::<Vec<_>>(),
                timestamps,
                "same frames, same order"
            );
            for ((_, on_time), (_, served)) in exact.iter().zip(&ticked) {
                assert!(served >= on_time, "never early");
                let late = served.saturating_duration_since(*on_time);
                assert!(late <= quantum + wake, "Q={quantum:?}: {late:?} late");
            }
            let drifting = deliver(false, Some(quantum));
            let worst = exact
                .iter()
                .zip(&drifting)
                .map(|((_, on_time), (_, served))| served.saturating_duration_since(*on_time))
                .max()
                .unwrap();
            assert!(
                worst > quantum + wake,
                "Q={quantum:?}: wake-time pacing drifts ({worst:?})"
            );
        }
        // A stall longer than a frame restarts the schedule from now: no burst.
        let mut playout = DecodedPlayoutQueue::new();
        playout.anchored = true;
        for timestamp in [0, 160, 320] {
            assert!(playout.push(AudioFrame::new(vec![0; 160], 8_000, 1, timestamp), start));
        }
        assert!(playout.pop_due(start).is_some());
        let stalled = start + Duration::from_millis(70);
        assert!(playout.pop_due(stalled).is_some());
        assert!(
            playout.pop_due(stalled).is_none(),
            "one frame per deadline after a stall"
        );
        assert_eq!(
            playout.next_deadline(),
            Some(stalled + Duration::from_millis(20))
        );
    }

    #[test]
    fn decoded_pacing_is_anchored_only_while_the_lane_tick_is_active() {
        use rvoip_rtp_core::executor::{
            override_media_executor_config_on_this_thread, MediaExecutorConfig,
        };
        for (config, anchored) in [
            (MediaExecutorConfig::v1(), false),
            (MediaExecutorConfig::v2(), true),
            (MediaExecutorConfig::v2().with_lane_tick(None), false),
            (MediaExecutorConfig::v2().with_lane_poller(false), false),
        ] {
            let _guard = override_media_executor_config_on_this_thread(config.clone());
            assert_eq!(DecodedPlayoutQueue::new().anchored, anchored, "{config:?}");
        }
    }

    #[tokio::test]
    async fn decoded_playout_overflow_drops_frames_but_keeps_the_receive_loop_alive() {
        // A bridge that releases media buffered until commit, or any producer
        // running ahead of real time, can hand the decode boundary more than
        // RTP_REORDER_MAX_PACKETS frames in one drain. That must cost frames,
        // never the dialog's receive loop.
        let config = MediaConfig {
            local_addr: "127.0.0.1:0".parse().expect("local addr"),
            remote_addr: None,
            preferred_codec: Some("PCMU".to_string()),
            parameters: std::collections::HashMap::new(),
        };
        let format = codec_runtime::resolve_codec(&config).expect("PCMU resolves");
        let runtime =
            Arc::new(codec_runtime::DialogCodecRuntime::new(format).expect("codec runtime"));
        let dialog_id = DialogId::new("playout-overflow");
        let mut playout = DecodedPlayoutQueue::new();
        let now = Instant::now();
        let overflow = 16u16;
        let burst: Vec<RtpPlayoutItem> = (0..(RTP_REORDER_MAX_PACKETS as u16 + overflow))
            .map(|sequence| RtpPlayoutItem::Packet(packet(sequence)))
            .collect();

        let terminate = enqueue_rtp_playout_items(
            burst,
            &runtime,
            &dialog_id,
            &mut playout,
            &mut source_timeline::SourceTimeline::default(),
            now,
        )
        .await;

        assert!(
            !terminate,
            "a playout overflow must not terminate the RTP receive loop"
        );
        assert_eq!(playout.pending.len(), RTP_REORDER_MAX_PACKETS);
        assert_eq!(playout.dropped_frames, u64::from(overflow));
        assert_eq!(
            playout
                .pop_due(now)
                .expect("the head frame survives the overflow")
                .timestamp,
            0,
            "overflow drops the newest frames, not the ones already queued"
        );
    }
    #[tokio::test]
    async fn selected_source_keeps_reordering_and_continuous_decoded_timestamps() {
        let config = MediaConfig {
            local_addr: "127.0.0.1:0".parse().unwrap(),
            remote_addr: None,
            preferred_codec: Some("PCMU".into()),
            parameters: Default::default(),
        };
        let runtime = Arc::new(
            codec_runtime::DialogCodecRuntime::new(codec_runtime::resolve_codec(&config).unwrap())
                .unwrap(),
        );
        let mut timeline = source_timeline::SourceTimeline::default();
        let mut reorder = RtpPlayoutBuffer::new();
        let mut decoded = DecodedPlayoutQueue::new();
        let now = Instant::now();
        let mut delivered = Vec::new();
        // Out-of-order packets inside the first source must remain reorderable.
        for (ssrc, seq, timestamp, elapsed) in [
            (1, 10, 14_560, 0),
            (1, 12, 14_880, 20),
            (1, 11, 14_720, 30),
            (2, 80, 456_000, 40),
            (2, 81, 456_160, 50),
            (1, 13, 15_040, 60),
            (2, 82, 456_320, 70),
        ] {
            let at = now + Duration::from_millis(elapsed);
            if !timeline.should_buffer(ssrc, seq, timestamp, at, 8_000) {
                continue;
            }
            let mut packet = packet_for_ssrc(seq, ssrc);
            packet.header.timestamp = timestamp;
            packet.payload = Bytes::from(vec![0x90; 160]);
            let items = reorder.push(packet, at);
            enqueue_rtp_playout_items(
                items,
                &runtime,
                &DialogId::new("source-switch"),
                &mut decoded,
                &mut timeline,
                at,
            )
            .await;
        }
        let mut deadline = now + Duration::from_secs(1);
        while let Some(frame) = decoded.pop_due(deadline) {
            assert!(frame.samples.iter().any(|sample| *sample != 0));
            delivered.push(frame.timestamp);
            deadline += Duration::from_millis(20);
        }
        assert_eq!(&delivered[..3], &[14_560, 14_720, 14_880]);
        assert_eq!(
            delivered.len(),
            5,
            "probation and retired-source audio do not escape"
        );
        assert_eq!(delivered[4].wrapping_sub(delivered[3]), 160);
        assert!(delivered[3].wrapping_sub(delivered[2]) <= 400);
    }
}

/// Which side of the DTLS handshake a dialog plays, per RFC 4145/5763's
/// `a=setup` active/passive mapping (the caller resolves the SDP
/// `a=setup` role to this before calling
/// [`MediaSessionController::run_dtls_handshake_and_install`]).
#[cfg(feature = "dtls-srtp")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtlsRole {
    /// Initiates the handshake (`a=setup:active`).
    Client,
    /// Accepts the handshake (`a=setup:passive`).
    Server,
}

#[cfg(feature = "dtls-srtp")]
fn verify_dtls_fingerprint(actual: [u8; 32], expected: [u8; 32]) -> Result<()> {
    if actual != expected {
        return Err(Error::config(
            "DTLS-SRTP certificate fingerprint does not match the authenticated SDP".to_string(),
        ));
    }
    Ok(())
}

/// Directional SRTP contexts from one completed DTLS-SRTP handshake
/// (RFC 5764 §4.2): the DTLS client protects with the client write key and
/// the server with the server write key; each side opens the other's.
#[cfg(feature = "dtls-srtp")]
fn dtls_srtp_contexts(
    result: &rtp_core::dtls_srtp::DtlsSrtpHandshakeResult,
    role: DtlsRole,
) -> Result<(rtp_core::srtp::SrtpContext, rtp_core::srtp::SrtpContext)> {
    let (send_key, recv_key) = match role {
        DtlsRole::Client => (
            result.client_write_key.clone(),
            result.server_write_key.clone(),
        ),
        DtlsRole::Server => (
            result.server_write_key.clone(),
            result.client_write_key.clone(),
        ),
    };
    let send_ctx = rtp_core::srtp::SrtpContext::new(result.profile.clone(), send_key)
        .map_err(|e| Error::config(format!("building DTLS-SRTP send context: {e}")))?;
    let recv_ctx = rtp_core::srtp::SrtpContext::new(result.profile.clone(), recv_key)
        .map_err(|e| Error::config(format!("building DTLS-SRTP recv context: {e}")))?;
    Ok((send_ctx, recv_ctx))
}

/// Releases a media-controller port reservation if `start_media` is cancelled
/// before ownership is committed to the controller maps.
struct MediaPortReservationGuard {
    allocator: Arc<PortAllocator>,
    session_id: Option<String>,
}

impl MediaPortReservationGuard {
    fn new(allocator: Arc<PortAllocator>, session_id: String) -> Self {
        Self {
            allocator,
            session_id: Some(session_id),
        }
    }

    fn disarm(&mut self) {
        self.session_id = None;
    }
}

impl Drop for MediaPortReservationGuard {
    fn drop(&mut self) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let allocator = self.allocator.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = allocator.release_session(&session_id).await;
            });
        }
    }
}

/// Controller-level capacity and pool tuning.
#[derive(Debug, Clone)]
pub struct MediaSessionControllerConfig {
    /// Initial capacity for hot session indexes.
    pub capacity_hint: usize,
    /// Shared audio frame pool configuration.
    pub audio_frame_pool: PoolConfig,
    /// RTP output buffer size in bytes.
    pub rtp_buffer_size: usize,
    /// Initial RTP output buffer count.
    pub rtp_buffer_initial_count: usize,
    /// Maximum RTP output buffer count.
    pub rtp_buffer_max_count: usize,
    /// RTP session queue sizing.
    pub rtp_session_buffer_config: RtpSessionBufferConfig,
    /// RTP transport event and receive buffer sizing.
    pub rtp_transport_buffer_config: RtpTransportBufferConfig,
}

impl Default for MediaSessionControllerConfig {
    fn default() -> Self {
        Self {
            capacity_hint: 0,
            audio_frame_pool: PoolConfig {
                initial_size: 32,
                max_size: 128,
                sample_rate: 8000,
                channels: 1,
                samples_per_frame: 160,
            },
            rtp_buffer_size: 480,
            rtp_buffer_initial_count: 32,
            rtp_buffer_max_count: 128,
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
        }
    }
}

/// The reservation [`MediaSessionController::start_media`] makes for a
/// session with or without a separate RTCP port.
fn session_port_reservation(separate_rtcp_port: bool) -> SessionPortReservation {
    if separate_rtcp_port {
        SessionPortReservation::RtpRtcpPair
    } else {
        SessionPortReservation::PortPair
    }
}

fn configured_port_range_len(base_port: u16, max_port: u16) -> usize {
    if max_port < base_port {
        0
    } else {
        usize::from(max_port - base_port) + 1
    }
}

fn is_retryable_rtp_bind_error(error: &rtp_core::Error) -> bool {
    let text = error.to_string();
    text.contains("bind") || text.contains("Address already in use")
}

// Sub-modules
pub mod advanced_processing;
pub mod audio_generation;
pub mod bridge;
pub mod cn_gate;
pub mod cn_transmitter;
pub mod codec_detection;
pub mod codec_fallback;
pub mod conference;
pub mod dtmf_transmitter;
pub mod rtp_management;
pub mod statistics;
pub mod types;
pub mod zero_copy;

#[cfg(test)]
mod tests;

// Re-export important types
pub use audio_generation::{AudioSource, AudioTransmitterConfig};
pub use bridge::{BridgeError, BridgeHandle};
pub use inline_rx::{
    inline_rx_counters, InlineAudioConsumer, InlineAudioConsumerGuard, InlineAudioDelivery,
    InlineRxCounters,
};
pub use rtp_management::{EncodedG711Format, G711SendNow, PreparedG711Sender};
pub use types::{
    AdvancedProcessorConfig, AdvancedProcessorSet, MediaConfig, MediaSessionEvent,
    MediaSessionInfo, MediaSessionStatus, AMR_DTX_PARAMETER, NEGOTIATED_FMTP_PARAMETER,
    REMOTE_RTCP_ADDR_PARAMETER, RTCP_MUX_PARAMETER, RTCP_REDUCED_MINIMUM_PARAMETER,
    RTCP_SEPARATE_PORT_PARAMETER, RTCP_XR_PARAMETER,
};

use types::{DialogTxGate, RtpSessionWrapper, SecureKeyingState};

#[cfg(feature = "memory-diagnostics")]
fn spawn_memory_tracked<F>(kind: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let guard = rvoip_infra_common::memory_diagnostics::ObjectGuard::new(kind, 0);
    rvoip_rtp_core::task_runtime::spawn_media_task(async move {
        let _guard = guard;
        future.await
    })
}

#[cfg(not(feature = "memory-diagnostics"))]
fn spawn_memory_tracked<F>(_: &'static str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    rvoip_rtp_core::task_runtime::spawn_media_task(future)
}

#[cfg(feature = "memory-diagnostics")]
fn record_transient_allocation(kind: &'static str, bytes: usize) {
    rvoip_infra_common::memory_diagnostics::record_transient_allocation(kind, bytes as u64);
}

#[cfg(not(feature = "memory-diagnostics"))]
fn record_transient_allocation(_: &'static str, _: usize) {}

fn perf_skip_audio_frame_delivery() -> bool {
    static SKIP: OnceLock<bool> = OnceLock::new();
    *SKIP.get_or_init(|| {
        if !cfg!(feature = "memory-diagnostics") {
            return false;
        }
        matches!(
            std::env::var("RVOIP_PERF_SKIP_AUDIO_FRAME_DELIVERY").as_deref(),
            Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("YES") | Ok("on") | Ok("ON")
        )
    })
}

#[derive(Default)]
struct RtpDeliveryState {
    decoded_audio_frame_count: u64,
    logged_missing_audio_callback: bool,
    last_delivered_at: Option<Instant>,
}

#[allow(clippy::too_many_arguments)]
async fn enqueue_rtp_playout_items(
    items: Vec<RtpPlayoutItem>,
    codec_runtime: &Arc<codec_runtime::DialogCodecRuntime>,
    dialog_id: &DialogId,
    decoded_playout: &mut DecodedPlayoutQueue,
    timeline: &mut source_timeline::SourceTimeline,
    arrived_at: Instant,
) -> bool {
    for item in items {
        let audio_frame = match item {
            RtpPlayoutItem::Packet(packet) => {
                let Some(accepted) = timeline.prepare(
                    packet.header.ssrc,
                    packet.header.sequence_number,
                    packet.header.timestamp,
                    arrived_at,
                    codec_runtime.format.clock_rate,
                ) else {
                    continue;
                };
                match codec_runtime
                    .decode_source(&packet.payload, accepted.timestamp, accepted.source_changed)
                    .await
                {
                    Ok(frame) => {
                        let ticks = (frame.samples_per_channel() as u64
                            * u64::from(codec_runtime.format.clock_rate)
                            / u64::from(frame.sample_rate.max(1)))
                            as u32;
                        timeline.commit(accepted, arrived_at, ticks);
                        frame
                    }
                    Err(error) => {
                        warn!(
                            "Failed to decode {} for dialog {}: {}",
                            codec_runtime.format.name, dialog_id, error
                        );
                        continue;
                    }
                }
            }
            RtpPlayoutItem::Gap {
                sequence_number,
                timestamp,
                samples_per_channel,
            } => {
                let Some(sample_count) = concealment_sample_count(
                    samples_per_channel,
                    codec_runtime.format.clock_rate,
                    codec_runtime.format.channels,
                ) else {
                    warn!(
                        %dialog_id,
                        sequence_number,
                        timestamp,
                        samples_per_channel,
                        clock_rate = codec_runtime.format.clock_rate,
                        channels = codec_runtime.format.channels,
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
                    vec![0; sample_count],
                    codec_runtime.format.clock_rate,
                    codec_runtime.format.channels,
                    timeline.map_gap(timestamp),
                )
            }
        };
        if !decoded_playout.push(audio_frame, arrived_at) {
            // The paced queue is a jitter bound, not a lifecycle boundary. A
            // producer running ahead of real time, such as a bridge releasing
            // media it buffered until commit, overflows it in bursts. Drop the
            // newest frame and keep the receive loop alive, exactly as a full
            // application callback channel is handled: terminating the loop
            // here silenced the dialog for the rest of the call.
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
    false
}

#[allow(clippy::too_many_arguments)]
fn deliver_due_audio_frame(
    now: Instant,
    decoded_playout: &mut DecodedPlayoutQueue,
    dialog_id: &DialogId,
    media_directions: &DashMap<DialogId, MediaDirection>,
    audio_frame_callbacks: &DashMap<DialogId, mpsc::Sender<AudioFrame>>,
    collect_audio_quality: bool,
    skip_audio_frame_delivery: bool,
    state: &mut RtpDeliveryState,
) -> bool {
    let Some(audio_frame) = decoded_playout.pop_due(now) else {
        return false;
    };
    let decoded_len = audio_frame.samples.len();

    let receive_enabled = media_directions
        .get(dialog_id)
        .map(|direction| {
            matches!(
                *direction,
                MediaDirection::SendRecv | MediaDirection::RecvOnly
            )
        })
        .unwrap_or(true);
    if !receive_enabled {
        debug!(
            "Dropping received RTP audio for dialog {} due to media direction",
            dialog_id
        );
        return false;
    }
    if collect_audio_quality {
        diagnostics::record_audio_rx_decoded_frame();
    }

    if skip_audio_frame_delivery {
        state.decoded_audio_frame_count += 1;
        record_transient_allocation(
            "media_core.audio.rx.decoded_without_frame_delivery",
            decoded_len * std::mem::size_of::<i16>(),
        );
        return false;
    }

    let sender = audio_frame_callbacks
        .get(dialog_id)
        .map(|entry| entry.value().clone());
    if let Some(sender) = sender {
        state.decoded_audio_frame_count += 1;
        record_transient_allocation(
            "media_core.audio.rx.audio_frame.samples_vec",
            audio_frame.samples.capacity() * std::mem::size_of::<i16>(),
        );

        match sender.try_send(audio_frame) {
            Ok(()) => {
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
                    trace!(
                        "✅ Sent decoded audio frame #{} to callback for dialog {}",
                        state.decoded_audio_frame_count,
                        dialog_id
                    );
                }
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                warn!(
                    "Audio frame buffer full for dialog {} at frame #{}, dropping frame",
                    dialog_id, state.decoded_audio_frame_count
                );
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                error!(
                        "❌ Audio frame channel closed for dialog {} at frame #{} - THIS IS THE 100-FRAME BUG!",
                        dialog_id, state.decoded_audio_frame_count
                    );
                return true;
            }
        }
    } else if !state.logged_missing_audio_callback {
        info!(
            "⚠️ No audio frame callback registered yet for dialog {}",
            dialog_id
        );
        state.logged_missing_audio_callback = true;
    }
    false
}

/// RFC 4733 DTMF event delivered from media-core to session-core.
#[derive(Debug, Clone, Copy)]
pub struct DtmfNotification {
    /// Decoded digit character ('0'-'9', '*', '#', 'A'-'D'). Unknown
    /// event codes (reserved for future use / fax / modem tones) are
    /// mapped to `'?'` so the callback consumer can choose to ignore.
    pub digit: char,
    /// Duration of the event at the time we observed `E=1`, converted
    /// from RTP timestamp units to milliseconds assuming the standard
    /// 8 kHz telephone-event clock.
    pub duration_ms: u32,
    /// Sending SSRC — for b2bua disambiguation when multiple streams
    /// share a socket.
    pub ssrc: u32,
}

/// Media Session Controller for managing media sessions and conference audio mixing
pub struct MediaSessionController {
    /// Active media sessions indexed by dialog ID. `DashMap` so
    /// per-dialog inserts/lookups don't serialise through one async
    /// RwLock on every bridge forward, RTCP report, stats query, or
    /// audio-frame callback dispatch.
    pub(super) sessions: Arc<DashMap<DialogId, MediaSessionInfo>>,
    /// Active RTP sessions indexed by dialog ID. Same `DashMap`
    /// rationale as [`Self::sessions`].
    pub(super) rtp_sessions: Arc<DashMap<DialogId, RtpSessionWrapper>>,
    /// Event channel for media session events
    pub(super) event_tx: mpsc::UnboundedSender<MediaSessionEvent>,
    /// Event receiver (taken by the user). Held here until
    /// `take_event_receiver` drains it on the consumer side.
    #[allow(dead_code)]
    event_rx: RwLock<Option<mpsc::UnboundedReceiver<MediaSessionEvent>>>,
    /// Event hub for global event coordination
    event_hub: Arc<RwLock<Option<Arc<crate::events::MediaEventHub>>>>,
    /// Session to media mapping
    session_to_media: Arc<DashMap<String, MediaSessionId>>,
    /// Media to session mapping
    media_to_session: Arc<DashMap<MediaSessionId, String>>,
    /// Every forward alias currently bound to each media id, so cleanup
    /// visits only that media's aliases instead of scanning every active
    /// session. Mapping writers and cleanup commit under this lock; lookups
    /// stay on the DashMaps.
    session_mapping_owners: Arc<Mutex<HashMap<MediaSessionId, HashSet<String>>>>,
    /// Audio mixer for conference calls
    pub(super) audio_mixer: Option<Arc<AudioMixer>>,
    /// Conference mixing configuration
    pub(super) conference_config: ConferenceMixingConfig,
    /// Conference event sender
    pub(super) conference_event_tx: mpsc::UnboundedSender<ConferenceMixingEvent>,
    /// Conference event receiver
    conference_event_rx: RwLock<Option<mpsc::UnboundedReceiver<ConferenceMixingEvent>>>,
    /// Quality monitor for conference sessions. Wired in but the
    /// per-controller reads land via the conference task; retained
    /// here so the controller can install/replace it at runtime.
    #[allow(dead_code)]
    pub(super) quality_monitor: Option<Arc<QualityMonitor>>,
    /// Port allocator for RTP ports (if custom range specified)
    port_allocator: Option<Arc<PortAllocator>>,

    // Performance library integration fields
    /// Global performance metrics for all sessions. Lock-free
    /// padded atomics — every per-frame `add_timing` /
    /// `add_allocation` is a handful of relaxed `fetch_add` /
    /// `fetch_min` / `fetch_max` calls on independent cache lines.
    /// Snapshot via `.snapshot()` materialises a `PerformanceMetrics`
    /// for external consumers.
    pub(super) performance_metrics: Arc<ConcurrentPerformanceMetrics>,
    /// Global frame pool for efficient allocation (shared across sessions)
    pub(super) frame_pool: Arc<AudioFramePool>,
    /// RTP output buffer pool for zero-copy encoding
    pub(super) rtp_buffer_pool: Arc<RtpBufferPool>,
    /// Advanced processors per dialog. `DashMap` so the per-frame
    /// AEC/AGC/VAD lookup doesn't take an outer async RwLock. The
    /// value is `Arc<AdvancedProcessorSet>` so callers can clone the
    /// Arc out of the shard guard and drop the guard before awaiting
    /// — the shard guard is `!Send` and would otherwise infect every
    /// task that holds one across `.await`.
    pub(super) advanced_processors: Arc<DashMap<DialogId, Arc<AdvancedProcessorSet>>>,
    /// Default configuration for advanced processors
    pub(super) default_processor_config: AdvancedProcessorConfig,
    /// SIMD processor for audio operations
    pub(super) simd_processor: SimdProcessor,

    /// Audio frame callbacks for sending decoded frames to session-core.
    /// `DashMap` so per-frame dispatch (called from the recv hot
    /// path) doesn't acquire an outer async RwLock just to find the
    /// per-dialog sender.
    pub(super) audio_frame_callbacks: Arc<DashMap<DialogId, mpsc::Sender<AudioFrame>>>,

    /// RFC 4733 DTMF callbacks for sending decoded digits to
    /// session-core. Fired once per digit (on the first observed
    /// end-of-event frame), deduping the two extra RFC 4733 §2.5.1.3
    /// retransmissions on the sender side.
    pub(super) dtmf_callbacks: Arc<DashMap<DialogId, mpsc::Sender<DtmfNotification>>>,

    /// Codec mapper for payload type resolution
    pub(super) codec_mapper: Arc<CodecMapper>,

    /// RTP bridge for processing incoming packets
    pub(super) rtp_bridge: Arc<RtpBridge>,

    /// Bidirectional partner map for sessions bridged at the RTP layer.
    /// Both directions are stored so lookup is O(1) from either end. Cleared
    /// on `BridgeHandle` drop or `stop_media` of a bridged session.
    pub(super) bridge_partners: Arc<DashMap<DialogId, DialogId>>,

    /// Sprint 3.6 C1 follow-up — RFC 3389 comfort-noise gate state per
    /// dialog. Lazily populated on first audio frame for sessions
    /// whose controller has [`comfort_noise_enabled`] set; the gate
    /// runs the simple energy/ZCR VAD over each outbound frame and
    /// returns `Send`/`Suppress`/`EmitCnThenSuppress`. Cleared on
    /// `stop_media` alongside the rest of the per-dialog state.
    ///
    /// [`comfort_noise_enabled`]: Self::comfort_noise_enabled
    pub(super) cn_gate_state:
        Arc<DashMap<DialogId, Arc<tokio::sync::Mutex<crate::relay::controller::cn_gate::CnGate>>>>,

    /// Sprint 3.6 C1 follow-up — global toggle. When `true`,
    /// `encode_and_send_audio_frame` consults the per-dialog
    /// [`cn_gate_state`](Self::cn_gate_state) to decide whether to
    /// send the audio packet, suppress it, or emit one PT 13 CN
    /// packet first. Wired from
    /// `session-core::Config::comfort_noise_enabled` via
    /// [`Self::set_comfort_noise_enabled`].
    pub(super) comfort_noise_enabled: Arc<std::sync::atomic::AtomicBool>,

    /// Per-dialog RTP/audio direction. This is the media-core enforcement
    /// point for SIP hold/resume and remote direction changes.
    pub(super) media_directions: Arc<DashMap<DialogId, MediaDirection>>,

    /// Exact negotiated codec identity and state for each dialog. Replacing
    /// the `Arc` commits a codec update atomically for both transmit and
    /// receive paths.
    codec_runtimes: Arc<DashMap<DialogId, Arc<codec_runtime::DialogCodecRuntime>>>,

    /// Inline receive leases (media executor v2): per-dialog control state
    /// mirrored for the inline sink on the socket owner. Empty unless
    /// `MediaExecutorConfig::inline_decode` is active; touched only at
    /// control-plane events, never per packet.
    inline_leases: Arc<DashMap<DialogId, Arc<inline_rx::DialogInlineLease>>>,

    /// RTP session queue sizing for new sessions.
    rtp_session_buffer_config: RtpSessionBufferConfig,

    /// RTP transport event and receive buffer sizing for new sessions.
    rtp_transport_buffer_config: RtpTransportBufferConfig,

    /// Source-compatible policy sidecar for symmetric-RTP learning. This is
    /// configured through a controller builder instead of adding a field to
    /// the public `MediaSessionControllerConfig` struct.
    symmetric_rtp_policy: SymmetricRtpPolicy,
}

impl MediaSessionController {
    /// Exact number of RTP/RTCP UDP ports currently leased by this controller.
    /// Controllers without a configured allocator own no range-backed ports.
    pub async fn allocated_port_count(&self) -> usize {
        match &self.port_allocator {
            Some(allocator) => allocator.allocated_count().await,
            None => 0,
        }
    }

    /// Media sessions this controller's range can still start on `ip`.
    ///
    /// `separate_rtcp_port` selects the reservation [`Self::start_media`]
    /// makes: a session started with
    /// [`MediaConfig::with_rtcp_separate_port`] needs an even RTP port and
    /// the port above it, every other session one port. Point-in-time and
    /// advisory; `start_media` stays the authoritative check. `None` when
    /// the controller has no range-backed allocator to count.
    pub fn available_session_port_slots(
        &self,
        ip: std::net::IpAddr,
        separate_rtcp_port: bool,
    ) -> Option<usize> {
        self.port_allocator
            .as_ref()?
            .available_session_slots(ip, session_port_reservation(separate_rtcp_port))
    }

    /// Media sessions the configured range holds when every port is free,
    /// for the reservation `separate_rtcp_port` selects (see
    /// [`Self::available_session_port_slots`]).
    pub fn configured_session_port_slots(&self, separate_rtcp_port: bool) -> Option<usize> {
        Some(
            self.port_allocator
                .as_ref()?
                .configured_session_slots(session_port_reservation(separate_rtcp_port)),
        )
    }

    /// Create a new media session controller
    pub fn new() -> Self {
        Self::new_with_capacity_hint(0)
    }

    /// Create a media session controller with preallocated hot indexes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::new_with_capacity_hint(capacity)
    }

    /// Create a media session controller with explicit capacity and pool tuning.
    pub fn with_config(config: MediaSessionControllerConfig) -> Self {
        Self::new_with_config(config)
    }

    fn new_with_capacity_hint(capacity_hint: usize) -> Self {
        Self::new_with_config(MediaSessionControllerConfig {
            capacity_hint,
            ..Default::default()
        })
    }

    fn new_with_config(config: MediaSessionControllerConfig) -> Self {
        let capacity_hint = config.capacity_hint;
        let rtp_session_buffer_config = config.rtp_session_buffer_config;
        let rtp_transport_buffer_config = config.rtp_transport_buffer_config;
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();

        // Initialize performance components
        let performance_metrics = Arc::new(ConcurrentPerformanceMetrics::new());

        // Create global frame pool (shared across sessions)
        let frame_pool: Arc<AudioFramePool> = AudioFramePool::new(config.audio_frame_pool.clone());

        // Create RTP buffer pool
        let rtp_buffer_pool = RtpBufferPool::new(
            config.rtp_buffer_size,
            config.rtp_buffer_initial_count,
            config.rtp_buffer_max_count,
        );

        // Default advanced processor configuration
        let default_processor_config = AdvancedProcessorConfig::default();

        // G.711 µ-law / A-law codecs are stateless (just `variant` +
        // `frame_size`), so we instantiate fresh per call instead of
        // sharing one tokio::Mutex<G711Codec> across every session.
        // That mutex was the single biggest serialisation point on the
        // per-frame encode/decode hot path under load — see Phase C5.

        // Create SIMD processor
        let simd_processor = SimdProcessor::new();

        // Create codec mapper
        let codec_mapper = Arc::new(CodecMapper::new());

        // Create RTP bridge with its dependencies
        let (integration_event_tx, _integration_event_rx) = mpsc::unbounded_channel();
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        let rtp_bridge = Arc::new(RtpBridge::new(
            RtpBridgeConfig::default(),
            integration_event_tx,
            codec_mapper.clone(),
            codec_detector,
            fallback_manager,
        ));

        Self {
            sessions: Arc::new(DashMap::with_capacity(capacity_hint)),
            rtp_sessions: Arc::new(DashMap::with_capacity(capacity_hint)),
            event_tx,
            event_rx: RwLock::new(None),
            event_hub: Arc::new(RwLock::new(None)),
            session_to_media: Arc::new(DashMap::with_capacity(capacity_hint)),
            media_to_session: Arc::new(DashMap::with_capacity(capacity_hint)),
            session_mapping_owners: Arc::new(Mutex::new(HashMap::with_capacity(capacity_hint))),
            audio_mixer: None,
            conference_config: ConferenceMixingConfig::default(),
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            port_allocator: None, // Use GlobalPortAllocator by default
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: Arc::new(DashMap::with_capacity(capacity_hint)),
            default_processor_config,
            simd_processor,
            audio_frame_callbacks: Arc::new(DashMap::with_capacity(capacity_hint)),
            dtmf_callbacks: Arc::new(DashMap::with_capacity(capacity_hint)),
            codec_mapper,
            rtp_bridge,
            bridge_partners: Arc::new(DashMap::with_capacity(capacity_hint)),
            cn_gate_state: Arc::new(DashMap::with_capacity(capacity_hint)),
            comfort_noise_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            media_directions: Arc::new(DashMap::with_capacity(capacity_hint)),
            codec_runtimes: Arc::new(DashMap::with_capacity(capacity_hint)),
            inline_leases: Arc::new(DashMap::new()),
            rtp_session_buffer_config,
            rtp_transport_buffer_config,
            symmetric_rtp_policy: SymmetricRtpPolicy::default(),
        }
    }

    /// Configure bounded symmetric-RTP learning for sessions created by this
    /// controller.
    pub fn with_symmetric_rtp_policy(mut self, policy: SymmetricRtpPolicy) -> Self {
        self.symmetric_rtp_policy = policy;
        self
    }

    /// Toggle RFC 3389 Comfort Noise gating on the audio TX path.
    /// When enabled, [`encode_and_send_audio_frame`](Self::encode_and_send_audio_frame)
    /// runs a per-dialog VAD over each outbound PCM frame and either
    /// sends the audio normally, suppresses it (silence already
    /// covered), or emits one PT 13 CN packet then suppresses
    /// (speech→silence transition / §4.1 refresh). Wired from
    /// `session-core::Config::comfort_noise_enabled` at coordinator
    /// boot via the media adapter.
    pub fn set_comfort_noise_enabled(&self, enabled: bool) {
        self.comfort_noise_enabled
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        if !enabled {
            // Drop any per-dialog gate state — next time CN is
            // re-enabled the gate will rebuild fresh.
            self.cn_gate_state.clear();
        }
    }

    /// Register an RTP event callback with the RTP bridge
    /// This allows external subscribers (like session-core) to receive RTP events
    pub async fn add_rtp_event_callback(&self, callback: RtpEventCallback) {
        self.rtp_bridge.add_rtp_event_callback(callback).await;
    }

    // ===== Event Hub Helper Methods =====

    /// Set the event hub for global event coordination
    pub async fn set_event_hub(&self, event_hub: Arc<crate::events::MediaEventHub>) {
        *self.event_hub.write().await = Some(event_hub);
    }

    /// Store session to media mapping
    pub fn store_session_mapping(&self, session_id: String, media_id: MediaSessionId) {
        let mut owners = self
            .session_mapping_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(previous_media) = self
            .session_to_media
            .insert(session_id.clone(), media_id.clone())
        {
            if previous_media != media_id {
                if let Some(previous_owners) = owners.get_mut(&previous_media) {
                    previous_owners.remove(&session_id);
                    if previous_owners.is_empty() {
                        owners.remove(&previous_media);
                    }
                }
            }
        }
        owners
            .entry(media_id.clone())
            .or_default()
            .insert(session_id.clone());
        // The previous media keeps its reverse entry until its own cleanup,
        // so callers can still resolve the retiring media's session.
        self.media_to_session.insert(media_id, session_id);
    }

    /// Get media session ID from session ID
    pub fn get_media_id(&self, session_id: &str) -> Option<MediaSessionId> {
        self.session_to_media
            .get(session_id)
            .map(|e| e.value().clone())
    }

    /// Get session ID from media session ID
    pub fn get_session_id(&self, media_id: &MediaSessionId) -> Option<String> {
        self.media_to_session
            .get(media_id)
            .map(|e| e.value().clone())
    }

    /// Feature-gated retained-object counts for perf leak investigations.
    #[cfg(feature = "perf-diagnostics")]
    #[doc(hidden)]
    pub fn diagnostic_counts(&self) -> serde_json::Value {
        let mut rtp_sender_queue_packets = 0usize;
        let mut rtp_sender_capacity_packets = 0usize;
        let mut rtp_receiver_queue_packets = 0usize;
        let mut rtp_receiver_capacity_packets = 0usize;
        let mut rtp_event_queue_events = 0usize;
        let mut rtp_event_receiver_count = 0usize;
        let mut rtp_sessions_locked_for_diag = 0usize;
        #[cfg(feature = "memory-diagnostics")]
        let mut rtp_streams = 0usize;

        for entry in self.rtp_sessions.iter() {
            match entry.value().session.try_lock() {
                Ok(session) => {
                    let counts = session.queue_diagnostics();
                    rtp_sender_queue_packets += counts.sender_queue_packets;
                    rtp_sender_capacity_packets += counts.sender_capacity_packets;
                    rtp_receiver_queue_packets += counts.receiver_queue_packets;
                    rtp_receiver_capacity_packets += counts.receiver_capacity_packets;
                    rtp_event_queue_events += counts.event_queue_events;
                    rtp_event_receiver_count += counts.event_receiver_count;
                    #[cfg(feature = "memory-diagnostics")]
                    {
                        rtp_streams += counts.stream_count;
                    }
                }
                Err(_) => {
                    rtp_sessions_locked_for_diag += 1;
                }
            }
        }

        let audio_callback_queue_frames: usize = self
            .audio_frame_callbacks
            .iter()
            .map(|entry| {
                let sender = entry.value();
                sender.max_capacity().saturating_sub(sender.capacity())
            })
            .sum();
        let audio_callback_capacity_frames: usize = self
            .audio_frame_callbacks
            .iter()
            .map(|entry| entry.value().max_capacity())
            .sum();

        #[cfg_attr(not(feature = "memory-diagnostics"), allow(unused_mut))]
        let mut value = serde_json::json!({
            "sessions": self.sessions.len(),
            "rtp_sessions": self.rtp_sessions.len(),
            "rtp_sender_queue_packets": rtp_sender_queue_packets,
            "rtp_sender_capacity_packets": rtp_sender_capacity_packets,
            "rtp_receiver_queue_packets": rtp_receiver_queue_packets,
            "rtp_receiver_capacity_packets": rtp_receiver_capacity_packets,
            "rtp_event_queue_events": rtp_event_queue_events,
            "rtp_event_receiver_count": rtp_event_receiver_count,
            "rtp_sessions_locked_for_diag": rtp_sessions_locked_for_diag,
            "session_to_media": self.session_to_media.len(),
            "media_to_session": self.media_to_session.len(),
            "audio_frame_callbacks": self.audio_frame_callbacks.len(),
            "audio_callback_queue_frames": audio_callback_queue_frames,
            "audio_callback_capacity_frames": audio_callback_capacity_frames,
            "dtmf_callbacks": self.dtmf_callbacks.len(),
            "bridge_partners": self.bridge_partners.len(),
            "cn_gate_state": self.cn_gate_state.len(),
            "advanced_processors": self.advanced_processors.len(),
            "media_directions": self.media_directions.len(),
            "inline_rx_leases": self.inline_leases.len(),
        });
        #[cfg(feature = "memory-diagnostics")]
        if let Some(obj) = value.as_object_mut() {
            obj.insert("rtp_streams".into(), serde_json::json!(rtp_streams));
            obj.insert("frame_pool".into(), self.frame_pool.diagnostic_counts());
            obj.insert(
                "rtp_buffer_pool".into(),
                self.rtp_buffer_pool.diagnostic_counts(),
            );
        }
        value
    }

    /// Emit a media event through both channel and event hub. Held
    /// alongside the public `take_event_receiver` path; new emission
    /// sites will use this helper once dialog events get wired in.
    #[allow(dead_code)]
    async fn emit_event(&self, event: MediaSessionEvent) {
        // Send to channel (legacy)
        let _ = self.event_tx.send(event.clone());

        // Send to event hub (new global bus)
        if let Some(hub) = self.event_hub.read().await.as_ref() {
            if let Err(e) = hub.publish_media_event(event).await {
                warn!("Failed to publish media event to global bus: {}", e);
            }
        }
    }

    /// Create a new media session controller with custom port range
    pub fn with_port_range(base_port: u16, max_port: u16) -> Self {
        Self::with_port_range_and_capacity(base_port, max_port, 0)
    }

    /// Create a media session controller with custom port range and capacity.
    pub fn with_port_range_and_capacity(
        base_port: u16,
        max_port: u16,
        capacity_hint: usize,
    ) -> Self {
        Self::with_port_range_and_config(
            base_port,
            max_port,
            MediaSessionControllerConfig {
                capacity_hint,
                ..Default::default()
            },
        )
    }

    /// Create a media session controller with custom port range and explicit tuning.
    pub fn with_port_range_and_config(
        base_port: u16,
        max_port: u16,
        config: MediaSessionControllerConfig,
    ) -> Self {
        let capacity_hint = config.capacity_hint;
        let mut controller = Self::new_with_config(config);

        // Create a custom port allocator with the specified range
        let mut config = PortAllocatorConfig::default();
        config.port_range_start = base_port;
        config.port_range_end = max_port;
        config.validate_ports = false;
        config.capacity_hint = capacity_hint;
        // Media controllers perform the authoritative bind in RtpSession. Use
        // the indexed unvalidated pool for every custom range so controllers
        // with the same bind domain share reservations before that bind.
        config.allocation_strategy = AllocationStrategy::Incremental;
        config.prefer_port_reuse = false;
        config.allocation_retries = configured_port_range_len(base_port, max_port) as u32;

        controller.port_allocator = Some(Arc::new(PortAllocator::with_config(config)));
        info!(
            "Created MediaSessionController with custom port range {}-{}",
            base_port, max_port
        );

        controller
    }

    /// Create a new media session controller with conference audio mixing enabled
    pub async fn with_conference_mixing(
        base_port: u16,
        max_port: u16,
        conference_config: ConferenceMixingConfig,
    ) -> Result<Self> {
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let (conference_event_tx, conference_event_rx) = mpsc::unbounded_channel();

        // Create audio mixer with the provided configuration
        let audio_mixer: Arc<AudioMixer> =
            Arc::new(AudioMixer::new(conference_config.clone()).await?);

        // Set up conference event forwarding
        audio_mixer
            .set_event_sender(conference_event_tx.clone())
            .await;

        // Initialize performance components
        let performance_metrics = Arc::new(ConcurrentPerformanceMetrics::new());

        // Create global frame pool with larger capacity for conference mixing
        let pool_config = PoolConfig {
            initial_size: 64, // Larger pool for conference mixing
            max_size: 256,
            sample_rate: conference_config.output_sample_rate,
            channels: conference_config.output_channels as u8,
            samples_per_frame: conference_config.output_samples_per_frame as usize,
        };
        let frame_pool: Arc<AudioFramePool> = AudioFramePool::new(pool_config);

        // Create RTP buffer pool
        let rtp_buffer_pool = RtpBufferPool::new(
            480, // Buffer size: max G.711 frame size (60ms at 8kHz)
            32,  // Initial buffer count (more for conference)
            128, // Max buffer count (more for conference)
        );

        // Default advanced processor configuration for conference
        let mut default_processor_config = AdvancedProcessorConfig::default();
        default_processor_config.frame_pool_size = 32; // Per-session pool size
        default_processor_config.enable_simd = conference_config.enable_simd_optimization;

        // G.711 µ-law / A-law codecs are stateless (just `variant` +
        // `frame_size`); see Phase C5 comment on the matching block in
        // `Self::new()`. Per-call instantiation avoids the shared
        // tokio::Mutex<G711Codec> hot-path lock that previously
        // serialised every encode/decode in every concurrent session.

        // Create SIMD processor
        let simd_processor = SimdProcessor::new();

        // Create codec mapper
        let codec_mapper = Arc::new(CodecMapper::new());

        // Create RTP bridge with its dependencies
        let (integration_event_tx, _integration_event_rx) = mpsc::unbounded_channel();
        let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
        let fallback_manager = Arc::new(CodecFallbackManager::new(
            codec_detector.clone(),
            codec_mapper.clone(),
        ));
        let rtp_bridge = Arc::new(RtpBridge::new(
            RtpBridgeConfig::default(),
            integration_event_tx,
            codec_mapper.clone(),
            codec_detector,
            fallback_manager,
        ));

        // Create a custom port allocator with the specified range
        let mut port_config = PortAllocatorConfig::default();
        port_config.port_range_start = base_port;
        port_config.port_range_end = max_port;
        port_config.validate_ports = false;
        port_config.allocation_strategy = AllocationStrategy::Incremental;
        port_config.prefer_port_reuse = false;
        port_config.allocation_retries = configured_port_range_len(base_port, max_port) as u32;
        let port_allocator = Some(Arc::new(PortAllocator::with_config(port_config)));

        Ok(Self {
            sessions: Arc::new(DashMap::new()),
            rtp_sessions: Arc::new(DashMap::new()),
            event_tx,
            event_rx: RwLock::new(None),
            event_hub: Arc::new(RwLock::new(None)),
            session_to_media: Arc::new(DashMap::new()),
            media_to_session: Arc::new(DashMap::new()),
            session_mapping_owners: Arc::new(Mutex::new(HashMap::new())),
            audio_mixer: Some(audio_mixer),
            conference_config,
            conference_event_tx,
            conference_event_rx: RwLock::new(Some(conference_event_rx)),
            quality_monitor: None,
            port_allocator,
            // Performance fields
            performance_metrics,
            frame_pool,
            rtp_buffer_pool,
            advanced_processors: Arc::new(DashMap::new()),
            default_processor_config,
            simd_processor,
            audio_frame_callbacks: Arc::new(DashMap::new()),
            dtmf_callbacks: Arc::new(DashMap::new()),
            codec_mapper,
            rtp_bridge,
            bridge_partners: Arc::new(DashMap::new()),
            cn_gate_state: Arc::new(DashMap::new()),
            comfort_noise_enabled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            media_directions: Arc::new(DashMap::new()),
            codec_runtimes: Arc::new(DashMap::new()),
            inline_leases: Arc::new(DashMap::new()),
            rtp_session_buffer_config: RtpSessionBufferConfig::default(),
            rtp_transport_buffer_config: RtpTransportBufferConfig::default(),
            symmetric_rtp_policy: SymmetricRtpPolicy::default(),
        })
    }

    /// Start a media session for a dialog
    pub async fn start_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        let media_start_guard = diagnostics::MediaStartGuard::new();
        info!("Starting media session for dialog: {}", dialog_id);

        // Check if media session already exists for this dialog
        if self.sessions.contains_key(&dialog_id) {
            return Err(Error::config(format!(
                "Media session already exists for dialog: {}",
                dialog_id
            )));
        }

        // Resolve and construct the complete codec generation before
        // allocating a port or mutating any controller state. Unsupported,
        // disabled, or malformed codec requests therefore fail atomically.
        let codec_format = codec_runtime::resolve_codec(&config)?;
        let codec_runtime = Arc::new(codec_runtime::DialogCodecRuntime::new(
            codec_format.clone(),
        )?);

        // Allocate RTP port using either our local allocator or the global one
        let allocator = if let Some(ref port_alloc) = self.port_allocator {
            // Use our custom port allocator with configured range
            port_alloc.clone()
        } else {
            // Fall back to global allocator
            GlobalPortAllocator::instance().await
        };

        let payload_type = codec_format.payload_type;
        let clock_rate = codec_format.clock_rate;

        let dialog_session_id = format!("dialog_{}", dialog_id);
        let mut last_bind_error: Option<rtp_core::Error> = None;
        let mut created_session = None;
        // The real RTP socket bind is authoritative. Scan at most one complete
        // configured range so occupied candidates are quarantined and skipped
        // without either failing after an arbitrary eight ports or looping
        // forever when the host has no usable port left.
        let bind_attempt_limit = allocator.configured_port_capacity().max(1);
        // RFC 3550 §11 pair for a peer that may decline rtcp-mux: even RTP
        // port, RTCP on the port above, reserved together.
        let separate_rtcp = config.rtcp_separate_port();
        for attempt in 1..=bind_attempt_limit {
            let allocate_started = Instant::now();
            let allocation = if separate_rtcp {
                allocator
                    .allocate_rtp_rtcp_pair(&dialog_session_id, Some(config.local_addr.ip()))
                    .await
                    .map(|(rtp, rtcp)| (rtp, Some(rtcp)))
            } else {
                allocator
                    .allocate_port_pair(&dialog_session_id, Some(config.local_addr.ip()))
                    .await
                    .map(|(rtp, _)| (rtp, None))
            };
            diagnostics::record_rtp_port_allocate(allocate_started.elapsed());
            let (local_rtp_addr, local_rtcp_addr) = match allocation {
                Ok(allocation) => allocation,
                Err(e) => {
                    let failure_kind = if last_bind_error.is_some() {
                        "rtp_bind_collision"
                    } else {
                        "port_pool_exhausted"
                    };
                    return Err(Error::config(format!(
                        "Media allocation failed [kind={}]: {}",
                        failure_kind, e
                    )));
                }
            };
            let mut reservation_guard =
                MediaPortReservationGuard::new(allocator.clone(), dialog_session_id.clone());

            // Create RTP session configuration
            let rtp_config = RtpSessionConfig {
                local_addr: local_rtp_addr,
                remote_addr: config.remote_addr,
                ssrc: Some(rand::random()),    // Generate random SSRC
                payload_type,                  // Use negotiated payload type
                clock_rate,                    // Use codec-appropriate clock rate
                jitter_buffer_size: Some(500), // Increased from 50 to handle burst traffic
                max_packet_age_ms: Some(1000), // Increased from 200ms to 1s for localhost testing
                enable_jitter_buffer: false,   // Disabled to reduce processing overhead
                session_buffer_config: self.rtp_session_buffer_config,
                transport_buffer_config: self.rtp_transport_buffer_config,
            };

            let session_started = Instant::now();
            let created = match local_rtcp_addr {
                Some(local_rtcp_addr) => {
                    RtpSession::new_event_driven_with_rtcp_socket(
                        rtp_config,
                        self.symmetric_rtp_policy,
                        local_rtcp_addr,
                    )
                    .await
                }
                None => {
                    RtpSession::new_event_driven_with_symmetric_rtp_policy(
                        rtp_config,
                        self.symmetric_rtp_policy,
                    )
                    .await
                }
            };
            match created {
                Ok(rtp_session) => {
                    diagnostics::record_rtp_session_new(session_started.elapsed());
                    created_session = Some((local_rtp_addr, rtp_session, reservation_guard));
                    break;
                }
                Err(e) => {
                    diagnostics::record_rtp_session_new(session_started.elapsed());
                    reservation_guard.disarm();
                    if is_retryable_rtp_bind_error(&e) {
                        if let Err(quarantine_error) =
                            allocator.quarantine_session(&dialog_session_id).await
                        {
                            warn!(
                                "Failed to quarantine RTP bind candidate for {}: {}",
                                dialog_id, quarantine_error
                            );
                            let _ = allocator.release_session(&dialog_session_id).await;
                        }

                        // Try the next reserved port. The allocator no longer probe-binds
                        // for media-controller sessions; the real RTP socket bind is the
                        // authoritative availability check.
                        debug!(
                            "RTP bind failed for {} on {} (attempt {}/{}); retrying with another port: {}",
                            dialog_id, local_rtp_addr, attempt, bind_attempt_limit, e
                        );
                        last_bind_error = Some(e);
                        if attempt < bind_attempt_limit {
                            continue;
                        }
                        break;
                    }

                    let _ = allocator.release_session(&dialog_session_id).await;
                    return Err(Error::config(format!(
                        "Media allocation failed [kind=rtp_session_creation] on {}: {}",
                        local_rtp_addr, e
                    )));
                }
            }
        }

        let (local_rtp_addr, rtp_session, mut reservation_guard) =
            created_session.ok_or_else(|| {
                Error::config(format!(
                    "Media allocation failed [kind=rtp_bind_collision] after {} bind attempts: {}",
                    bind_attempt_limit,
                    last_bind_error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "no bind attempt completed".to_string())
                ))
            })?;

        let rtp_port = local_rtp_addr.port();
        let rtcp_port = rtp_session.local_rtcp_addr().map(|addr| addr.port());
        if rtcp_port.is_some() {
            if config.remote_rtcp_addr().is_some() {
                rtp_session.set_remote_rtcp_addr(config.remote_rtcp_addr());
            }
            if config.rtcp_mux() {
                rtp_session.set_rtcp_mux(true);
            }
        }

        // RTCP policy carried in the configuration. Both default off in the
        // RTP session, so only an explicit opt-in needs applying.
        if config.rtcp_xr() {
            rtp_session.set_rtcp_xr_enabled(true);
        }
        if config.rtcp_reduced_minimum() {
            rtp_session.set_rtcp_reduced_minimum(true);
        }

        // Subscribe to RTP session events before wrapping
        let subscribe_started = Instant::now();
        let rtp_events = rtp_session.subscribe();
        diagnostics::record_rtp_event_subscription(subscribe_started.elapsed());

        // Media executor v2: offer the inline receive sink before any media
        // can flow. The v1 handler below is still spawned: it serves DTMF and
        // stream events, and takes over if the dialog is ever demoted.
        let inline_lease = self.offer_inline_rx(&dialog_id, &rtp_session, &codec_runtime);

        // Wrap RTP session
        let typed_send_handle = rtp_session.send_handle();
        let rtp_wrapper = RtpSessionWrapper {
            session: Arc::new(tokio::sync::Mutex::new(rtp_session)),
            update_lock: Arc::new(tokio::sync::Mutex::new(())),
            local_addr: local_rtp_addr,
            remote_addr: config.remote_addr,
            created_at: std::time::Instant::now(),
            audio_transmitter: None,
            transmission_enabled: true, // Enable transmission by default
            is_muted: false,
            secure_keying: SecureKeyingState::Ready,
            typed_send_handle,
            tx_gate: Arc::new(DialogTxGate::default()),
            #[cfg(feature = "memory-diagnostics")]
            memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard::new(
                "media_core.rtp_session_wrapper",
                std::mem::size_of::<RtpSessionWrapper>(),
            ),
        };
        self.media_directions
            .insert(dialog_id.clone(), MediaDirection::SendRecv);

        // Create media session info
        let session_info = MediaSessionInfo {
            dialog_id: dialog_id.clone(),
            status: MediaSessionStatus::Active,
            config: config.clone(),
            rtp_port: Some(rtp_port),
            rtcp_port,
            rtp_stats: None,
            stats_updated_at: None,
            created_at: std::time::Instant::now(),
        };

        // Store session and RTP session (DashMap inserts are sharded
        // and don't take any outer guard).
        #[cfg(feature = "memory-diagnostics")]
        rvoip_infra_common::memory_diagnostics::record_created(
            "media_core.media_session_info",
            std::mem::size_of::<MediaSessionInfo>(),
        );
        self.sessions.insert(dialog_id.clone(), session_info);
        self.rtp_sessions.insert(dialog_id.clone(), rtp_wrapper);
        self.codec_runtimes.insert(dialog_id.clone(), codec_runtime);
        if let Some(lease) = inline_lease.as_ref() {
            self.publish_inline_lease(&dialog_id, lease);
        }
        // The controller maps now own the RTP session and stop_media owns the
        // matching allocator release.
        reservation_guard.disarm();

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionCreated {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        // Spawn task to handle RTP events for this session
        let handler_started = Instant::now();
        self.spawn_rtp_event_handler_with_inline(
            dialog_id.clone(),
            rtp_events,
            payload_type,
            inline_lease,
        );
        diagnostics::record_rtp_event_handler_spawn(handler_started.elapsed());

        info!(
            "✅ Created media session with REAL RTP session: {} (port: {}, codec: {}, PT: {}, clock: {}Hz)",
            dialog_id,
            rtp_port,
            config.preferred_codec.as_deref().unwrap_or("PCMU"),
            payload_type,
            clock_rate
        );
        media_start_guard.finish_success();
        Ok(())
    }

    /// Install RFC 4568 SDES-SRTP contexts on the dialog's RTP
    /// session, switching its transport from plain RTP to encrypted
    /// SRTP for both directions.
    ///
    /// Must be called after [`Self::start_media`] (the RTP session
    /// must already exist) and before audio transmission begins so
    /// that no plaintext packets leak. The contexts are consumed —
    /// each call replaces any previously-installed pair (allowing
    /// re-keying after a session refresh, though that's not yet
    /// driven from session-core).
    pub async fn install_srtp_contexts(
        &self,
        dialog_id: &DialogId,
        send_ctx: rvoip_rtp_core::srtp::SrtpContext,
        recv_ctx: rvoip_rtp_core::srtp::SrtpContext,
    ) -> Result<()> {
        // Extract the session Arc from the DashMap shard, drop the
        // shard guard before any `.await`, then lock the per-session
        // mutex against just our caller — no cross-dialog
        // serialisation.
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|r| r.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        // RtpSession exposes its transport via a typed accessor; we
        // need the UdpRtpTransport concrete type to reach
        // the reversible SRTP context API. Downcast once — the only transport
        // type media-core constructs is UDP, so this is the
        // architecturally-correct narrowing.
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rvoip_rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "install_srtp_contexts: RTP session is not a UdpRtpTransport".to_string(),
                )
            })?;
        let _rollback = udp_transport
            .replace_srtp_contexts(send_ctx, recv_ctx)
            .await
            .map_err(|error| Error::config(format!("failed to install SRTP contexts: {error}")))?;
        info!("Installed SDES-SRTP contexts on dialog {}", dialog_id);
        Ok(())
    }

    /// Prepare a reversible directional SRTP replacement for one exact RTP
    /// session. Dropping the returned token commits the new contexts.
    pub async fn prepare_srtp_context_swap(
        &self,
        dialog_id: &DialogId,
        send_ctx: rvoip_rtp_core::srtp::SrtpContext,
        recv_ctx: rvoip_rtp_core::srtp::SrtpContext,
    ) -> Result<rvoip_rtp_core::transport::SrtpContextRollback> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rvoip_rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "prepare_srtp_context_swap: RTP session is not a UdpRtpTransport".to_string(),
                )
            })?;
        udp_transport
            .replace_srtp_contexts(send_ctx, recv_ctx)
            .await
            .map_err(|error| Error::config(format!("failed to replace SRTP contexts: {error}")))
    }

    /// Restore a prepared SRTP replacement if its SIP commit reaches a
    /// definite zero-wire failure.
    pub async fn rollback_srtp_context_swap(
        &self,
        dialog_id: &DialogId,
        rollback: rvoip_rtp_core::transport::SrtpContextRollback,
    ) -> Result<()> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rvoip_rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "rollback_srtp_context_swap: RTP session is not a UdpRtpTransport".to_string(),
                )
            })?;
        udp_transport
            .rollback_srtp_contexts(rollback)
            .await
            .map_err(|error| Error::config(format!("failed to restore SRTP contexts: {error}")))
    }

    /// Irreversibly latch one media session into secure-only operation before
    /// asynchronous DTLS key agreement starts. DTLS packets continue through
    /// RFC 7983 demux, but RTP/RTCP cannot fall back to plaintext.
    #[cfg(feature = "dtls-srtp")]
    pub async fn require_secure_media(&self, dialog_id: &DialogId) -> Result<()> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "require_secure_media: RTP session is not a UdpRtpTransport".to_string(),
                )
            })?;
        udp_transport.require_secure_media();
        // Outbound audio waits for the handshake's keys. A re-negotiation of
        // a call that already has keys keeps sending under them.
        let keys_installed = udp_transport.srtp_enabled().await;
        drop(session_guard);
        if !keys_installed {
            self.set_secure_keying(dialog_id, SecureKeyingState::Pending);
        }
        Ok(())
    }

    /// Record where a dialog's media keying stands for the outbound audio
    /// gate in [`Self::encode_and_send_audio`].
    #[cfg(feature = "dtls-srtp")]
    fn set_secure_keying(&self, dialog_id: &DialogId, state: SecureKeyingState) {
        if let Some(mut wrapper) = self.rtp_sessions.get_mut(dialog_id) {
            wrapper.secure_keying = state;
            // A prepared G.711 sender bypasses this gate; invalidate it.
            wrapper.tx_gate.bump();
        }
    }

    /// Run a real DTLS-SRTP handshake (RFC 5764) for a dialog's RTP session
    /// and install the resulting keys the same way [`Self::install_srtp_contexts`]
    /// installs SDES ones — this is the DTLS-SRTP equivalent of that call,
    /// sharing the same underlying `UdpRtpTransport::set_srtp_contexts`.
    ///
    /// Must be called after [`Self::start_media`], with `remote_addr` set to
    /// the peer's RTP address from its SDP answer/offer (the handshake runs
    /// over the same socket RTP/RTCP already use, demultiplexed by the
    /// first byte of each datagram — see `rvoip_rtp_core::transport::dtls_bridge`).
    /// `expected_remote_fingerprint_sha256` is the fingerprint authenticated
    /// by SIP/SDP. It is checked before any SRTP context is installed.
    #[cfg(feature = "dtls-srtp")]
    pub async fn run_dtls_handshake_and_install(
        &self,
        dialog_id: &DialogId,
        identity: rtp_core::dtls_srtp::DtlsIdentity,
        role: DtlsRole,
        remote_addr: std::net::SocketAddr,
        expected_remote_fingerprint_sha256: [u8; 32],
        srtp_profiles: Vec<rtp_core::dtls_srtp::SrtpProtectionProfile>,
        timeout: std::time::Duration,
    ) -> Result<rtp_core::dtls_srtp::DtlsSrtpHandshakeResult> {
        let outcome = self
            .run_dtls_handshake_and_install_inner(
                dialog_id,
                identity,
                role,
                remote_addr,
                expected_remote_fingerprint_sha256,
                srtp_profiles,
                timeout,
            )
            .await;
        // Outbound audio was waiting on this handshake: release it, or make
        // every later send fail with the reason instead of dropping frames
        // forever.
        self.set_secure_keying(
            dialog_id,
            match &outcome {
                Ok(_) => SecureKeyingState::Ready,
                Err(error) => SecureKeyingState::Failed(error.to_string()),
            },
        );
        outcome
    }

    #[cfg(feature = "dtls-srtp")]
    #[allow(clippy::too_many_arguments)]
    async fn run_dtls_handshake_and_install_inner(
        &self,
        dialog_id: &DialogId,
        identity: rtp_core::dtls_srtp::DtlsIdentity,
        role: DtlsRole,
        remote_addr: std::net::SocketAddr,
        expected_remote_fingerprint_sha256: [u8; 32],
        srtp_profiles: Vec<rtp_core::dtls_srtp::SrtpProtectionProfile>,
        timeout: std::time::Duration,
    ) -> Result<rtp_core::dtls_srtp::DtlsSrtpHandshakeResult> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|r| r.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        let conn = {
            let session_guard = session_arc.lock().await;
            let transport = session_guard.transport();
            let udp_transport = transport
                .as_any()
                .downcast_ref::<rtp_core::transport::UdpRtpTransport>()
                .ok_or_else(|| {
                    Error::config(
                        "run_dtls_handshake_and_install: RTP session is not a UdpRtpTransport"
                            .to_string(),
                    )
                })?;
            udp_transport.set_remote_rtp_addr(remote_addr).await;
            udp_transport
                .dtls_conn_adapter()
                .map_err(|e| Error::config(format!("dtls_conn_adapter: {e}")))?
        };

        let handshake = async {
            match role {
                DtlsRole::Client => {
                    rtp_core::dtls_srtp::handshake_client(conn, identity, srtp_profiles).await
                }
                DtlsRole::Server => {
                    rtp_core::dtls_srtp::handshake_server(conn, identity, srtp_profiles).await
                }
            }
        };

        let result = tokio::time::timeout(timeout, handshake)
            .await
            .map_err(|_| Error::config("DTLS-SRTP handshake timed out".to_string()))?
            .map_err(|e| Error::config(format!("DTLS-SRTP handshake failed: {e}")))?;

        // Authenticate the peer certificate before deriving or installing any
        // media context. A mismatch leaves the already-latched transport with
        // no keys, so RTP/RTCP cannot fall through to plaintext.
        verify_dtls_fingerprint(
            result.remote_fingerprint_sha256,
            expected_remote_fingerprint_sha256,
        )?;

        let (send_ctx, recv_ctx) = dtls_srtp_contexts(&result, role)?;

        self.install_srtp_contexts(dialog_id, send_ctx, recv_ctx)
            .await?;
        info!("Installed DTLS-SRTP contexts on dialog {}", dialog_id);

        Ok(result)
    }

    /// Latch a dialog's separate RTCP socket into keying of its own, for
    /// DTLS-SRTP without RTP/RTCP multiplexing (RFC 5764 §4.1). From here on
    /// RTCP on that socket passes only under the keys
    /// [`Self::run_rtcp_dtls_handshake_and_install`] installs, and is
    /// dropped until then. Also latches secure media.
    ///
    /// Returns `false`, changing nothing, when the session has no separate
    /// RTCP socket (it multiplexes, or was started without
    /// [`RTCP_SEPARATE_PORT_PARAMETER`]).
    #[cfg(feature = "dtls-srtp")]
    pub async fn require_rtcp_component_keying(&self, dialog_id: &DialogId) -> Result<bool> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let session_guard = session_arc.lock().await;
        let transport = session_guard.transport();
        let udp_transport = transport
            .as_any()
            .downcast_ref::<rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "require_rtcp_component_keying: RTP session is not a UdpRtpTransport"
                        .to_string(),
                )
            })?;
        Ok(udp_transport.require_rtcp_component_keying())
    }

    /// Run the RTCP component's own DTLS-SRTP handshake on a dialog's
    /// separate RTCP socket and install the SRTCP keys it derives there
    /// (RFC 5764 §4.1).
    ///
    /// Without RTP/RTCP multiplexing each component carries its own DTLS
    /// association on its own 5-tuple. This one runs from the local RTCP
    /// port to the peer's RTCP address (its `a=rtcp:` address, or its RTP
    /// port + 1), with the same `role` and certificate (`identity`, a clone
    /// of the one the RTP handshake uses) and the same expected
    /// fingerprint, because one `a=setup` and one `a=fingerprint` cover the
    /// whole media section. Keys map to directions as for RTP: the DTLS
    /// client sends under the client write key. Call
    /// [`Self::require_rtcp_component_keying`] first.
    ///
    /// On error nothing is installed and the RTCP port stays closed to
    /// RTCP; RTP keyed by [`Self::run_dtls_handshake_and_install`] is not
    /// affected.
    #[cfg(feature = "dtls-srtp")]
    pub async fn run_rtcp_dtls_handshake_and_install(
        &self,
        dialog_id: &DialogId,
        identity: rtp_core::dtls_srtp::DtlsIdentity,
        role: DtlsRole,
        expected_remote_fingerprint_sha256: [u8; 32],
        srtp_profiles: Vec<rtp_core::dtls_srtp::SrtpProtectionProfile>,
        timeout: std::time::Duration,
    ) -> Result<rtp_core::dtls_srtp::DtlsSrtpHandshakeResult> {
        let session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let conn = {
            let session_guard = session_arc.lock().await;
            let transport = session_guard.transport();
            let udp_transport = transport
                .as_any()
                .downcast_ref::<rtp_core::transport::UdpRtpTransport>()
                .ok_or_else(|| {
                    Error::config(
                        "run_rtcp_dtls_handshake_and_install: RTP session is not a UdpRtpTransport"
                            .to_string(),
                    )
                })?;
            if !udp_transport.rtcp_component_keying_required() {
                return Err(Error::config(
                    "the RTCP component is not latched for its own DTLS-SRTP keys".to_string(),
                ));
            }
            udp_transport
                .dtls_rtcp_conn_adapter()
                .map_err(|e| Error::config(format!("dtls_rtcp_conn_adapter: {e}")))?
        };

        let handshake = async {
            match role {
                DtlsRole::Client => {
                    rtp_core::dtls_srtp::handshake_client(conn, identity, srtp_profiles).await
                }
                DtlsRole::Server => {
                    rtp_core::dtls_srtp::handshake_server(conn, identity, srtp_profiles).await
                }
            }
        };
        let result = tokio::time::timeout(timeout, handshake)
            .await
            .map_err(|_| Error::config("RTCP DTLS-SRTP handshake timed out".to_string()))?
            .map_err(|e| Error::config(format!("RTCP DTLS-SRTP handshake failed: {e}")))?;
        verify_dtls_fingerprint(
            result.remote_fingerprint_sha256,
            expected_remote_fingerprint_sha256,
        )?;
        let (send_ctx, recv_ctx) = dtls_srtp_contexts(&result, role)?;

        let session_guard = session_arc.lock().await;
        session_guard
            .transport()
            .as_any()
            .downcast_ref::<rtp_core::transport::UdpRtpTransport>()
            .ok_or_else(|| {
                Error::config(
                    "run_rtcp_dtls_handshake_and_install: RTP session is not a UdpRtpTransport"
                        .to_string(),
                )
            })?
            .install_rtcp_component_srtp_contexts(send_ctx, recv_ctx)
            .map_err(|e| Error::config(format!("installing RTCP component SRTCP keys: {e}")))?;
        info!(
            "Installed RTCP-component DTLS-SRTP contexts on dialog {}",
            dialog_id
        );
        Ok(result)
    }

    fn cleanup_per_dialog_side_state(&self, dialog_id: &DialogId) {
        self.media_directions.remove(dialog_id);
        self.advanced_processors.remove(dialog_id);
        self.audio_frame_callbacks.remove(dialog_id);
        self.dtmf_callbacks.remove(dialog_id);
        self.cn_gate_state.remove(dialog_id);
        if let Some((_, runtime)) = self.codec_runtimes.remove(dialog_id) {
            runtime
                .encoded_generation_live
                .store(false, std::sync::atomic::Ordering::Release);
        }
        self.retire_inline_lease(dialog_id);

        let media_id = MediaSessionId::from_dialog(dialog_id);
        let mut owners = self
            .session_mapping_owners
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, session_id)) = self.media_to_session.remove(&media_id) {
            // A raw session identifier can be rebound to a newer media
            // generation before delayed cleanup of the old reverse entry is
            // observed. Remove the forward entry only when it still points at
            // the exact media id being stopped.
            self.session_to_media
                .remove_if(&session_id, |_, mapped| mapped == &media_id);
        }

        // Only this media's own aliases, never a scan of every session.
        if let Some(session_ids) = owners.remove(&media_id) {
            for session_id in session_ids {
                self.session_to_media
                    .remove_if(&session_id, |_, mapped| mapped == &media_id);
            }
        }
    }

    /// Stop media session for a dialog.
    ///
    /// Idempotent: repeated cleanup attempts still clear callbacks and side
    /// maps and then return `Ok(())`.
    pub async fn stop_media(&self, dialog_id: &DialogId) -> Result<()> {
        let stop_started = Instant::now();
        info!("Stopping media session for dialog: {}", dialog_id);

        // If the session is bridged, clear the partnership so the partner's
        // forwarder task can exit cleanly rather than sending to a dead
        // session. Does nothing when no bridge is active.
        self.clear_bridge_partner(dialog_id);

        // Remove session and get info for cleanup. DashMap removes
        // are sharded; no outer guard.
        let session_info = match self.sessions.remove(dialog_id) {
            Some((_, info)) => {
                #[cfg(feature = "memory-diagnostics")]
                rvoip_infra_common::memory_diagnostics::record_dropped(
                    "media_core.media_session_info",
                    std::mem::size_of::<MediaSessionInfo>(),
                );
                info
            }
            None => {
                self.cleanup_per_dialog_side_state(dialog_id);
                diagnostics::record_stop_media(stop_started.elapsed());
                debug!(
                    "Media session for dialog {} was already stopped; cleared side state",
                    dialog_id
                );
                return Ok(());
            }
        };

        // Stop and remove RTP session. Extract the wrapper out of the
        // shard, drop the guard, then close the underlying session
        // outside the map's locking territory.
        if let Some((_, mut rtp_wrapper)) = self.rtp_sessions.remove(dialog_id) {
            // No cached TX sender may use this dialog again.
            rtp_wrapper.tx_gate.retire();
            if let Some(transmitter) = rtp_wrapper.audio_transmitter.take() {
                transmitter.stop().await;
            }
            let mut rtp_session = rtp_wrapper.session.lock().await;
            let _ = rtp_session.close().await;
            info!("✅ Stopped RTP session for dialog: {}", dialog_id);
        }

        // Release port via the appropriate allocator
        if session_info.rtp_port.is_some() {
            let allocator = if let Some(ref port_alloc) = self.port_allocator {
                // Use our custom port allocator
                port_alloc.clone()
            } else {
                // Fall back to global allocator
                GlobalPortAllocator::instance().await
            };

            let dialog_session_id = format!("dialog_{}", dialog_id);
            let release_started = Instant::now();
            if let Err(e) = allocator.release_session(&dialog_session_id).await {
                warn!("Failed to release ports for dialog {}: {}", dialog_id, e);
            }
            diagnostics::record_port_release(release_started.elapsed());
        }

        self.cleanup_per_dialog_side_state(dialog_id);

        // Send event
        let _ = self.event_tx.send(MediaSessionEvent::SessionDestroyed {
            dialog_id: dialog_id.clone(),
            session_id: dialog_id.clone(),
        });

        diagnostics::record_stop_media(stop_started.elapsed());
        Ok(())
    }

    /// Close a dialog's separate RTCP socket and hand its port back to the
    /// allocator, once RTP/RTCP multiplexing makes it unnecessary. RTCP uses
    /// the RTP socket from then on. Returns whether a port was released;
    /// `false` when the session has none (already released, or started
    /// without [`RTCP_SEPARATE_PORT_PARAMETER`]).
    ///
    /// Call it only after multiplexing is committed: the port cannot be
    /// reserved again for this session, so a negotiation that may still roll
    /// back to a non-mux answer must keep it.
    pub async fn release_rtcp_port(&self, dialog_id: &DialogId) -> Result<bool> {
        let update_lock = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| Arc::clone(&entry.value().update_lock))
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let _update_guard = update_lock.lock().await;
        let rtp_session_arc = self
            .rtp_sessions
            .get(dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let released = rtp_session_arc.lock().await.release_rtcp_socket().await;
        let Some(released) = released else {
            return Ok(false);
        };
        if let Some(mut entry) = self.sessions.get_mut(dialog_id) {
            entry.value_mut().rtcp_port = None;
        }
        let allocator = if let Some(ref port_alloc) = self.port_allocator {
            port_alloc.clone()
        } else {
            GlobalPortAllocator::instance().await
        };
        let dialog_session_id = format!("dialog_{}", dialog_id);
        if !allocator
            .release_session_port(&dialog_session_id, released.ip(), released.port())
            .await
        {
            warn!(
                "RTCP port of dialog {} was not tracked by the allocator",
                dialog_id
            );
        }
        Ok(true)
    }

    /// Update media configuration (e.g., when remote address becomes known or codec changes during re-INVITE).
    ///
    /// Restructured for DashMap: each shard guard is acquired,
    /// mutated synchronously, and dropped before any `.await`. The
    /// actual RTP-session mutations happen on the per-session Arc
    /// after the map guards are released.
    pub async fn update_media(&self, dialog_id: DialogId, config: MediaConfig) -> Result<()> {
        info!("Updating media session for dialog: {}", dialog_id);

        let update_lock = self
            .rtp_sessions
            .get(&dialog_id)
            .map(|entry| Arc::clone(&entry.value().update_lock))
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let _update_guard = update_lock.lock().await;

        let old_config = self
            .sessions
            .get(&dialog_id)
            .map(|entry| entry.value().config.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;
        let old_runtime = self
            .codec_runtimes
            .get(&dialog_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        // A live bridge's framing guard ran once, when the bridge was made.
        // Re-check it here, before anything is mutated, so a re-INVITE that
        // would leave the bridge relaying a framing the far end cannot parse
        // is refused with the previous generation intact.
        self.revalidate_bridge_format(&dialog_id, &config)
            .map_err(|error| Error::config(error.to_string()))?;

        // Build replacement codec state before changing the RTP session or
        // published configuration. Any unsupported codec or invalid shape
        // leaves the preceding stable generation untouched.
        let new_format = codec_runtime::resolve_codec(&config)?;
        let codec_changed = old_runtime.format != new_format;
        let replacement_runtime = if codec_changed {
            Some(Arc::new(codec_runtime::DialogCodecRuntime::new(
                new_format.clone(),
            )?))
        } else {
            None
        };
        let remote_changed = config.remote_addr != old_config.remote_addr;
        let rtcp_mux_changed = config.rtcp_mux() != old_config.rtcp_mux();
        let remote_rtcp_changed = config.remote_rtcp_addr() != old_config.remote_rtcp_addr();
        let rtcp_xr_changed = config.rtcp_xr() != old_config.rtcp_xr();
        let rtcp_reduced_minimum_changed =
            config.rtcp_reduced_minimum() != old_config.rtcp_reduced_minimum();

        let rtp_session_arc = self
            .rtp_sessions
            .get(&dialog_id)
            .map(|entry| entry.value().session.clone())
            .ok_or_else(|| Error::session_not_found(dialog_id.as_str()))?;

        // A generated-audio task owns one codec generation. Stop it only
        // after the replacement runtime has been validated, then rebuild it
        // against the new generation before publishing the update.
        let transmitter_to_replace = if codec_changed {
            self.rtp_sessions.get_mut(&dialog_id).and_then(|mut entry| {
                entry
                    .value_mut()
                    .audio_transmitter
                    .take()
                    .map(|transmitter| {
                        let config = transmitter.replacement_config();
                        (transmitter, config)
                    })
            })
        } else {
            None
        };
        let replacement_transmitter_config = transmitter_to_replace
            .as_ref()
            .map(|(_, config)| config.clone());
        if let Some((transmitter, _)) = transmitter_to_replace {
            transmitter.stop().await;
        }

        {
            let mut rtp_session = rtp_session_arc.lock().await;
            // Before the peer address: a non-mux peer must never see a report.
            if rtcp_mux_changed {
                rtp_session.set_rtcp_mux(config.rtcp_mux());
            }
            if remote_rtcp_changed {
                rtp_session.set_remote_rtcp_addr(config.remote_rtcp_addr());
            }
            if rtcp_xr_changed {
                rtp_session.set_rtcp_xr_enabled(config.rtcp_xr());
            }
            if rtcp_reduced_minimum_changed {
                rtp_session.set_rtcp_reduced_minimum(config.rtcp_reduced_minimum());
            }
            if let Some(remote_addr) = config.remote_addr.filter(|_| remote_changed) {
                rtp_session.set_remote_addr(remote_addr).await;
            }
            if codec_changed {
                rtp_session.set_payload_type(new_format.payload_type);
                rtp_session.set_clock_rate(new_format.clock_rate);
            }
        }

        // Commit the externally visible generation only after all validation
        // and lower-layer application succeeds.
        if let Some(runtime) = replacement_runtime.as_ref() {
            self.codec_runtimes
                .insert(dialog_id.clone(), Arc::clone(runtime));
            // TX paths cached for the previous generation fall back now.
            old_runtime
                .encoded_generation_live
                .store(false, std::sync::atomic::Ordering::Release);
            self.resync_inline_lease(&dialog_id);
        }
        if let Some(mut entry) = self.rtp_sessions.get_mut(&dialog_id) {
            entry.value_mut().remote_addr = config.remote_addr;
        } else {
            return Err(Error::session_not_found(dialog_id.as_str()));
        }
        if let Some(mut entry) = self.sessions.get_mut(&dialog_id) {
            entry.value_mut().config = config.clone();
        } else {
            return Err(Error::session_not_found(dialog_id.as_str()));
        }

        if let (Some(transmitter_config), Some(runtime)) =
            (replacement_transmitter_config, replacement_runtime)
        {
            let mut replacement = audio_generation::AudioTransmitter::new_with_config(
                Arc::clone(&rtp_session_arc),
                transmitter_config,
                runtime,
            );
            replacement.start().await;
            if let Some(mut entry) = self.rtp_sessions.get_mut(&dialog_id) {
                entry.value_mut().audio_transmitter = Some(replacement);
            } else {
                return Err(Error::session_not_found(dialog_id.as_str()));
            }
        }

        if let Some(remote_addr) = config.remote_addr.filter(|_| remote_changed) {
            let _ = self.event_tx.send(MediaSessionEvent::RemoteAddressUpdated {
                dialog_id: dialog_id.clone(),
                remote_addr,
            });
        }
        if codec_changed {
            info!(
                "🔄 Codec changed for dialog {}: {} (PT {}, {}Hz) -> {} (PT {}, {}Hz)",
                dialog_id,
                old_runtime.format.name,
                old_runtime.format.payload_type,
                old_runtime.format.clock_rate,
                new_format.name,
                new_format.payload_type,
                new_format.clock_rate
            );
            let _ = self.event_tx.send(MediaSessionEvent::CodecChanged {
                dialog_id: dialog_id.clone(),
                old_codec: old_config.preferred_codec.clone(),
                new_codec: config.preferred_codec.clone(),
                new_payload_type: new_format.payload_type,
                new_clock_rate: new_format.clock_rate,
            });
        }

        if remote_changed || codec_changed || config != old_config {
            info!(
                "✅ Media session successfully updated for dialog: {}",
                dialog_id
            );
        } else {
            debug!("No RTP session updates needed for dialog: {}", dialog_id);
        }

        Ok(())
    }

    /// Whether a media session is published for `dialog_id`, without
    /// sampling RTP statistics (which locks the RTP session).
    pub fn has_media_session(&self, dialog_id: &DialogId) -> bool {
        self.sessions.contains_key(dialog_id)
    }

    /// The published configuration of a media session, without sampling RTP
    /// statistics (which locks the RTP session).
    pub fn get_session_config(&self, dialog_id: &DialogId) -> Option<MediaConfig> {
        self.sessions
            .get(dialog_id)
            .map(|entry| entry.value().config.clone())
    }

    /// Get information about a media session
    pub async fn get_session_info(&self, dialog_id: &DialogId) -> Option<MediaSessionInfo> {
        // Clone the MediaSessionInfo out of the shard, drop the
        // guard, then enrich with stats (an async call that may
        // re-touch other maps).
        let mut info = self.sessions.get(dialog_id).map(|r| r.value().clone())?;
        info.rtp_stats = self.get_rtp_statistics(dialog_id).await;
        info.stats_updated_at = Some(Instant::now());
        Some(info)
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<MediaSessionInfo> {
        self.sessions.iter().map(|r| r.value().clone()).collect()
    }

    // REMOVED: Channel-based communication - use GlobalEventCoordinator instead
    // pub async fn take_event_receiver(&self) -> Option<mpsc::UnboundedReceiver<MediaSessionEvent>> {
    //     let mut event_rx = self.event_rx.write().await;
    //     event_rx.take()
    // }

    /// Set audio frame callback for a dialog
    pub async fn set_audio_frame_callback(
        &self,
        dialog_id: DialogId,
        sender: mpsc::Sender<AudioFrame>,
    ) -> Result<()> {
        self.audio_frame_callbacks.insert(dialog_id.clone(), sender);
        self.resync_inline_lease(&dialog_id);
        info!("🔊 Set audio frame callback for dialog: {}", dialog_id);
        Ok(())
    }

    /// Remove audio frame callback for a dialog
    pub async fn remove_audio_frame_callback(&self, dialog_id: &DialogId) -> Result<()> {
        if self.audio_frame_callbacks.remove(dialog_id).is_some() {
            debug!("🔇 Removed audio frame callback for dialog: {}", dialog_id);
        }
        self.resync_inline_lease(dialog_id);
        Ok(())
    }

    /// Set RFC 4733 DTMF callback for a dialog. The callback fires once
    /// per digit (on the first observed `E=1` frame), so session-core
    /// can surface `Event::DtmfReceived` without dedup logic.
    pub async fn set_dtmf_callback(
        &self,
        dialog_id: DialogId,
        sender: mpsc::Sender<DtmfNotification>,
    ) -> Result<()> {
        self.dtmf_callbacks.insert(dialog_id.clone(), sender);
        info!("☎️  Set DTMF callback for dialog: {}", dialog_id);
        Ok(())
    }

    /// Remove RFC 4733 DTMF callback for a dialog.
    pub async fn remove_dtmf_callback(&self, dialog_id: &DialogId) -> Result<()> {
        if self.dtmf_callbacks.remove(dialog_id).is_some() {
            debug!("☎️  Removed DTMF callback for dialog: {}", dialog_id);
        }
        Ok(())
    }

    /// Send audio frame to session-core for a dialog. Extract the
    /// sender out of the DashMap shard and drop the guard before
    /// awaiting the send — `mpsc::Sender::send` is async and we don't
    /// want to hold a shard guard across it.
    pub async fn send_audio_frame(&self, dialog_id: &DialogId, frame: AudioFrame) -> Result<()> {
        let sender = self
            .audio_frame_callbacks
            .get(dialog_id)
            .map(|r| r.value().clone());
        if let Some(sender) = sender {
            if let Err(e) = sender.send(frame).await {
                warn!(
                    "Failed to send audio frame to session-core for dialog {}: {}",
                    dialog_id, e
                );
                return Err(Error::config(format!("Failed to send audio frame: {}", e)));
            }
            debug!(
                "📤 Sent audio frame to session-core for dialog: {}",
                dialog_id
            );
        }
        Ok(())
    }

    /// Spawn a task to handle RTP events and decode audio
    #[cfg(test)]
    fn spawn_rtp_event_handler(
        &self,
        dialog_id: DialogId,
        rtp_events: tokio::sync::broadcast::Receiver<rtp_core::session::RtpSessionEvent>,
        expected_payload_type: u8,
    ) {
        self.spawn_rtp_event_handler_with_inline(
            dialog_id,
            rtp_events,
            expected_payload_type,
            None,
        );
    }

    /// The v1 RTP event handler. With an inline receive lease (media
    /// executor v2), media packets normally bypass it: it still serves DTMF
    /// and stream events, takes over after a demotion (continuing the inline
    /// sink's source timeline), and leaves its loop when the inline sink saw
    /// the audio channel close, the point where it would have stopped itself.
    fn spawn_rtp_event_handler_with_inline(
        &self,
        dialog_id: DialogId,
        mut rtp_events: tokio::sync::broadcast::Receiver<rtp_core::session::RtpSessionEvent>,
        _expected_payload_type: u8,
        inline_lease: Option<Arc<inline_rx::DialogInlineLease>>,
    ) {
        let audio_frame_callbacks = self.audio_frame_callbacks.clone();
        let dtmf_callbacks = self.dtmf_callbacks.clone();
        let codec_runtimes = self.codec_runtimes.clone();
        let media_directions = self.media_directions.clone();

        // RFC 4733 §2.5.1.3 retransmit dedup formerly lived here as a
        // `(ssrc, rtp_timestamp)` seen-set. Sprint 2.5 P4 moved it
        // down to `rtp-core::transport::udp::UdpRtpTransport` (keyed
        // by `(peer_addr, ssrc, ts)`) so this handler now sees one
        // logical digit per tone — the upstream layer collapses the
        // three E=1 retransmits before they are even decoded into a
        // typed `DtmfEvent`.

        let skip_audio_frame_delivery = perf_skip_audio_frame_delivery();
        let collect_audio_quality = diagnostics::audio_quality_enabled();

        spawn_memory_tracked("media_core.rtp_event_handler_task", async move {
            info!("🎧 Started RTP event handler for dialog: {}", dialog_id);
            let mut rtp_count = 0u64;
            let mut last_sequence_number: Option<u16> = None;
            let mut last_rtp_timestamp: Option<u32> = None;
            let mut last_rtp_arrival: Option<Instant> = None;
            let mut jitter_ns = 0.0_f64;
            let mut rtp_playout = RtpPlayoutBuffer::new();
            let mut source_timeline = source_timeline::SourceTimeline::default();
            let mut receive_codec: Option<Arc<codec_runtime::DialogCodecRuntime>> = None;
            let mut metrics_ssrc = None;
            let mut decoded_playout = DecodedPlayoutQueue::new();
            let mut delivery_state = RtpDeliveryState::default();
            // A fixed 5 ms poll woke every session 200 times per second, even
            // before its first audio packet and after its queue emptied. Keep
            // one reusable timer and arm it only at the next reorder or audio
            // deadline. Channel closure and lifecycle cancellation still own
            // termination of this task.
            let playout_timer = tokio::time::sleep(Duration::from_secs(86_400));
            tokio::pin!(playout_timer);
            let mut armed_deadline = None;

            'events: loop {
                if inline_lease
                    .as_ref()
                    .is_some_and(|lease| lease.inline_stopped())
                {
                    break 'events;
                }
                let now = Instant::now();
                let items = rtp_playout.tick(now);
                if !items.is_empty() {
                    if let Some(codec_runtime) = codec_runtimes
                        .get(&dialog_id)
                        .map(|entry| entry.value().clone())
                    {
                        if enqueue_rtp_playout_items(
                            items,
                            &codec_runtime,
                            &dialog_id,
                            &mut decoded_playout,
                            &mut source_timeline,
                            now,
                        )
                        .await
                        {
                            break 'events;
                        }
                    }
                }
                if deliver_due_audio_frame(
                    now,
                    &mut decoded_playout,
                    &dialog_id,
                    &media_directions,
                    &audio_frame_callbacks,
                    collect_audio_quality,
                    skip_audio_frame_delivery,
                    &mut delivery_state,
                ) {
                    break 'events;
                }
                let deadline = match (
                    rtp_playout.next_deadline(now),
                    decoded_playout.next_deadline(),
                ) {
                    (Some(reorder), Some(audio)) => Some(reorder.min(audio)),
                    (reorder, audio) => reorder.or(audio),
                };
                if deadline != armed_deadline {
                    if let Some(deadline) = deadline {
                        playout_timer
                            .as_mut()
                            .reset(tokio::time::Instant::from_std(deadline));
                    }
                    armed_deadline = deadline;
                }
                let received = tokio::select! {
                    event = rtp_events.recv() => Some(event),
                    _ = &mut playout_timer, if deadline.is_some() => None,
                };
                if received.is_none() {
                    continue;
                }
                if inline_lease
                    .as_ref()
                    .is_some_and(|lease| lease.inline_stopped())
                {
                    break 'events;
                }
                match received.expect("received event checked above") {
                    Ok(event) => {
                        match event {
                            rtp_core::session::RtpSessionEvent::PacketReceived(packet) => {
                                // A demoted inline sink hands over the source
                                // timeline it built, so decoded timestamps stay
                                // on the same call timeline across the switch.
                                if let Some(handoff) = inline_lease
                                    .as_ref()
                                    .and_then(|lease| lease.take_v1_handoff())
                                {
                                    source_timeline = handoff.source_timeline;
                                    receive_codec = handoff.receive_codec;
                                }
                                rtp_count += 1;
                                let packet_arrival = collect_audio_quality.then(Instant::now);
                                let Some(codec_runtime) = codec_runtimes
                                    .get(&dialog_id)
                                    .map(|entry| entry.value().clone())
                                else {
                                    debug!("No codec runtime for dialog {}", dialog_id);
                                    continue;
                                };
                                if packet.header.payload_type != codec_runtime.format.payload_type {
                                    debug!(
                                        "Ignoring unnegotiated payload type {} for dialog {} (expected {})",
                                        packet.header.payload_type,
                                        dialog_id,
                                        codec_runtime.format.payload_type
                                    );
                                    continue;
                                }

                                if receive_codec
                                    .as_ref()
                                    .is_some_and(|previous| !Arc::ptr_eq(previous, &codec_runtime))
                                {
                                    // Packets buffered under the previous codec
                                    // cannot be decoded by this one. Frames already
                                    // decoded are valid audio and still play.
                                    source_timeline = source_timeline::SourceTimeline::default();
                                    rtp_playout = RtpPlayoutBuffer::new();
                                    metrics_ssrc = None;
                                }
                                receive_codec = Some(Arc::clone(&codec_runtime));
                                let arrived_at = Instant::now();
                                if !source_timeline.should_buffer(
                                    packet.header.ssrc,
                                    packet.header.sequence_number,
                                    packet.header.timestamp,
                                    arrived_at,
                                    codec_runtime.format.clock_rate,
                                ) {
                                    continue;
                                }
                                if metrics_ssrc != Some(packet.header.ssrc) {
                                    last_sequence_number = None;
                                    last_rtp_timestamp = None;
                                    last_rtp_arrival = None;
                                    jitter_ns = 0.0;
                                    metrics_ssrc = Some(packet.header.ssrc);
                                }

                                if rtp_count % 10 == 0
                                    || rtp_count == 100
                                    || rtp_count == 101
                                    || rtp_count > 100 && rtp_count < 110
                                {
                                    trace!(
                                        "📦 Received RTP packet #{} for dialog {}: PT={}, seq={}, ts={}, payload_size={}",
                                        rtp_count,
                                        dialog_id,
                                        packet.header.payload_type,
                                        packet.header.sequence_number,
                                        packet.header.timestamp,
                                        packet.payload.len()
                                    );
                                }

                                if let Some(arrival) = packet_arrival {
                                    let sequence_gap_packets =
                                        if let Some(previous) = last_sequence_number {
                                            let expected = previous.wrapping_add(1);
                                            if packet.header.sequence_number == expected {
                                                0
                                            } else {
                                                let gap = packet
                                                    .header
                                                    .sequence_number
                                                    .wrapping_sub(expected);
                                                if gap < 32_768 {
                                                    u64::from(gap)
                                                } else {
                                                    0
                                                }
                                            }
                                        } else {
                                            0
                                        };

                                    let interarrival_gap = last_rtp_arrival
                                        .map(|previous| arrival.duration_since(previous));
                                    if let (Some(previous_arrival), Some(previous_timestamp)) =
                                        (last_rtp_arrival, last_rtp_timestamp)
                                    {
                                        let arrival_delta =
                                            arrival.duration_since(previous_arrival).as_nanos()
                                                as f64;
                                        let rtp_delta_samples = packet
                                            .header
                                            .timestamp
                                            .wrapping_sub(previous_timestamp)
                                            as f64;
                                        let rtp_delta_ns = rtp_delta_samples * 1_000_000_000_f64
                                            / f64::from(codec_runtime.format.clock_rate);
                                        let transit_delta = (arrival_delta - rtp_delta_ns).abs();
                                        jitter_ns += (transit_delta - jitter_ns) / 16.0;
                                    }

                                    diagnostics::record_audio_rx_packet(
                                        sequence_gap_packets,
                                        interarrival_gap,
                                        Some(std::time::Duration::from_nanos(
                                            jitter_ns.max(0.0).min(u64::MAX as f64) as u64,
                                        )),
                                    );
                                    last_sequence_number = Some(packet.header.sequence_number);
                                    last_rtp_timestamp = Some(packet.header.timestamp);
                                    last_rtp_arrival = Some(arrival);
                                }

                                let playout_items = rtp_playout.push(packet, arrived_at);
                                if enqueue_rtp_playout_items(
                                    playout_items,
                                    &codec_runtime,
                                    &dialog_id,
                                    &mut decoded_playout,
                                    &mut source_timeline,
                                    arrived_at,
                                )
                                .await
                                {
                                    break 'events;
                                }
                                // A frame that is already due leaves on arrival
                                // rather than waiting up to one playout tick.
                                // The first frame of a stream is due the moment
                                // it is queued, so this keeps first-frame
                                // latency at the pre-pacing sub-millisecond
                                // level; later frames still pace on the tick.
                                if deliver_due_audio_frame(
                                    arrived_at,
                                    &mut decoded_playout,
                                    &dialog_id,
                                    &media_directions,
                                    &audio_frame_callbacks,
                                    collect_audio_quality,
                                    skip_audio_frame_delivery,
                                    &mut delivery_state,
                                ) {
                                    break 'events;
                                }
                            }
                            rtp_core::session::RtpSessionEvent::NewStreamDetected {
                                ssrc, ..
                            } => {
                                info!(
                                    "🎵 New RTP stream detected for dialog {}: SSRC={:08x}",
                                    dialog_id, ssrc
                                );
                            }
                            rtp_core::session::RtpSessionEvent::DtmfReceived {
                                event,
                                end_of_event,
                                duration,
                                ssrc: dtmf_ssrc,
                                ..
                            } => {
                                // Dedup is upstream now (Sprint 2.5 P4 —
                                // moved to UDP transport). We still gate
                                // on `end_of_event` because the sender
                                // emits start (E=0) + continuations
                                // (E=0) + 3× end (E=1); only the first
                                // E=1 reaches us, marking the tone
                                // boundary at which we fire the
                                // callback.
                                if !end_of_event {
                                    continue;
                                }
                                let digit = match event {
                                    0..=9 => (b'0' + event) as char,
                                    10 => '*',
                                    11 => '#',
                                    12..=15 => (b'A' + (event - 12)) as char,
                                    _ => '?',
                                };
                                // RFC 4733 telephone-event clock is
                                // 8 kHz by default; one timestamp tick
                                // is 1/8 ms, hence / 8.
                                let duration_ms = (duration as u32) / 8;
                                let notification = DtmfNotification {
                                    digit,
                                    duration_ms,
                                    ssrc: dtmf_ssrc,
                                };
                                let sender =
                                    dtmf_callbacks.get(&dialog_id).map(|r| r.value().clone());
                                if let Some(sender) = sender {
                                    match sender.try_send(notification) {
                                        Ok(_) => info!(
                                            "☎️  Delivered DTMF '{}' (duration={}ms) for dialog {}",
                                            digit, duration_ms, dialog_id
                                        ),
                                        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                            warn!(
                                                "DTMF callback buffer full for dialog {}; dropping '{}'",
                                                dialog_id, digit
                                            );
                                        }
                                        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                                            debug!(
                                                "DTMF callback channel closed for dialog {}; '{}' dropped",
                                                dialog_id, digit
                                            );
                                        }
                                    }
                                } else {
                                    debug!(
                                        "No DTMF callback registered for dialog {}; '{}' dropped",
                                        dialog_id, digit
                                    );
                                }
                            }
                            _ => {
                                // Other events we don't need to handle
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        error!(
                            "❌ RTP event handler lagged {} events for dialog {} - PACKET LOSS!",
                            n, dialog_id
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        info!(
                            "RTP event channel closed for dialog {}, stopping handler",
                            dialog_id
                        );
                        break;
                    }
                }
            }

            info!("🛑 RTP event handler stopped for dialog: {}", dialog_id);
        });
    }
}

impl Default for MediaSessionController {
    fn default() -> Self {
        Self::new()
    }
}

// Implementation modules are in separate files
