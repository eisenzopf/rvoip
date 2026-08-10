//! What an AMR-WB decoder does when a frame does not arrive intact.
//!
//! Everything else in this crate reconstructs a frame from its bits. This
//! module reconstructs one from *history*, and it is the only part of the
//! decoder that runs exclusively when something has already gone wrong.
//!
//! # Two kinds of bad, and they are not interchangeable
//!
//! The reference draws a line that is easy to miss and expensive to blur:
//!
//! - **Damaged** ([`FrameQuality::Bad`], the reference's `bfi = 1`,
//!   `unusable_frame = 0`). The payload arrived; a checksum says some of it is
//!   wrong. The parameters that a single flipped bit would ruin — the spectrum,
//!   the gains, the pitch lag — are rebuilt from history, but the algebraic
//!   codebook indices and the LTP filter select bit are still *decoded from the
//!   payload*. A wrong pulse position costs one subframe of noise; a wrong lag
//!   or gain rings for hundreds of milliseconds, so only the latter are worth
//!   discarding. This is what a cleared RTP Q bit means.
//! - **Lost** ([`FrameQuality::Unusable`], `bfi = 1, unusable_frame = 1`).
//!   Nothing usable arrived at all. Now the innovation is white noise from the
//!   decoder's own generator and the LTP low-pass is forced off, because there
//!   are no bits to say otherwise.
//!
//! Treating a damaged frame as lost throws away perfectly good pulses and
//! sounds hollow; treating a lost frame as damaged decodes noise as if it were
//! signal and sounds like a burst of static. Both are audible, neither fails
//! a build, so the distinction is carried explicitly through every entry point
//! here rather than inferred.
//!
//! # The state machine has memory beyond the frame
//!
//! Concealment is not a per-frame function. Three pieces of state outlive the
//! frame that sets them:
//!
//! - a **severity counter** that climbs with each consecutive loss and selects
//!   how hard the gains are attenuated. Wideband *halves* it on a good frame
//!   rather than clearing it, so one clean frame in the middle of a bad patch
//!   does not reset the decoder's opinion of the channel. (Narrowband clears
//!   it. Copying the narrowband rule here mutes the wrong number of frames
//!   after a burst.)
//! - a **lag history** of the last five good subframes, plus their pitch
//!   gains, which together decide whether a received-but-suspect lag is
//!   plausible enough to keep.
//! - **two generators**, one for the substituted innovation and one for the
//!   jitter added to a substituted lag. They are separate in the reference and
//!   must stay separate: sharing one couples the pitch contour to the noise
//!   and drifts the moment either is consumed a different number of times.

use super::codebook::L_SUBFR;
use super::gain::FrameQuality;
use super::highband::NoiseGenerator;
use crate::fixed_point::arith::{add, mult, sub};
use crate::fixed_point::shift::shr;
use crate::fixed_point::types::{DspContext, Word16};

/// How many past good subframes the lag and gain histories span.
const HISTORY: usize = 5;

/// 1/3 in Q15, for averaging the three largest history lags.
const ONE_PER_3: Word16 = Word16(10923);

/// 1/5 in Q15, for the mean of the whole lag history.
const ONE_PER_HISTORY: Word16 = Word16(6554);

/// A pitch gain of 0.5 in Q14: above this the subframe was confidently voiced,
/// so its lag is worth trusting.
const CONFIDENT_GAIN: Word16 = Word16(8192);

/// A pitch gain of 0.4 in Q14: below this the history is unvoiced throughout
/// and no single lag in it deserves to be reused.
const WEAK_GAIN: Word16 = Word16(6554);

/// The widest lag spread, in samples, that counts as a steady pitch contour.
const STEADY_SPREAD: Word16 = Word16(10);

/// The widest lag spread that still lets a received lag inside the history's
/// range pass unchanged.
const LOOSE_SPREAD: Word16 = Word16(70);

/// Ceiling on the random jitter added to a substituted lag, in samples.
const MAX_JITTER: Word16 = Word16(40);

/// The reset pitch lag, in samples at 12.8 kHz — mid-range, and what the
/// history holds before any frame has been decoded.
const RESET_LAG: Word16 = Word16(64);

/// The severity counter's ceiling. Beyond six consecutive losses there is
/// nothing left to attenuate.
const MAX_SEVERITY: Word16 = Word16(6);

