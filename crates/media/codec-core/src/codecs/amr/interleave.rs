//! RFC 4867 §4.4.1 interleaving: putting the frame-blocks back in order.
//!
//! # What interleaving does and why a receiver must undo it
//!
//! A sender that interleaves spreads consecutive frame-blocks across several
//! packets, so one lost packet costs a scattering of single frames rather than
//! a contiguous burst. Concealment handles isolated losses far better than
//! bursts, which is the whole point.
//!
//! Payload `ILP` of a group of `ILL + 1` carries the frame-blocks at positions
//! `ILP`, `ILP + (ILL+1)`, `ILP + 2(ILL+1)`, … of the original sequence. A
//! receiver that ignores this and decodes payloads as they arrive produces
//! audio whose 20 ms blocks are shuffled — which sounds broken but parses
//! perfectly, so nothing upstream reports an error.
//!
//! # Receive-only, and what that does not buy
//!
//! This module handles interleaved payloads arriving *at* us. It does not make
//! interleaving usable end to end, because RFC 4867 §8.1 makes the fmtp
//! parameters declarative: a peer naming `interleaving` is saying it wants to
//! **receive** interleaved payloads, which obliges our transmit side. We do
//! not interleave on transmit, so such a session is still refused — see
//! `AmrAdapter::new` in media-core.
//!
//! What this closes is the parse-only gap on the direction we control: the
//! ILL/ILP fields the payload parser has always carried now mean something,
//! and a peer that interleaves toward us is handled correctly rather than
//! decoded in shuffled order.
//!
//! # Bounded by construction
//!
//! A group spans at most 16 packets (`ILL` is four bits) each carrying at most
//! 32 frame-blocks, and the reassembly buffer holds exactly one group. A peer
//! that never completes a group cannot make this grow: the group is flushed
//! when the next one starts, and missing positions are reported as lost frames
//! for concealment to answer rather than waited for indefinitely.

use super::payload::{AmrInterleaving, AmrPayloadFrame};

/// One frame-block's place in the reassembled stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deinterleaved {
    /// A frame that arrived, at its true position.
    Frame(AmrPayloadFrame),
    /// A position whose packet never arrived. The caller decodes this as a
    /// lost frame so concealment runs, exactly as for any other gap.
    Lost,
}

/// Reassembles interleaved frame-blocks into their original order.
///
/// Holds one interleaving group. Feed it every arriving payload's
/// interleaving fields and frames; it returns frames in decode order as each
/// group completes or is superseded.
#[derive(Debug, Clone, Default)]
pub struct Deinterleaver {
    /// Positions of the group being assembled, `None` where nothing arrived.
    slots: Vec<Option<AmrPayloadFrame>>,
    /// The group's `ILL`, so a packet from a differently-shaped group is
    /// recognised as a new group rather than merged into this one.
    ill: Option<u8>,
    /// Indices already seen this group, so a duplicate packet cannot
    /// overwrite a slot or be mistaken for a new group.
    seen: u16,
}

