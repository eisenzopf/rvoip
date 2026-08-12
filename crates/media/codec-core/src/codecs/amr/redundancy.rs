//! RFC 4867 `max-red` redundancy: sending a frame more than once.
//!
//! # What redundancy is here
//!
//! AMR payloads may carry several frame-blocks (§4.3), and the format does not
//! distinguish "these are consecutive new frames" from "these are repeats of
//! frames I already sent". Redundancy is therefore not a separate mechanism —
//! it is the multi-frame payload used deliberately, re-sending recent frames
//! alongside the new one so a lost packet does not cost its audio outright.
//!
//! The cost is bandwidth: depth 2 sends every frame twice. `max-red` is the
//! peer's ceiling on it, in milliseconds between a frame's first transmission
//! and its last, so depth is `max_red / 20 + 1` frame-blocks.
//!
//! # The timestamp rule, which is easy to get backwards
//!
//! §4.3: frames in a payload are ordered **oldest first**, and the RTP
//! timestamp is that of the *first* frame — the oldest one, not the new one.
//! A receiver reconstructs each frame's timestamp by walking forward in 20 ms
//! steps. Getting this inverted produces audio that is subtly early or late by
//! the redundancy depth and drifts as the depth changes, which is far harder
//! to see than a packet that simply fails to parse.
//!
//! # What this module does not do
//!
//! It does not decide *whether* to use redundancy. That is a policy question
//! about loss and bandwidth, and we currently advertise `max-red=0` — no
//! redundancy — so nothing constructs a scheduler with a depth above one
//! unless a peer both permits it and a caller opts in.

use super::mode::AmrFrameType;
use super::payload::AmrPayloadFrame;
use crate::error::{CodecError, Result};
use std::collections::VecDeque;

/// One AMR frame-block, in milliseconds. Fixed by the codec for both variants.
pub const FRAME_BLOCK_MS: u16 = 20;

/// Builds the frame list for each outgoing payload, repeating recent frames up
/// to the negotiated depth.
///
/// Depth 1 is the no-redundancy case and is what every session gets unless a
/// peer declared a `max-red` above zero *and* a caller asked for it.
#[derive(Debug, Clone)]
pub struct RedundancyScheduler {
    /// Total frames per payload: one new plus `depth - 1` repeats.
    depth: usize,
    /// Previously sent frames, newest last. Never longer than `depth - 1`.
    history: VecDeque<AmrPayloadFrame>,
}

impl RedundancyScheduler {
    /// A scheduler for a negotiated `max-red`, in milliseconds.
    ///
    /// `None` means the peer declared no limit; RFC 4867 leaves that as "no
    /// restriction", but sending unbounded redundancy because a peer forgot to
    /// name a ceiling is not a defensible reading, so it is treated as no
    /// redundancy and the caller may raise it explicitly.
    ///
    /// # Errors
    ///
    /// When `requested_depth` exceeds what `max_red` permits, or exceeds the
    /// 32 frame-blocks a payload's table of contents can address.
    pub fn new(max_red_ms: Option<u16>, requested_depth: usize) -> Result<Self> {
        if requested_depth == 0 {
            return Err(CodecError::invalid_config(
                "redundancy depth is a frame count and must be at least 1",
            ));
        }
        if requested_depth > 32 {
            return Err(CodecError::invalid_config(
                "an AMR payload's table of contents addresses at most 32 frame-blocks",
            ));
        }
        let permitted = Self::permitted_depth(max_red_ms);
        if requested_depth > permitted {
            return Err(CodecError::invalid_config(format!(
                "redundancy depth {requested_depth} exceeds the {permitted} frame-blocks \
                 the peer's max-red allows"
            )));
        }
        Ok(Self {
            depth: requested_depth,
            history: VecDeque::new(),
        })
    }