/// Everything the decoder must remember across frames in order to conceal one.
///
/// One instance per decoder. [`Self::begin_frame`] must be called once at the
/// top of every frame — good frames included, because that is where the
/// severity counter recovers and where `prev_bfi` rolls forward.
#[derive(Debug, Clone)]
pub struct Erasure {
    /// Consecutive-erasure severity, 0 to 6. Indexes the gain attenuation
    /// tables in [`super::gain`].
    severity: Word16,
    /// Whether the frame *before* the one now being decoded was bad.
    previous_bad: bool,
    /// Whether the frame now being decoded is bad, latched so the flag above
    /// can roll forward without the caller having to close the frame.
    current_bad: bool,
    lags: LagHistory,
    /// The reference's `seed`: substituted innovation, lost frames only.
    innovation: NoiseGenerator,
}

impl Default for Erasure {
    fn default() -> Self {
        Self::new()
    }
}

impl Erasure {
    /// A decoder that has seen nothing yet.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            severity: Word16(0),
            previous_bad: false,
            current_bad: false,
            lags: LagHistory::new(),
            innovation: NoiseGenerator::new(),
        }
    }

    /// Open a frame: roll the previous frame's verdict forward and move the
    /// severity counter.
    ///
    /// Call once per frame, before any subframe work, whatever the quality.
    pub fn begin_frame(&mut self, quality: FrameQuality) {
        let mut ctx = DspContext::default();
        self.previous_bad = self.current_bad;
        self.current_bad = quality != FrameQuality::Good;

        if self.current_bad {
            self.severity = add(&mut ctx, self.severity, Word16(1));
            if self.severity.0 > MAX_SEVERITY.0 {
                self.severity = MAX_SEVERITY;
            }
        } else {
            // Halved, not cleared. A single good frame inside a bad patch is
            // weak evidence that the channel recovered, and clearing here
            // un-mutes the output a frame or two too early.
            self.severity = shr(&mut ctx, self.severity, 1);
        }
    }

    /// The severity counter, as [`super::gain::GainDecoder::decode`] wants it.
    ///
    /// Q0, 0 to 6.
    #[must_use]
    pub fn severity(&self) -> usize {
        usize::from(self.severity.0.unsigned_abs())
    }

    /// Whether the frame before this one was bad.
    ///
    /// The reference's `prev_bfi`, which is a *frame*-level flag: all four
    /// subframes of a recovery frame see the same value. Exposed because the
    /// per-stage trace compares against the reference's copy of it.
    #[must_use]
    pub const fn previous_frame_was_bad(&self) -> bool {
        self.previous_bad
    }

    /// The pitch lag and fraction to actually use for a subframe, Q0 samples at
    /// 12.8 kHz and quarters.
    ///
    /// On a good frame this returns what was received. Otherwise the fraction
    /// is dropped — a quarter-sample position is far more precision than a
    /// guess deserves — and the lag is passed through [`LagHistory::substitute`].
    pub fn pitch_lag(
        &mut self,
        ctx: &mut DspContext,
        lag: u16,
        frac: u8,
        quality: FrameQuality,
    ) -> (u16, u8) {
        if quality == FrameQuality::Good {
            return (lag, frac);
        }
        let received = Word16(i16::try_from(lag).unwrap_or(RESET_LAG.0));
        let chosen = self
            .lags
            .substitute(ctx, received, quality == FrameQuality::Unusable);
        // Non-negative by construction: every value in the history entered it
        // through `note_good_subframe` from an unsigned lag, and the result is
        // clamped into the history's own range.
        (chosen.0.unsigned_abs(), 0)
    }

    /// Record a subframe that decoded cleanly.
    ///
    /// Only good subframes enter the history: a concealed lag is this module's
    /// own guess, and feeding it back would let one erasure define the pitch
    /// contour for the next five.
    pub fn note_good_subframe(&mut self, lag: u16, pitch_gain: Word16) {
        self.lags
            .push(Word16(i16::try_from(lag).unwrap_or(RESET_LAG.0)), pitch_gain);
    }

    /// A substituted innovation vector for a frame that carried no usable bits,
    /// Q9 like the algebraic codebook it replaces.
    ///
    /// The three-bit shift is not decoration: the gain decoder normalises by
    /// the vector's own energy, so an innovation at full scale would be scaled
    /// straight back down. Matching the codebook's headroom keeps the
    /// substitution at the level the rest of the chain expects.
    #[must_use]
    pub fn lost_innovation(&mut self, ctx: &mut DspContext) -> [Word16; L_SUBFR] {
        let mut code = [Word16(0); L_SUBFR];
        for slot in &mut code {
            let sample = self.innovation.next(ctx);
            *slot = shr(ctx, sample, 3);
        }
        code
    }
}