impl Deinterleaver {
    /// An empty de-interleaver.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            slots: Vec::new(),
            ill: None,
            seen: 0,
        }
    }

    /// Offer one payload's frames.
    ///
    /// Returns the frames of the *previous* group, in decode order, when this
    /// payload starts a new one — interleaving means a group's frames cannot
    /// be ordered until the group is done, so output necessarily lags arrival
    /// by up to `ILL` packets. That delay is what a deployment accepts when it
    /// negotiates interleaving.
    ///
    /// A group also completes as soon as every index has been seen, which is
    /// the common case and keeps the added delay at its minimum.
    pub fn push(
        &mut self,
        interleaving: AmrInterleaving,
        frames: Vec<AmrPayloadFrame>,
    ) -> Vec<Deinterleaved> {
        let group_len = usize::from(interleaving.group_len());
        let index = usize::from(interleaving.ilp);

        // A packet whose group shape differs, or whose index has already been
        // filled, belongs to a new group: flush what we have first.
        let starts_new_group = match self.ill {
            Some(ill) => ill != interleaving.ill || self.seen & (1u16 << index) != 0,
            None => false,
        };
        let mut flushed = if starts_new_group {
            self.flush()
        } else {
            Vec::new()
        };

        if self.ill.is_none() {
            self.ill = Some(interleaving.ill);
            // Positions are `index + n * group_len`, so the group holds
            // `group_len * frames_per_packet` blocks. Sized on first use from
            // the packet that opened it.
            self.slots = vec![None; group_len * frames.len().max(1)];
        }

        // Grow only if a later packet in the same group carries more frames
        // than the first did. Bounded by 16 * 32 by the field widths.
        let needed = index + (frames.len().saturating_sub(1)) * group_len + 1;
        if self.slots.len() < needed {
            self.slots.resize(needed, None);
        }

        for (offset, frame) in frames.into_iter().enumerate() {
            let position = index + offset * group_len;
            if let Some(slot) = self.slots.get_mut(position) {
                *slot = Some(frame);
            }
        }
        self.seen |= 1u16 << index;

        // Complete when every index of the group has been seen.
        if self.seen.count_ones() as usize >= group_len {
            let mut completed = self.flush();
            flushed.append(&mut completed);
        }
        flushed
    }

    /// Emit whatever is held, in order, and start fresh.
    ///
    /// Call this when the stream ends or a talk-spurt boundary makes waiting
    /// pointless; otherwise the group flushes on its own.
    pub fn flush(&mut self) -> Vec<Deinterleaved> {
        let out = self
            .slots
            .drain(..)
            .map(|slot| slot.map_or(Deinterleaved::Lost, Deinterleaved::Frame))
            .collect();
        self.ill = None;
        self.seen = 0;
        out
    }

    /// Frame-blocks currently buffered but not yet emitted.
    #[must_use]
    pub fn buffered(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::{AmrFrameType, AmrMode, AmrVariant};

    const NB: AmrVariant = AmrVariant::NarrowBand;

    fn frame(tag: u8) -> AmrPayloadFrame {
        let mode = AmrMode::new(NB, 7).expect("12.2 is a narrowband mode");
        AmrPayloadFrame::new(
            AmrFrameType::Speech(mode),
            true,
            vec![tag; mode.octet_aligned_bytes()],
        )
        .expect("a full-length frame")
    }

    fn tags(out: &[Deinterleaved]) -> Vec<Option<u8>> {
        out.iter()
            .map(|entry| match entry {
                Deinterleaved::Frame(frame) => Some(frame.data[0]),
                Deinterleaved::Lost => None,
            })
            .collect()
    }

    fn il(ill: u8, ilp: u8) -> AmrInterleaving {
        AmrInterleaving::new(ill, ilp).expect("valid interleaving fields")
    }

    /// The worked example from §4.4.1's description: a group of two, each
    /// packet carrying alternate frame-blocks.
    #[test]
    fn a_two_packet_group_reassembles_to_the_original_order() {
        let mut deinterleaver = Deinterleaver::new();

        // Packet 0 of 2 carries the frame-blocks at positions 0, 2, 4.
        let out = deinterleaver.push(il(1, 0), vec![frame(0), frame(2), frame(4)]);
        assert!(
            out.is_empty(),
            "a group cannot be ordered until it is complete"
        );

        // Packet 1 of 2 carries positions 1, 3, 5 — and completes the group.
        let out = deinterleaver.push(il(1, 1), vec![frame(1), frame(3), frame(5)]);
        assert_eq!(
            tags(&out),
            [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)],
            "the interleaved blocks must come back in their original order"
        );
    }

    /// The property interleaving exists for: one lost packet becomes scattered
    /// single-frame gaps rather than a burst.
    #[test]
    fn a_lost_packet_becomes_isolated_gaps_not_a_burst() {
        let mut deinterleaver = Deinterleaver::new();

        // Packet 1 of the group never arrives; packet 0 carries 0, 2, 4.
        deinterleaver.push(il(1, 0), vec![frame(0), frame(2), frame(4)]);
        // The next group's first packet supersedes the incomplete one.
        let out = deinterleaver.push(il(1, 0), vec![frame(10), frame(12), frame(14)]);

        assert_eq!(
            tags(&out),
            [Some(0), None, Some(2), None, Some(4), None],
            "losses must be single frames spread through the group"
        );
    }

    /// Three-packet groups, to show the stride is `ILL + 1` rather than 2.
    #[test]
    fn the_stride_is_the_group_length() {
        let mut deinterleaver = Deinterleaver::new();
        deinterleaver.push(il(2, 0), vec![frame(0), frame(3)]);
        deinterleaver.push(il(2, 1), vec![frame(1), frame(4)]);
        let out = deinterleaver.push(il(2, 2), vec![frame(2), frame(5)]);
        assert_eq!(
            tags(&out),
            [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)]
        );
    }

    /// A group of one is the degenerate case: no interleaving at all, and the
    /// frames pass straight through in arrival order.
    #[test]
    fn a_group_of_one_passes_frames_straight_through() {
        let mut deinterleaver = Deinterleaver::new();
        let out = deinterleaver.push(il(0, 0), vec![frame(0), frame(1)]);
        assert_eq!(tags(&out), [Some(0), Some(1)]);
    }

    /// A duplicate packet must not corrupt the group or be read as a new one
    /// mid-group.
    #[test]
    fn a_duplicated_packet_starts_a_new_group_rather_than_overwriting() {
        let mut deinterleaver = Deinterleaver::new();
        deinterleaver.push(il(1, 0), vec![frame(0), frame(2)]);
        // The same index again: the first group is flushed with its gaps
        // rather than having its slots rewritten.
        let out = deinterleaver.push(il(1, 0), vec![frame(20), frame(22)]);
        assert_eq!(tags(&out), [Some(0), None, Some(2), None]);
    }

    /// Memory is bounded by the field widths, and a peer that never completes
    /// a group cannot make the buffer grow without limit.
    #[test]
    fn an_endless_run_of_incomplete_groups_stays_bounded() {
        let mut deinterleaver = Deinterleaver::new();
        for _ in 0..1_000 {
            // Always index 0 of a 16-packet group: the group never completes,
            // and every packet supersedes the last.
            deinterleaver.push(il(15, 0), vec![frame(1), frame(2), frame(3)]);
            assert!(
                deinterleaver.buffered() <= 3,
                "an incomplete group must be flushed, not accumulated"
            );
        }
    }

    /// Flushing a partial group reports its gaps rather than dropping them
    /// silently, so concealment runs for the audio that never arrived.
    #[test]
    fn flush_reports_gaps_for_the_positions_that_never_arrived() {
        let mut deinterleaver = Deinterleaver::new();
        deinterleaver.push(il(3, 2), vec![frame(2)]);
        let out = deinterleaver.flush();
        assert_eq!(tags(&out), [None, None, Some(2), None]);
        assert_eq!(deinterleaver.buffered(), 0, "flush must reset the buffer");
    }
}