    /// The deepest redundancy a `max-red` permits, in frame-blocks.
    ///
    /// `max-red` bounds the span from a frame's first transmission to its
    /// last, so `max-red=0` permits depth 1 (send once, span zero) and each
    /// further 20 ms permits one more copy.
    #[must_use]
    pub const fn permitted_depth(max_red_ms: Option<u16>) -> usize {
        match max_red_ms {
            // No ceiling declared: treated as no redundancy. See `new`.
            None => 1,
            Some(ms) => 1 + (ms / FRAME_BLOCK_MS) as usize,
        }
    }

    /// Frames per payload this scheduler emits.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// Take the next frame and return the payload's full frame list, oldest
    /// first — the order §4.3 requires.
    ///
    /// The returned list is at most `depth` long and shorter only at the start
    /// of a stream, before enough frames exist to repeat.
    pub fn next_payload(&mut self, frame: AmrPayloadFrame) -> Vec<AmrPayloadFrame> {
        let mut frames: Vec<AmrPayloadFrame> = self.history.iter().cloned().collect();
        frames.push(frame.clone());

        // Only speech and comfort-noise frames are worth repeating. A NO_DATA
        // repeat costs a table-of-contents entry to tell the receiver nothing
        // it cannot already infer from the timestamps.
        if matches!(
            frame.frame_type,
            AmrFrameType::Speech(_) | AmrFrameType::Sid(_)
        ) {
            self.history.push_back(frame);
        }
        while self.history.len() >= self.depth {
            self.history.pop_front();
        }
        frames
    }

    /// The RTP timestamp for a payload, given the new frame's timestamp and
    /// how many frames the payload carries.
    ///
    /// §4.3 again: the timestamp names the *oldest* frame, so a payload
    /// carrying two repeats plus one new frame is stamped 40 ms before the new
    /// frame's own time.
    #[must_use]
    pub fn payload_timestamp(
        newest_timestamp: u32,
        frame_count: usize,
        samples_per_frame: u32,
    ) -> u32 {
        // A payload addresses at most 32 frame-blocks, so this never
        // saturates in practice; clamping rather than casting keeps that true
        // even if a caller passes something absurd.
        let older = u32::try_from(frame_count.saturating_sub(1)).unwrap_or(u32::MAX);
        newest_timestamp.wrapping_sub(older.wrapping_mul(samples_per_frame))
    }
}

/// Drops frames a receiver has already decoded.
///
/// Redundancy means the same frame arrives more than once whenever nothing was
/// lost, which is the normal case — so without this every repeat would be
/// decoded a second time and the stream would run at a multiple of real time.
#[derive(Debug, Clone, Default)]
pub struct RedundancyDedup {
    /// Timestamp of the newest frame already decoded, if any.
    newest: Option<u32>,
}

