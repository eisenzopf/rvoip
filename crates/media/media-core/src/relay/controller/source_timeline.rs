//! Collapse admitted audio RTP sources onto one decoded-call timeline.
//!
//! SSRC clocks are independent (RFC 3550). Resolve that distinction before
//! AudioFrame loses the source ID, not by clamping timestamps at a later sink.
//! This is for the controller's single-speaker receive path, not an RTP mixer.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const SOURCE_QUIET: Duration = Duration::from_millis(200);
const MAX_RETIRED: usize = 8;

#[derive(Clone, Copy)]
struct Active {
    packet: AcceptedPacket,
    arrival: Instant,
    duration_ticks: u32,
}

#[derive(Clone, Copy)]
struct Candidate {
    ssrc: u32,
    sequence: u16,
    arrival: Instant,
}

#[derive(Clone, Copy)]
pub(super) struct AcceptedPacket {
    ssrc: u32,
    sequence: u16,
    source_timestamp: u32,
    pub timestamp: u32,
    pub source_changed: bool,
}

#[derive(Default)]
pub(super) struct SourceTimeline {
    active: Option<Active>,
    candidate: Option<Candidate>,
    retired: VecDeque<u32>,
}

impl SourceTimeline {
    /// A replacement source needs two sequential packets within 200 ms.
    /// The first is probation and is not decoded. Recently replaced sources
    /// may return (e.g. hold/resume), but only after the active source is quiet.
    /// State is bounded regardless of how many SSRCs arrive on the admitted leg.
    pub fn prepare(
        &mut self,
        ssrc: u32,
        sequence: u16,
        timestamp: u32,
        arrival: Instant,
        clock_rate: u32,
    ) -> Option<AcceptedPacket> {
        let Some(active) = self.active else {
            return Some(AcceptedPacket {
                ssrc,
                sequence,
                source_timestamp: timestamp,
                // Preserve the original epoch until an actual source change.
                timestamp,
                source_changed: false,
            });
        };
        let previous = active.packet;
        let changed = ssrc != previous.ssrc;
        // Receive time elapsed since the anchor, in RTP ticks. Used only where
        // the sender's own clock cannot be trusted to place the packet.
        let elapsed_ticks = (arrival.saturating_duration_since(active.arrival).as_nanos()
            * u128::from(clock_rate)
            / 1_000_000_000)
            .min(u128::from(u32::MAX / 2)) as u32;
        let output = if !changed {
            // Sequence numbers, not timestamps, identify duplicates and late
            // packets (RFC 3550 section 5.1). Do not feed those into a stateful
            // decoder. Ordinary loss, DTX gaps and wrap survive.
            let sequence_delta = sequence.wrapping_sub(previous.sequence);
            if sequence_delta == 0 || sequence_delta >= (1 << 15) {
                return None;
            }
            let timestamp_delta = timestamp.wrapping_sub(previous.source_timestamp);
            if timestamp_delta == 0 || timestamp_delta >= (1 << 31) {
                // The sequence advanced but the clock did not: the sender
                // reset or stepped its RTP clock without changing SSRC.
                // Re-anchor here rather than discarding audio until the old
                // clock catches up, which could take hours.
                previous.timestamp.wrapping_add(
                    active
                        .duration_ticks
                        .saturating_mul(u32::from(sequence_delta))
                        .max(elapsed_ticks),
                )
            } else {
                previous.timestamp.wrapping_add(timestamp_delta)
            }
        } else {
            if self.retired.contains(&ssrc)
                && arrival.saturating_duration_since(active.arrival) < SOURCE_QUIET
            {
                return None;
            }
            // Sequence continuity is the probation test. The replacement's
            // own clock is not used to place it, so a sender whose timestamps
            // do not advance is not locked out.
            let validated = self.candidate.is_some_and(|candidate| {
                candidate.ssrc == ssrc
                    && sequence == candidate.sequence.wrapping_add(1)
                    && arrival.saturating_duration_since(candidate.arrival) < SOURCE_QUIET
            });
            if !validated {
                self.candidate = Some(Candidate {
                    ssrc,
                    sequence,
                    arrival,
                });
                return None;
            }
            // Use elapsed receive time only across independent clock epochs.
            // Within an epoch the sender's clock (including silence) is kept.
            previous
                .timestamp
                .wrapping_add(active.duration_ticks.max(elapsed_ticks))
        };
        Some(AcceptedPacket {
            ssrc,
            sequence,
            source_timestamp: timestamp,
            timestamp: output,
            source_changed: changed,
        })
    }

