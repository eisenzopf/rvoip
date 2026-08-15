//! Which comfort-noise frames are actually transmitted.
//!
//! The encoder builds a full SID payload on every frame it decides is comfort
//! noise. Most of them are then not sent: this is the machine that decides,
//! and it is deliberately separate from the codec. TS 26.173 puts it in
//! `bits.c`'s `Write_serial` and TS 26.073 in `sid_sync.c`, in both cases
//! outside the encoder proper, because it is about the *channel* rather than
//! about the signal.
//!
//! The schedule it produces:
//!
//! ```text
//! S S S D . . D . . . . . . . D . . . . . . . D
//!       ^     ^               ^
//!       |     |               the strict period: 7 quiet, 1 update
//!       |     the first update, two frames after the first
//!       the first comfort-noise frame of a silence
//! ```
//!
//! `SID_FIRST` immediately on the transition out of speech, then two
//! `NO_DATA`, then `SID_UPDATE`, then one update every eight frames. The
//! receiver therefore learns the noise spectrum quickly and then cheaply.
//!
//! Both variants run the same machine. That is worth stating because almost
//! nothing else about their DTX is shared — the histories are different
//! lengths, the SID payloads are different shapes, and the two `tx_dtx_handler`
//! implementations are different state machines.

use super::mode::{AmrFrameType, AmrVariant};

/// Frames between comfort-noise updates once the cadence has settled.
const SID_UPDATE_PERIOD: i16 = 8;

/// How many frames the counter is set to after the first SID of a silence.
///
/// Three, not eight: the first update follows the first frame closely so the
/// receiver is not left synthesising from the `SID_FIRST`'s empty payload for
/// long.
const SID_FIRST_FOLLOWUP: i16 = 3;

/// The transmit-side frame-type scheduler.
#[derive(Debug, Clone)]
pub struct SidCadence {
    variant: AmrVariant,
    /// Frames until the next update is due.
    countdown: i16,
    /// Updates owed from a handover, which nothing in either reference ever
    /// sets. Kept because the decrement is part of the machine and its absence
    /// would be an unexplained gap rather than a simplification.
    handover_debt: i16,
    /// Whether the previous frame was speech, which is what makes a
    /// comfort-noise frame the *first* of its silence.
    previous_was_speech: bool,
    /// Whether the most recent SID was an update rather than a first.
    last_sid_was_update: bool,
}

impl SidCadence {
    /// A scheduler in its reset state.
    #[must_use]
    pub const fn new(variant: AmrVariant) -> Self {
        Self {
            variant,
            countdown: SID_FIRST_FOLLOWUP,
            handover_debt: 0,
            previous_was_speech: true,
            last_sid_was_update: false,
        }
    }

    /// The frame type to transmit, given what the encoder produced.
    ///
    /// `comfort_noise` is the encoder's decision, not the transmission's: a
    /// frame can be comfort noise and still go out as `NO_DATA`, which is the
    /// common case.
    #[allow(clippy::missing_const_for_fn)]
    pub fn next(&mut self, comfort_noise: bool, mode: super::mode::AmrMode) -> AmrFrameType {
        if !comfort_noise {
            self.countdown = SID_UPDATE_PERIOD;
            self.previous_was_speech = true;
            return AmrFrameType::Speech(mode);
        }

        self.countdown -= 1;
        let frame_type = if self.previous_was_speech {
            // The transition out of speech. The countdown restarts short so
            // the real update follows quickly.
            self.countdown = SID_FIRST_FOLLOWUP;
            self.last_sid_was_update = false;
            AmrFrameType::Sid(self.variant)
        } else if self.handover_debt > 0 && self.countdown > 2 {
            self.handover_debt -= 1;
            self.last_sid_was_update = true;
            AmrFrameType::Sid(self.variant)
        } else if self.countdown == 0 {
            self.countdown = SID_UPDATE_PERIOD;
            self.last_sid_was_update = true;
            AmrFrameType::Sid(self.variant)
        } else {
            AmrFrameType::NoData
        };
        self.previous_was_speech = false;
        frame_type
    }

    /// Whether the SID just scheduled was an update rather than the first of
    /// its silence.
    ///
    /// This is the STI bit: clear on a `SID_FIRST`, set on a `SID_UPDATE`. The
    /// two carry the same frame type on the wire and are told apart only by
    /// it, and a `SID_FIRST`'s 35 payload bits are zeroed — the receiver is
    /// meant to keep synthesising from whatever it already had.
    #[must_use]
    pub const fn last_sid_was_an_update(&self) -> bool {
        self.last_sid_was_update
    }

    /// Reset to the state a fresh stream starts in.
    #[allow(clippy::missing_const_for_fn)]
    pub fn reset(&mut self) {
        *self = Self::new(self.variant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrMode;

    fn mode() -> AmrMode {
        AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s")
    }

    #[test]
    fn the_schedule_is_first_then_two_gaps_then_every_eighth() {
        let mut cadence = SidCadence::new(AmrVariant::WideBand);
        let m = mode();
        for _ in 0..3 {
            assert_eq!(cadence.next(false, m), AmrFrameType::Speech(m));
        }

        let mut types = Vec::new();
        for _ in 0..20 {
            types.push(cadence.next(true, m));
        }

        let sid = AmrFrameType::Sid(AmrVariant::WideBand);
        assert_eq!(types[0], sid, "the first quiet frame is a SID");
        assert_eq!(types[1], AmrFrameType::NoData);
        assert_eq!(types[2], AmrFrameType::NoData);
        assert_eq!(types[3], sid, "the first update follows after two gaps");
        // Then a strict period of eight.
        let updates: Vec<usize> = types
            .iter()
            .enumerate()
            .filter(|(_, &t)| t == sid)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(updates, vec![0, 3, 11, 19]);
    }

    #[test]
    fn a_talk_spurt_restarts_the_schedule() {
        let mut cadence = SidCadence::new(AmrVariant::WideBand);
        let m = mode();
        for _ in 0..6 {
            cadence.next(true, m);
        }
        assert_eq!(cadence.next(false, m), AmrFrameType::Speech(m));
        assert_eq!(
            cadence.next(true, m),
            AmrFrameType::Sid(AmrVariant::WideBand),
            "the frame after speech is always a SID_FIRST"
        );
    }

    #[test]
    fn both_variants_carry_their_own_sid_frame_type() {
        // FT 8 narrowband, FT 9 wideband. The machine is shared; the frame
        // type it names is not.
        let m = mode();
        let mut wide = SidCadence::new(AmrVariant::WideBand);
        let mut narrow = SidCadence::new(AmrVariant::NarrowBand);
        assert_eq!(
            wide.next(true, m).frame_type_index(),
            AmrVariant::WideBand.sid_frame_type()
        );
        let nb_mode = AmrMode::new(AmrVariant::NarrowBand, 4).expect("7.40 kbit/s");
        assert_eq!(
            narrow.next(true, nb_mode).frame_type_index(),
            AmrVariant::NarrowBand.sid_frame_type()
        );
    }
}