impl RedundancyDedup {
    /// A dedup filter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self { newest: None }
    }

    /// Which frames of an arriving payload are new.
    ///
    /// `packet_timestamp` is the RTP timestamp — the *oldest* frame's time.
    /// Returns one flag per frame, in the payload's own oldest-first order.
    ///
    /// Frames at or before the newest already decoded are repeats and are
    /// dropped. Frames after it are new, including ones that skip ahead: a gap
    /// means the packets carrying those timestamps were lost outright and
    /// concealment, not this filter, is what answers for them.
    pub fn accept(
        &mut self,
        packet_timestamp: u32,
        frame_count: usize,
        samples_per_frame: u32,
    ) -> Vec<bool> {
        let mut flags = Vec::with_capacity(frame_count);
        for index in 0..frame_count {
            let offset = u32::try_from(index).unwrap_or(u32::MAX);
            let timestamp = packet_timestamp.wrapping_add(offset.wrapping_mul(samples_per_frame));
            // Wrapping-aware ordering: the RTP timestamp space is modular,
            // and a 32-bit counter at 8 kHz wraps every ~6 days. Comparing
            // with `>` would drop every frame for a whole epoch after the
            // wrap.
            let is_new = self.newest.is_none_or(|newest| {
                let ahead = timestamp.wrapping_sub(newest);
                ahead != 0 && ahead < u32::MAX / 2
            });
            if is_new {
                self.newest = Some(timestamp);
            }
            flags.push(is_new);
        }
        flags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::{AmrMode, AmrVariant};

    const NB: AmrVariant = AmrVariant::NarrowBand;
    /// 20 ms at 8 kHz.
    const NB_SAMPLES: u32 = 160;

    fn speech(index: u8) -> AmrPayloadFrame {
        let mode = AmrMode::new(NB, 7).expect("12.2 is a narrowband mode");
        AmrPayloadFrame::new(
            AmrFrameType::Speech(mode),
            true,
            vec![index; mode.octet_aligned_bytes()],
        )
        .expect("a full-length 12.2 frame")
    }

    #[test]
    fn max_red_bounds_the_depth() {
        // max-red is the span from first to last transmission, so zero permits
        // one transmission rather than none.
        assert_eq!(RedundancyScheduler::permitted_depth(Some(0)), 1);
        assert_eq!(RedundancyScheduler::permitted_depth(Some(20)), 2);
        assert_eq!(RedundancyScheduler::permitted_depth(Some(220)), 12);
        // An undeclared ceiling is not permission for unbounded redundancy.
        assert_eq!(RedundancyScheduler::permitted_depth(None), 1);

        assert!(RedundancyScheduler::new(Some(0), 2).is_err());
        assert!(RedundancyScheduler::new(Some(20), 2).is_ok());
        assert!(RedundancyScheduler::new(Some(20), 3).is_err());
        assert!(RedundancyScheduler::new(Some(0), 0).is_err());
        // The table of contents cannot address more than 32 frame-blocks
        // however generous the peer is.
        assert!(RedundancyScheduler::new(Some(10_000), 33).is_err());
    }

    #[test]
    fn depth_one_sends_each_frame_exactly_once() {
        let mut scheduler = RedundancyScheduler::new(Some(0), 1).expect("depth 1");
        for index in 0..4u8 {
            let payload = scheduler.next_payload(speech(index));
            assert_eq!(payload.len(), 1, "depth 1 must never bundle");
            assert_eq!(payload[0].data[0], index);
        }
    }

    #[test]
    fn deeper_payloads_carry_the_previous_frames_oldest_first() {
        let mut scheduler = RedundancyScheduler::new(Some(40), 3).expect("depth 3");

        // The stream's first payloads cannot be full: there is nothing yet to
        // repeat.
        let first = scheduler.next_payload(speech(0));
        assert_eq!(first.len(), 1);
        let second = scheduler.next_payload(speech(1));
        assert_eq!(second.len(), 2);
        assert_eq!(second.iter().map(|f| f.data[0]).collect::<Vec<_>>(), [0, 1]);

        // From here every payload is full, and the new frame is last.
        let third = scheduler.next_payload(speech(2));
        assert_eq!(
            third.iter().map(|f| f.data[0]).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        let fourth = scheduler.next_payload(speech(3));
        assert_eq!(
            fourth.iter().map(|f| f.data[0]).collect::<Vec<_>>(),
            [1, 2, 3],
            "the window must slide, not grow"
        );
    }

    #[test]
    fn the_payload_timestamp_names_the_oldest_frame() {
        // A payload carrying two repeats plus one new frame at t=1000 is
        // stamped 40 ms earlier, because §4.3 timestamps the first frame.
        assert_eq!(
            RedundancyScheduler::payload_timestamp(1_000, 3, NB_SAMPLES),
            1_000 - 2 * NB_SAMPLES
        );
        // One frame: the timestamp is its own.
        assert_eq!(
            RedundancyScheduler::payload_timestamp(1_000, 1, NB_SAMPLES),
            1_000
        );
    }

    #[test]
    fn repeats_are_dropped_when_nothing_is_lost() {
        let mut dedup = RedundancyDedup::new();
        // Depth 3, no loss: each payload repeats the two previous frames.
        // Only the newest frame of each is new.
        assert_eq!(dedup.accept(0, 1, NB_SAMPLES), [true]);
        assert_eq!(dedup.accept(0, 2, NB_SAMPLES), [false, true]);
        assert_eq!(dedup.accept(0, 3, NB_SAMPLES), [false, false, true]);
        assert_eq!(
            dedup.accept(NB_SAMPLES, 3, NB_SAMPLES),
            [false, false, true]
        );
    }

    #[test]
    fn a_lost_packet_is_recovered_from_the_next_one() {
        // The property redundancy exists for: drop a packet and the frame it
        // carried still arrives, inside its successor.
        let mut dedup = RedundancyDedup::new();
        assert_eq!(dedup.accept(0, 1, NB_SAMPLES), [true]);

        // The packet whose newest frame is t=160 never arrives. The next one
        // carries t=0, 160, 320 — and 160 is recovered rather than concealed.
        let flags = dedup.accept(0, 3, NB_SAMPLES);
        assert_eq!(flags, [false, true, true]);
    }

    #[test]
    fn every_frame_survives_a_one_in_two_loss_pattern_at_depth_two() {
        // A stronger statement of the same property, over a run: with depth 2
        // and every other packet dropped, the receiver still sees each
        // frame exactly once.
        let mut dedup = RedundancyDedup::new();
        let mut recovered = Vec::new();
        for index in 0..20u32 {
            let newest = index * NB_SAMPLES;
            // Depth 2, so each payload carries the previous frame and this
            // one — except the first, which has nothing to repeat.
            let frame_count = if index == 0 { 1 } else { 2 };
            let packet_timestamp =
                RedundancyScheduler::payload_timestamp(newest, frame_count, NB_SAMPLES);
            if index % 2 == 1 {
                continue; // this packet is lost
            }
            for (slot, is_new) in dedup
                .accept(packet_timestamp, frame_count, NB_SAMPLES)
                .into_iter()
                .enumerate()
            {
                if is_new {
                    recovered
                        .push(packet_timestamp + u32::try_from(slot).unwrap_or(0) * NB_SAMPLES);
                }
            }
        }
        // Every frame except the last one's: the run ends on a lost packet,
        // and depth 2 recovers a frame from its *successor* — which never
        // arrives. Redundancy bounds how much a loss costs, it does not make
        // the final packet of a stream optional.
        let expected: Vec<u32> = (0..19u32).map(|index| index * NB_SAMPLES).collect();
        assert_eq!(
            recovered, expected,
            "depth 2 must survive alternate-packet loss with no gaps and no repeats"
        );
    }

    #[test]
    fn the_timestamp_space_wraps_without_swallowing_an_epoch() {
        // 32 bits at 8 kHz wraps every ~6 days, and a naive `>` comparison
        // would treat everything after the wrap as old and drop it all.
        let mut dedup = RedundancyDedup::new();
        let before = u32::MAX - NB_SAMPLES;
        assert_eq!(dedup.accept(before, 1, NB_SAMPLES), [true]);
        // The next frame wraps past zero.
        assert_eq!(
            dedup.accept(before.wrapping_add(NB_SAMPLES), 1, NB_SAMPLES),
            [true],
            "a wrapped timestamp is newer, not older"
        );
    }

    #[test]
    fn no_data_frames_are_not_worth_repeating() {
        let mut scheduler = RedundancyScheduler::new(Some(40), 3).expect("depth 3");
        scheduler.next_payload(speech(0));
        // A NO_DATA frame goes out, but nothing repeats it later: it carries
        // no audio a receiver could not infer.
        let gap = AmrPayloadFrame::new(AmrFrameType::NoData, true, Vec::new())
            .expect("a NO_DATA frame");
        scheduler.next_payload(gap);
        let next = scheduler.next_payload(speech(2));
        assert!(
            next.iter()
                .all(|frame| !matches!(frame.frame_type, AmrFrameType::NoData)),
            "a NO_DATA frame was repeated as redundancy"
        );
    }
}