    /// Select sources before reordering without rejecting reorderable packets
    /// from the active source. A successful candidate remains valid until decode.
    pub fn should_buffer(
        &mut self,
        ssrc: u32,
        sequence: u16,
        timestamp: u32,
        arrival: Instant,
        clock_rate: u32,
    ) -> bool {
        if self.active.is_none_or(|active| active.packet.ssrc == ssrc) {
            return true;
        }
        self.prepare(ssrc, sequence, timestamp, arrival, clock_rate)
            .is_some()
    }

    pub fn map_gap(&self, timestamp: u32) -> u32 {
        self.active.map_or(timestamp, |active| {
            active
                .packet
                .timestamp
                .wrapping_add(timestamp.wrapping_sub(active.packet.source_timestamp))
        })
    }

    /// Commit only after successful decoding. Invalid payloads cannot replace
    /// the active source or advance its clock/sequence state.
    pub fn commit(&mut self, packet: AcceptedPacket, arrival: Instant, duration_ticks: u32) {
        if packet.source_changed {
            self.retired.retain(|ssrc| *ssrc != packet.ssrc);
            if let Some(previous) = self.active {
                if self.retired.len() == MAX_RETIRED {
                    self.retired.pop_front();
                }
                self.retired.push_back(previous.packet.ssrc);
            }
            self.candidate = None;
        }
        self.active = Some(Active {
            packet,
            arrival,
            duration_ticks: duration_ticks.max(1),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receive(t: &mut SourceTimeline, now: Instant, ssrc: u32, seq: u16, ts: u32) -> Option<u32> {
        let packet = t.prepare(ssrc, seq, ts, now, 48_000)?;
        t.commit(packet, now, 960);
        Some(packet.timestamp)
    }

    #[test]
    fn independent_clocks_and_overlapping_old_packets_remain_continuous() {
        for new_clock in [100, 456_000, u32::MAX - 2_000] {
            let mut t = SourceTimeline::default();
            let now = Instant::now();
            assert_eq!(receive(&mut t, now, 1, 10, 14_560), Some(14_560));
            assert_eq!(
                receive(&mut t, now + Duration::from_millis(10), 2, 80, new_clock),
                None
            );
            assert_eq!(
                receive(&mut t, now + Duration::from_millis(20), 1, 11, 15_520),
                Some(15_520)
            );
            assert_eq!(
                receive(
                    &mut t,
                    now + Duration::from_millis(30),
                    2,
                    81,
                    new_clock.wrapping_add(960)
                ),
                Some(16_480)
            );
            assert_eq!(
                receive(&mut t, now + Duration::from_millis(40), 1, 12, 16_480),
                None
            );
            assert_eq!(
                receive(
                    &mut t,
                    now + Duration::from_millis(50),
                    2,
                    82,
                    new_clock.wrapping_add(1920)
                ),
                Some(17_440)
            );
            assert_eq!(
                receive(&mut t, now + Duration::from_millis(60), 1, 13, 17_440),
                None
            );
        }
    }

    #[test]
    fn silence_loss_wrap_and_late_packets() {
        let now = Instant::now();
        let mut t = SourceTimeline::default();
        assert_eq!(
            receive(&mut t, now, 7, 65535, u32::MAX - 959),
            Some(u32::MAX - 959)
        );
        assert_eq!(receive(&mut t, now, 7, 0, 0), Some(0));
        assert_eq!(receive(&mut t, now, 7, 0, 0), None);
        assert_eq!(receive(&mut t, now, 7, 65535, u32::MAX - 959), None);
        // One lost packet, then five seconds of DTX without a source change.
        assert_eq!(receive(&mut t, now, 7, 2, 1920), Some(1920));
        assert_eq!(receive(&mut t, now, 7, 3, 241_920), Some(241_920));
    }

    /// A sender that keeps its SSRC and sequence continuity but resets or
    /// steps its RTP clock must keep playing, re-anchored onto the call
    /// timeline, instead of being treated as late until the old clock
    /// catches up.
    #[test]
    fn same_source_timestamp_discontinuity_reanchors_without_dropping() {
        let now = Instant::now();
        let ms = |n| now + Duration::from_millis(n);
        let mut t = SourceTimeline::default();
        assert_eq!(receive(&mut t, ms(0), 7, 10, 1_000_000), Some(1_000_000));
        assert_eq!(receive(&mut t, ms(20), 7, 11, 1_000_960), Some(1_000_960));
        // Clock reset to zero: re-anchored one frame later, not dropped.
        assert_eq!(receive(&mut t, ms(40), 7, 12, 0), Some(1_001_920));
        // The new clock is then followed exactly.
        assert_eq!(receive(&mut t, ms(60), 7, 13, 960), Some(1_002_880));
        // A stuck clock (no timestamp advance) still plays.
        assert_eq!(receive(&mut t, ms(80), 7, 14, 960), Some(1_003_840));
        // A forward step of 2^31 or more is indistinguishable from a reset.
        let stepped = 960u32.wrapping_add(1 << 31);
        assert_eq!(receive(&mut t, ms(100), 7, 15, stepped), Some(1_004_800));
        // A backward step after loss accounts for the lost packets...
        let back = stepped.wrapping_sub(50_000);
        assert_eq!(receive(&mut t, ms(120), 7, 18, back), Some(1_007_680));
        // ...and for receive time when that is longer (silence before reset).
        let again = back.wrapping_sub(1);
        assert_eq!(receive(&mut t, ms(1_120), 7, 19, again), Some(1_055_680));
        // Duplicates and late packets are still identified by sequence.
        assert_eq!(receive(&mut t, ms(1_140), 7, 19, again + 960), None);
        assert_eq!(receive(&mut t, ms(1_140), 7, 17, again + 960), None);
        assert_eq!(
            receive(&mut t, ms(1_140), 7, 20, again + 960),
            Some(1_056_640)
        );
    }

    #[test]
    fn replacement_source_with_a_stuck_clock_is_admitted() {
        let now = Instant::now();
        let mut t = SourceTimeline::default();
        receive(&mut t, now, 1, 1, 100).unwrap();
        assert_eq!(receive(&mut t, now, 2, 50, 7), None);
        assert_eq!(receive(&mut t, now, 2, 51, 7), Some(1060));
        assert_eq!(receive(&mut t, now, 2, 52, 7), Some(2020));
    }

    #[test]
    fn retired_source_can_resume_after_active_source_is_quiet() {
        let now = Instant::now();
        let mut t = SourceTimeline::default();
        receive(&mut t, now, 1, 1, 100).unwrap();
        assert_eq!(receive(&mut t, now, 2, 1, 5000), None);
        assert_eq!(receive(&mut t, now, 2, 2, 5960), Some(1060));
        assert_eq!(
            receive(&mut t, now + Duration::from_millis(300), 1, 2, 1060),
            None
        );
        assert_eq!(
            receive(&mut t, now + Duration::from_millis(320), 1, 3, 2020),
            Some(16_420)
        );
    }

    #[test]
    fn probation_and_failed_decode_do_not_commit_source() {
        let now = Instant::now();
        let mut t = SourceTimeline::default();
        receive(&mut t, now, 1, 1, 100).unwrap();
        assert!(t.prepare(2, 1, 1000, now, 48_000).is_none());
        assert!(t.prepare(2, 3, 2920, now, 48_000).is_none());
        let _failed_decode = t.prepare(2, 4, 3880, now, 48_000).unwrap();
        assert_eq!(receive(&mut t, now, 1, 2, 1060), Some(1060));
        // Expired probation cannot switch a source either.
        assert!(t
            .prepare(2, 5, 4840, now + Duration::from_secs(1), 48_000)
            .is_none());
    }

    #[test]
    fn source_history_is_bounded() {
        let now = Instant::now();
        let mut t = SourceTimeline::default();
        receive(&mut t, now, 1, 1, 100).unwrap();
        for source in 2..1000 {
            assert_eq!(receive(&mut t, now, source, 1, 100), None);
            assert!(receive(&mut t, now, source, 2, 1060).is_some());
            assert!(t.retired.len() <= MAX_RETIRED);
        }
    }
}