/// The last five good subframes' pitch lags and gains.
///
/// # Two orders, deliberately
///
/// The reference stores lags newest-first and gains oldest-first, and the
/// substitution logic reads `lag_hist[0]` and `gain_hist[4]` as "the most
/// recent" of each. Normalising them to one order here would be tidier and
/// would silently swap "the newest gain" for "the oldest" in three separate
/// tests, so the asymmetry is kept and named instead.
#[derive(Debug, Clone)]
struct LagHistory {
    /// Pitch lags of the last five good subframes, **newest first**.
    lags: [Word16; HISTORY],
    /// Pitch gains of the same subframes, Q14, **oldest first**.
    gains: [Word16; HISTORY],
    /// The most recent good lag, kept separately because the "hold what we had"
    /// branch wants it even after the history has been reordered.
    last_good: Word16,
    /// The reference's `seed3`: jitter for a substituted lag.
    jitter: NoiseGenerator,
}

impl LagHistory {
    /// A history seeded with the reset lag throughout.
    ///
    /// Not zero: a zero lag would make the spread look enormous and the very
    /// first erasure would take the substitution branch on nonsense.
    const fn new() -> Self {
        Self {
            lags: [RESET_LAG; HISTORY],
            gains: [Word16(0); HISTORY],
            last_good: RESET_LAG,
            jitter: NoiseGenerator::new(),
        }
    }

    /// Shift a good subframe in.
    fn push(&mut self, lag: Word16, pitch_gain: Word16) {
        self.lags.rotate_right(1);
        self.lags[0] = lag;
        self.gains.rotate_left(1);
        self.gains[HISTORY - 1] = pitch_gain;
        self.last_good = lag;
    }

    /// Decide the lag for a subframe of a bad frame, Q0 samples.
    ///
    /// `received` is the lag the payload decoded to, which is meaningful when
    /// the frame is merely damaged and meaningless when it is lost.
    ///
    /// # Why a received lag is often kept
    ///
    /// A damaged frame's lag index is usually intact — five or six bits out of
    /// several hundred. Substituting unconditionally would flatten the pitch
    /// contour of every damaged frame, which is audible as a monotone. So the
    /// received value is tested against the history for plausibility and only
    /// replaced when it is *implausible*: the tests below are all of the form
    /// "does this lag sit where the last five did".
    fn substitute(&mut self, ctx: &mut DspContext, received: Word16, lost: bool) -> Word16 {
        let newest_lag = self.lags[0];
        let newest_gain = self.gains[HISTORY - 1];
        let previous_gain = self.gains[HISTORY - 2];

        // Comparisons throughout are plain: lags live in 34..=231 and pitch
        // gains in 0..=16384, so no difference here can reach the saturation
        // point where `sub` and `-` would disagree.
        let min_lag = self.lags.iter().map(|w| w.0).min().unwrap_or(RESET_LAG.0);
        let max_lag = self.lags.iter().map(|w| w.0).max().unwrap_or(RESET_LAG.0);
        let weakest_gain = self.gains.iter().map(|w| w.0).min().unwrap_or(0);
        let spread = sub(ctx, Word16(max_lag), Word16(min_lag));

        // Voiced throughout, and the contour barely moved: last frame's lag is
        // as good an answer as exists.
        let steady_and_voiced = weakest_gain > CONFIDENT_GAIN.0 && spread.0 < STEADY_SPREAD.0;
        // The two most recent subframes were confidently voiced, so the newest
        // lag alone is trustworthy even if older history wandered.
        let recently_voiced =
            newest_gain.0 > CONFIDENT_GAIN.0 && previous_gain.0 > CONFIDENT_GAIN.0;

        if lost {
            // Nothing arrived, so there is no received lag to weigh.
            let chosen = if steady_and_voiced {
                self.last_good
            } else if recently_voiced {
                newest_lag
            } else {
                self.weighted_guess(ctx)
            };
            return clamp(chosen, min_lag, max_lag);
        }

        // The bits arrived. Five ways a received lag earns the benefit of the
        // doubt, in the reference's order.
        let above_floor = received.0 > sub(ctx, Word16(min_lag), Word16(5)).0;
        let near_max = sub(ctx, received, Word16(max_lag)).0 < 5;
        let inside = received.0 > min_lag && received.0 < max_lag;
        let near_newest = {
            let gap = sub(ctx, received, newest_lag).0;
            gap > -10 && gap < 10
        };
        let mean_lag = {
            let mut sum = Word16(0);
            for lag in &self.lags {
                sum = add(ctx, sum, *lag);
            }
            mult(ctx, sum, ONE_PER_HISTORY)
        };

        let plausible = (spread.0 < STEADY_SPREAD.0 && above_floor && near_max)
            || (recently_voiced && near_newest)
            || (weakest_gain < WEAK_GAIN.0 && newest_gain.0 == weakest_gain && inside)
            || (spread.0 < LOOSE_SPREAD.0 && inside)
            || (received.0 > mean_lag.0 && received.0 < max_lag);

        if plausible {
            return received;
        }

        // Implausible: fall back on the same three-way choice the lost case
        // makes, except that "hold the last good lag" becomes "hold the newest
        // history lag" — they differ only after a run of erasures, where
        // `last_good` is the older and therefore staler of the two.
        let chosen = if steady_and_voiced || recently_voiced {
            newest_lag
        } else {
            self.weighted_guess(ctx)
        };
        clamp(chosen, min_lag, max_lag)
    }

    /// A lag built from the history's three largest values, plus jitter.
    ///
    /// Biased high on purpose. Halving a pitch period is a far worse error than
    /// doubling one — it doubles the perceived pitch and is instantly audible —
    /// so when the history disagrees with itself the mean of its upper half is
    /// the safer guess. The jitter, spread over the gap between the largest and
    /// the median, stops a long erasure from producing an exactly periodic
    /// buzz at one frequency.
    fn weighted_guess(&mut self, ctx: &mut DspContext) -> Word16 {
        let mut sorted = self.lags.map(|w| w.0);
        sorted.sort_unstable();

        let mut gap = sub(ctx, Word16(sorted[4]), Word16(sorted[2]));
        if gap.0 > MAX_JITTER.0 {
            gap = MAX_JITTER;
        }
        let half = shr(ctx, gap, 1);
        let draw = self.jitter.next(ctx);
        let jitter = mult(ctx, half, draw);

        let pair = add(ctx, Word16(sorted[2]), Word16(sorted[3]));
        let upper = add(ctx, pair, Word16(sorted[4]));
        let mean = mult(ctx, upper, ONE_PER_3);
        add(ctx, mean, jitter)
    }
}

/// Hold a substituted lag inside the range the history actually visited.
///
/// The weighted guess and its jitter can both leave that range, and a lag the
/// talker never used is worse than a stale one.
const fn clamp(lag: Word16, min_lag: i16, max_lag: i16) -> Word16 {
    if lag.0 > max_lag {
        Word16(max_lag)
    } else if lag.0 < min_lag {
        Word16(min_lag)
    } else {
        lag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a run of good subframes with a given lag and gain.
    fn warm(erasure: &mut Erasure, lag: u16, gain: i16, count: usize) {
        for _ in 0..count {
            erasure.begin_frame(FrameQuality::Good);
            erasure.note_good_subframe(lag, Word16(gain));
        }
    }

    #[test]
    fn severity_halves_on_a_good_frame_rather_than_clearing() {
        // The single difference between the wideband and narrowband state
        // machines. Getting it wrong mutes the wrong number of frames after a
        // burst, which is audible and passes every per-stage test.
        let mut e = Erasure::new();
        for _ in 0..4 {
            e.begin_frame(FrameQuality::Bad);
        }
        assert_eq!(e.severity(), 4);
        e.begin_frame(FrameQuality::Good);
        assert_eq!(e.severity(), 2, "a good frame must halve, not clear");
        e.begin_frame(FrameQuality::Good);
        assert_eq!(e.severity(), 1);
        e.begin_frame(FrameQuality::Good);
        assert_eq!(e.severity(), 0);
    }

    #[test]
    fn severity_saturates_at_six() {
        let mut e = Erasure::new();
        for _ in 0..20 {
            e.begin_frame(FrameQuality::Bad);
        }
        assert_eq!(e.severity(), 6, "the attenuation tables only have seven rows");
    }

    #[test]
    fn prev_bfi_covers_a_whole_frame_not_one_subframe() {
        let mut e = Erasure::new();
        e.begin_frame(FrameQuality::Good);
        assert!(!e.previous_frame_was_bad());
        e.begin_frame(FrameQuality::Bad);
        assert!(!e.previous_frame_was_bad(), "the flag is about the frame before");
        e.begin_frame(FrameQuality::Good);
        assert!(e.previous_frame_was_bad());
        e.begin_frame(FrameQuality::Good);
        assert!(!e.previous_frame_was_bad());
    }

    #[test]
    fn a_steady_voiced_history_holds_its_lag_through_a_loss() {
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        warm(&mut e, 80, 12000, 6);

        e.begin_frame(FrameQuality::Unusable);
        // The received values are meaningless on a lost frame and must not
        // reach the output.
        let (lag, frac) = e.pitch_lag(&mut ctx, 200, 3, FrameQuality::Unusable);
        assert_eq!(lag, 80);
        assert_eq!(frac, 0, "a guessed lag does not get a quarter-sample fraction");
    }

    #[test]
    fn a_damaged_frames_lag_is_kept_when_the_history_agrees_with_it() {
        // A damaged frame's five-bit lag index is usually intact. Replacing it
        // regardless flattens the pitch contour into a monotone.
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        for lag in [78u16, 79, 80, 81, 82] {
            e.begin_frame(FrameQuality::Good);
            e.note_good_subframe(lag, Word16(11000));
        }
        e.begin_frame(FrameQuality::Bad);
        let (lag, _) = e.pitch_lag(&mut ctx, 81, 2, FrameQuality::Bad);
        assert_eq!(lag, 81, "a lag inside the history's own range was rejected");
    }

    #[test]
    fn a_damaged_frames_lag_is_rejected_when_it_is_nowhere_near_the_history() {
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        for lag in [40u16, 41, 42, 41, 40] {
            e.begin_frame(FrameQuality::Good);
            e.note_good_subframe(lag, Word16(2000));
        }
        e.begin_frame(FrameQuality::Bad);
        let (lag, _) = e.pitch_lag(&mut ctx, 220, 1, FrameQuality::Bad);
        assert!(
            (40..=42).contains(&lag),
            "a lag of 220 against a history of 40..42 survived as {lag}"
        );
    }

    #[test]
    fn a_substituted_lag_never_leaves_the_range_the_talker_used() {
        // The weighted guess plus jitter can overshoot; the clamp is what keeps
        // a concealed frame from inventing a pitch that was never spoken.
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        for lag in [50u16, 90, 60, 120, 70] {
            e.begin_frame(FrameQuality::Good);
            e.note_good_subframe(lag, Word16(1000));
        }
        for _ in 0..30 {
            e.begin_frame(FrameQuality::Unusable);
            let (lag, _) = e.pitch_lag(&mut ctx, 0, 0, FrameQuality::Unusable);
            assert!(
                (50..=120).contains(&lag),
                "substituted lag {lag} is outside the history's range"
            );
        }
    }

    #[test]
    fn the_lag_jitter_and_the_innovation_come_from_different_generators() {
        // They are separate seeds in the reference. Sharing one couples the
        // pitch contour to the noise and drifts as soon as either is consumed
        // a different number of times -- a divergence that only appears
        // several frames into a burst.
        let mut ctx = DspContext::default();

        let mut with_innovation = Erasure::new();
        warm(&mut with_innovation, 45, 1000, 5);
        let mut without = with_innovation.clone();

        // One decoder draws an innovation first; both then draw a lag.
        with_innovation.begin_frame(FrameQuality::Unusable);
        let _ = with_innovation.lost_innovation(&mut ctx);
        let a = with_innovation.pitch_lag(&mut ctx, 0, 0, FrameQuality::Unusable);

        without.begin_frame(FrameQuality::Unusable);
        let b = without.pitch_lag(&mut ctx, 0, 0, FrameQuality::Unusable);

        assert_eq!(a, b, "drawing an innovation moved the lag generator");
    }

    #[test]
    fn a_concealed_lag_never_enters_the_history() {
        // Otherwise one erasure defines the pitch contour for the next five
        // subframes, and a burst locks onto its own first guess.
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        warm(&mut e, 100, 13000, 5);

        for _ in 0..5 {
            e.begin_frame(FrameQuality::Unusable);
            let _ = e.pitch_lag(&mut ctx, 0, 0, FrameQuality::Unusable);
        }
        e.begin_frame(FrameQuality::Unusable);
        let (lag, _) = e.pitch_lag(&mut ctx, 0, 0, FrameQuality::Unusable);
        assert_eq!(lag, 100, "the history drifted, so concealed lags fed back into it");
    }

    #[test]
    fn a_substituted_innovation_stays_within_the_codebooks_headroom() {
        // The gain decoder normalises by this vector's energy, so a
        // full-scale substitution would just be scaled back down -- and the
        // dot product it is normalised with would overflow on the way.
        let mut ctx = DspContext::default();
        let mut e = Erasure::new();
        let code = e.lost_innovation(&mut ctx);
        assert_eq!(code.len(), L_SUBFR);
        assert!(
            code.iter().all(|c| c.0.abs() <= 4096),
            "the substituted innovation exceeds the codebook's Q9 headroom"
        );
        // It must actually vary; a constant vector is a dead generator.
        let distinct = code.iter().map(|c| c.0).collect::<std::collections::BTreeSet<_>>();
        assert!(distinct.len() > 32, "only {} distinct samples", distinct.len());
    }
}
