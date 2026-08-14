//! Pitch lag decoding and the adaptive codebook, 3GPP TS 26.090 §5.6/§6.1.
//!
//! Implements the TS 26.073 fixed-point reference's `Dec_lag3` (`dec_lag3.c`),
//! `Dec_lag6` (`dec_lag6.c`) and `Pred_lt_3or6` (`pred_lt.c`), together with
//! the delta-window derivation and excitation-history maintenance that
//! `dec_amr.c` performs around them — those are part of the same contract, and
//! a lag decoder without them decodes the wrong index range.
//!
//! Validated bit-exactly against the `lag3`, `lag6`, `predlt3` and `predlt6`
//! sections of `testdata/nb_stages.txt`, which `tools/amrnb_stage_oracle.c`
//! produces by driving the reference's own functions.
//!
//! # The adaptive codebook filters in place
//!
//! [`Excitation::predict`] writes each output sample into the same buffer it is
//! reading. When the lag is shorter than a subframe — the common case for
//! female and child speech — later samples are interpolated from samples this
//! very call already wrote. That recursion *is* the standard's short-lag
//! behaviour, not an aliasing bug: it is what makes a 20-sample period fill a
//! 40-sample subframe.
//!
//! Hence [`Excitation`] owns one contiguous buffer instead of offering an
//! immutable history plus an output slice. The latter API cannot express the
//! recursion and produces plausible, wrong samples for every lag below 50.
//!
//! # What the assembler still has to do
//!
//! - The lag is decoded *before* the bad-frame substitutions, and `old_T0`'s
//!   graceful `+1` degradation happens *after* [`delta_window`] and the lag
//!   decoder have consumed the old value.
//! - The current subframe of [`Excitation`] must be overwritten with the
//!   **total** excitation (adaptive plus fixed contribution) before
//!   [`Excitation::advance`] shifts it into the history. Shifting the bare
//!   adaptive-codebook vector diverges from the second subframe of the first
//!   frame onward.
//! - [`Excitation::advance`] runs once per *subframe* in the decoder. The
//!   encoder shifts by a whole frame once per frame; using that cadence here
//!   reads the history at the wrong offset for subframes 1 to 3.

use super::decoder_tables::INTER_6_PRED;
use super::{L_INTERPOL, L_SUBFR, PIT_MAX, PIT_MIN, PIT_MIN_MR122};
use crate::fixed_point::arith::{add, mult, negate, round, sub};
use crate::fixed_point::arith32::l_mac;
use crate::fixed_point::shift::shl;
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Interpolation phases per sample.
const PHASE_COUNT: i16 = 6;

/// [`PHASE_COUNT`] as the stride between a phase's successive coefficients.
const UP_SAMP_MAX: usize = PHASE_COUNT as usize;

/// Taps per side of the interpolation filter.
const TAPS_PER_SIDE: usize = L_INTERPOL - 1;

/// Length of the polyphase filter, `UP_SAMP_MAX * TAPS_PER_SIDE + 1`.
/// Taps in the interpolation filter, `6 * 10 + 1`.
///
/// Not a free parameter: it is the length of [`INTER_6_PRED`], and the
/// assertion below fails the build rather than a test if a regenerated table
/// ever disagrees with the loop bounds here.
const FIR_SIZE: usize = UP_SAMP_MAX * TAPS_PER_SIDE + 1;

const _: () = assert!(
    INTER_6_PRED.len() == FIR_SIZE,
    "the generated interpolation filter is not the length this module indexes"
);

/// Past excitation retained ahead of the current subframe, `PIT_MAX +
/// L_INTERPOL`.
///
/// The whole span is live. The last slot looks dead — no valid bitstream
/// reaches it — but a 7.95 kbit/s delta index of 63, a legal six-bit codeword
/// the encoder never emits, decodes to a lag of 144 with a fraction that shifts
/// the filter one further sample back, and reads it. Sizing this 153 turns a
/// bit error into a panic.
const HISTORY: usize = PIT_MAX as usize + L_INTERPOL;

/// Total excitation buffer: the history followed by the subframe being built.
const EXC_LEN: usize = HISTORY + L_SUBFR;

/// Mode index of 7.95 kbit/s, the one rate with a six-bit delta lag index and
/// therefore a window twice as wide as everyone else's.
const MODE_MR795: u8 = 5;

/// Highest mode index using the four-bit anchored delta path: 4.75, 5.15, 5.90
/// and 6.70 kbit/s.
const MAX_FOUR_BIT_DELTA_MODE: u8 = 3;

/// Mode index of 12.2 kbit/s, the only rate at one-sixth resolution.
const MODE_MR122: u8 = 7;

/// `round(2^15 / 3)`.
///
/// `mult(x, THIRD)` is `floor(x / 3)` for every argument the lag decoders
/// reach. The floor matters: `mult` is an arithmetic `>> 15`, so
/// `mult(-1, THIRD)` is `-1`, where an integer division would give `0` and
/// silently move adaptive-codebook index 4 by a third of a sample in every
/// four-bit delta subframe.
const THIRD: Word16 = Word16(10923);

/// `ceil(2^15 / 6)` — rounded up, not to nearest.
///
/// 32768/6 is 5461.33, so the *nearest* integer is 5461. The reference rounds
/// up so the product never falls short of the exact quotient, which makes
/// `mult(x, SIXTH)` exactly `floor(x / 6)` for every argument the one-sixth
/// decoder produces (at most a nine-bit index plus five).
const SIXTH: Word16 = Word16(5462);

/// How finely a rate resolves the pitch lag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LagResolution {
    /// One third of a sample — every rate except 12.2 kbit/s.
    OneThird,
    /// One sixth of a sample — 12.2 kbit/s only.
    OneSixth,
}

impl LagResolution {
    /// Which rate a mode index uses.
    #[must_use]
    pub const fn for_mode(mode_index: u8) -> Self {
        if mode_index == MODE_MR122 {
            Self::OneSixth
        } else {
            Self::OneThird
        }
    }

    /// Fractional units per sample: 3 or 6.
    #[must_use]
    pub const fn units_per_sample(self) -> i32 {
        match self {
            Self::OneThird => 3,
            Self::OneSixth => 6,
        }
    }
}

/// A decoded pitch lag.
///
/// The effective lag is `integer + frac / units_per_sample`. Both fields are
/// plain Q0 integers — `frac` is a signed *count* of fractional units, not a
/// Q-format fraction, and is routinely negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PitchLag {
    /// Integer part, in samples.
    pub integer: Word16,
    /// Fractional part, as a signed count of `1/3` or `1/6` sample depending on
    /// [`resolution`](Self::resolution).
    pub frac: Word16,
    /// Which fraction [`frac`](Self::frac) counts. Carried with the lag so the
    /// adaptive codebook cannot be built with the wrong filter phase — the two
    /// resolutions share one table and differ only by a doubling.
    pub resolution: LagResolution,
}

impl PitchLag {
    /// A lag with no fractional part, as bad-frame concealment substitutes.
    #[must_use]
    pub const fn integral(integer: Word16, resolution: LagResolution) -> Self {
        Self {
            integer,
            frac: Word16(0),
            resolution,
        }
    }

    /// The lag expressed in fractional units, `integer * 3 + frac` or
    /// `integer * 6 + frac`.
    ///
    /// Useful for comparing lags across codes; the decoders are strictly
    /// monotone in this quantity.
    #[must_use]
    pub const fn units(self) -> i32 {
        self.integer.0 as i32 * self.resolution.units_per_sample() + self.frac.0 as i32
    }
}

/// The search range a delta-coded lag index is relative to.
///
/// The width is always the mode's `delta_frc_range`; both clamps in
/// [`delta_window`] preserve it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagWindow {
    /// Lowest integer lag in the range.
    pub min: Word16,
    /// Highest integer lag in the range.
    pub max: Word16,
}

/// Which of the two one-third-resolution delta codings a rate uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaCoding {
    /// Five or six bits addressing the window uniformly at one-third
    /// resolution — 7.40, 7.95 and 10.2 kbit/s.
    ///
    /// Neither the previous lag nor the window's upper bound is consulted.
    Uniform,
    /// Four bits anchored on the previous subframe's lag — 4.75, 5.15, 5.90 and
    /// 6.70 kbit/s.
    ///
    /// Only the middle eight of the sixteen codes carry a fraction; the outer
    /// ones step whole samples, which is how sixteen codes still span the
    /// ten-sample window.
    Anchored {
        /// Integer lag of the previous subframe, before any concealment update.
        previous_lag: Word16,
    },
}

/// Whether a subframe's pitch index is coded relative to the previous subframe.
///
/// Subframe 0 is always absolute and subframes 1 and 3 are always relative. The
/// interesting case is subframe 2: `dec_amr.c` derives its flag from the sample
/// offset, which is non-zero, then forces it to zero for every rate except 4.75
/// and 5.15 kbit/s. Those two are therefore the only rates that delta-code the
/// second half of the frame — miss it and they diverge from sample 80 of every
/// frame while the other six stay bit-exact.
#[must_use]
pub const fn is_delta_coded(mode_index: u8, subframe: usize) -> bool {
    match subframe {
        0 => false,
        2 => mode_index == 0 || mode_index == 1,
        _ => true,
    }
}

/// Which delta coding a rate's relative subframes use.
///
/// 12.2 kbit/s never reaches this: it decodes at one-sixth resolution through
/// [`delta_lag_1_6`], which has no such split.
#[must_use]
pub const fn delta_coding(mode_index: u8, previous_lag: Word16) -> DeltaCoding {
    if mode_index <= MAX_FOUR_BIT_DELTA_MODE {
        DeltaCoding::Anchored { previous_lag }
    } else {
        DeltaCoding::Uniform
    }
}

/// The delta search window for a one-third-resolution subframe.
///
/// `previous_lag` is the integer lag of the previous subframe, Q0, read
/// **before** bad-frame concealment increments it. The window is derived every
/// subframe, including absolute-coded ones where it goes unused, because the
/// reference does — and because the value it is derived from moves.
///
/// 7.95 kbit/s spends six bits on the delta index and gets a twenty-sample
/// window; every other one-third-resolution rate gets ten.
#[must_use]
pub fn delta_window(ctx: &mut DspContext, mode_index: u8, previous_lag: Word16) -> LagWindow {
    let (low, range) = if mode_index == MODE_MR795 {
        (Word16(10), Word16(19))
    } else {
        (Word16(5), Word16(9))
    };

    let mut min = sub(ctx, previous_lag, low);
    if sub(ctx, min, Word16(PIT_MIN)).0 < 0 {
        min = Word16(PIT_MIN);
    }
    let mut max = add(ctx, min, range);
    if sub(ctx, max, Word16(PIT_MAX)).0 > 0 {
        // Clamping the top drags the bottom with it, so the window keeps its
        // width and the index space stays fully used.
        max = Word16(PIT_MAX);
        min = sub(ctx, max, range);
    }
    LagWindow { min, max }
}

/// Decode an absolutely coded pitch index — `Dec_lag3` and `Dec_lag6`, first
/// branch.
///
/// `index` is eight bits at one-third resolution and nine at one-sixth, both
/// Q0. Used by subframe 0 of every rate and subframe 2 of all but 4.75 and 5.15
/// kbit/s.
///
/// The code space is split: low indices resolve the short lags finely, and
/// above the split point the lag is coded as a whole number of samples, because
/// a long period does not need a third of a sample of accuracy.
#[must_use]
pub fn absolute_lag(ctx: &mut DspContext, index: Word16, resolution: LagResolution) -> PitchLag {
    match resolution {
        LagResolution::OneThird => {
            if sub(ctx, index, Word16(197)).0 < 0 {
                let biased = add(ctx, index, Word16(2));
                let quotient = mult(ctx, biased, THIRD);
                let integer = add(ctx, quotient, Word16(19));
                // Three times the *final* lag, not three times the quotient.
                let tripled = triple(ctx, integer);
                let residue = sub(ctx, index, tripled);
                let frac = add(ctx, residue, Word16(58));
                PitchLag {
                    integer,
                    frac,
                    resolution,
                }
            } else {
                PitchLag::integral(sub(ctx, index, Word16(112)), resolution)
            }
        }
        LagResolution::OneSixth => {
            if sub(ctx, index, Word16(463)).0 < 0 {
                let biased = add(ctx, index, Word16(5));
                let quotient = mult(ctx, biased, SIXTH);
                let integer = add(ctx, quotient, Word16(17));
                let tripled = triple(ctx, integer);
                // Six times the lag, reached by doubling the tripled value
                // rather than by a second multiply.
                let sixfold = add(ctx, tripled, tripled);
                let residue = sub(ctx, index, sixfold);
                let frac = add(ctx, residue, Word16(105));
                PitchLag {
                    integer,
                    frac,
                    resolution,
                }
            } else {
                PitchLag::integral(sub(ctx, index, Word16(368)), resolution)
            }
        }
    }
}

/// `3v`, accumulated the way the reference does.
fn triple(ctx: &mut DspContext, v: Word16) -> Word16 {
    let doubled = add(ctx, v, v);
    add(ctx, doubled, v)
}

/// Decode a delta-coded pitch index at one-third resolution — `Dec_lag3`,
/// second branch.
///
/// `window` comes from [`delta_window`]; `coding` from [`delta_coding`].
/// `index` is Q0, four to six bits wide depending on the rate.
///
/// No range guard: five-bit rates use their entire code space, and the two
/// unused six-bit codewords of 7.95 kbit/s decode to a lag one sample past the
/// maximum rather than being rejected. Clamping them would change the audio a
/// corrupted frame produces.
#[must_use]
pub fn delta_lag_1_3(
    ctx: &mut DspContext,
    index: Word16,
    window: LagWindow,
    coding: DeltaCoding,
) -> PitchLag {
    let resolution = LagResolution::OneThird;
    match coding {
        DeltaCoding::Uniform => {
            let biased = add(ctx, index, Word16(2));
            let quotient = mult(ctx, biased, THIRD);
            let step = sub(ctx, quotient, Word16(1));
            // The integer lag has to be read off `step` before it is tripled;
            // reversing the two gives a plausible lag with a wrong fraction.
            let integer = add(ctx, step, window.min);
            let tripled = triple(ctx, step);
            let residue = sub(ctx, index, Word16(2));
            let frac = sub(ctx, residue, tripled);
            PitchLag {
                integer,
                frac,
                resolution,
            }
        }
        DeltaCoding::Anchored { previous_lag } => {
            // Pull the anchor into the window. Under the window derived by
            // `delta_window` these two clamps always land on `min + 5`, but
            // they are the code path shared with the encoder and are written
            // out rather than folded away.
            let mut anchor = previous_lag;
            let above = sub(ctx, anchor, window.min);
            if sub(ctx, above, Word16(5)).0 > 0 {
                anchor = add(ctx, window.min, Word16(5));
            }
            let below = sub(ctx, window.max, anchor);
            if sub(ctx, below, Word16(4)).0 > 0 {
                anchor = sub(ctx, window.max, Word16(4));
            }

            if sub(ctx, index, Word16(4)).0 < 0 {
                // Codes 0..3: whole samples below the anchor.
                let base = sub(ctx, anchor, Word16(5));
                PitchLag::integral(add(ctx, base, index), resolution)
            } else if sub(ctx, index, Word16(12)).0 < 0 {
                // Codes 4..11: the fractional region around the anchor. Index 4
                // is the one code whose quotient is negative, and the only
                // place a truncating division would go wrong.
                let biased = sub(ctx, index, Word16(5));
                let quotient = mult(ctx, biased, THIRD);
                let step = sub(ctx, quotient, Word16(1));
                let integer = add(ctx, step, anchor);
                let tripled = triple(ctx, step);
                let residue = sub(ctx, index, Word16(9));
                let frac = sub(ctx, residue, tripled);
                PitchLag {
                    integer,
                    frac,
                    resolution,
                }
            } else {
                // Codes 12..15: whole samples above the anchor.
                let above_anchor = sub(ctx, index, Word16(12));
                let base = add(ctx, above_anchor, anchor);
                PitchLag::integral(add(ctx, base, Word16(1)), resolution)
            }
        }
    }
}

/// Decode a delta-coded pitch index at one-sixth resolution — `Dec_lag6`,
/// second branch. 12.2 kbit/s only.
///
/// Unlike [`delta_lag_1_3`], this derives its own ten-sample window: the
/// reference keeps the derivation inside `Dec_lag6` rather than in the caller.
/// `previous_lag` is the integer lag carried out of the previous subframe, Q0.
///
/// The reference passes the lag bounds in as arguments; the decoder's only call
/// site passes 18 and 143, so they are constants here.
///
/// Indices 61 to 63 are unused. This does not reject them — the caller detects
/// them and substitutes the previous lag, and moving the test in here would
/// change what a bad frame sounds like.
#[must_use]
pub fn delta_lag_1_6(ctx: &mut DspContext, index: Word16, previous_lag: Word16) -> PitchLag {
    let mut min = sub(ctx, previous_lag, Word16(5));
    if sub(ctx, min, Word16(PIT_MIN_MR122)).0 < 0 {
        min = Word16(PIT_MIN_MR122);
    }
    let max = add(ctx, min, Word16(9));
    if sub(ctx, max, Word16(PIT_MAX)).0 > 0 {
        // `max` exists only to pull `min` down near the top of the range; it is
        // never read afterwards, but dropping it moves the window.
        min = sub(ctx, Word16(PIT_MAX), Word16(9));
    }

    let biased = add(ctx, index, Word16(5));
    let quotient = mult(ctx, biased, SIXTH);
    let step = sub(ctx, quotient, Word16(1));
    // Again the lag comes off `step` before `step` is tripled.
    let integer = add(ctx, step, min);
    let tripled = triple(ctx, step);
    let sixfold = add(ctx, tripled, tripled);
    let residue = sub(ctx, index, Word16(3));
    let frac = sub(ctx, residue, sixfold);
    PitchLag {
        integer,
        frac,
        resolution: LagResolution::OneSixth,
    }
}

/// The decoder's excitation signal: past excitation followed by the subframe
/// being built.
///
/// One contiguous buffer, because [`predict`](Self::predict) reads and writes
/// it at once. See the module documentation.
#[derive(Debug, Clone)]
pub struct Excitation {
    samples: [Word16; EXC_LEN],
}

impl Default for Excitation {
    fn default() -> Self {
        Self::new()
    }
}

impl Excitation {
    /// A silent history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            samples: [Word16(0); EXC_LEN],
        }
    }

    /// Clear the history.
    ///
    /// Needed on more than construction: the reference resets the decoder on
    /// every frame that is not speech — SID, comfort noise, no data — so a
    /// decoder that only clears this at startup carries stale pitch history
    /// through every silence.
    pub const fn reset(&mut self) {
        self.samples = [Word16(0); EXC_LEN];
    }

    /// The subframe just built, 40 samples, Q0.
    #[must_use]
    pub fn subframe(&self) -> &[Word16] {
        &self.samples[HISTORY..]
    }

    /// The subframe just built, for the caller to overwrite with the total
    /// excitation before [`advance`](Self::advance).
    pub fn subframe_mut(&mut self) -> &mut [Word16] {
        &mut self.samples[HISTORY..]
    }

    /// The whole buffer: 154 samples of history then the current subframe.
    #[must_use]
    pub const fn all(&self) -> &[Word16] {
        &self.samples
    }

    /// The whole buffer, mutable.
    ///
    /// The synthesis filter overflowing requires the entire span — history
    /// included — to be scaled down before the filter is re-run, which is the
    /// one operation that touches all of it at once.
    pub const fn all_mut(&mut self) -> &mut [Word16] {
        &mut self.samples
    }

    /// Shift the current subframe into the history.
    ///
    /// Once per subframe. The encoder does the equivalent once per frame over a
    /// longer buffer; borrowing that cadence here makes every subframe after
    /// the first read the history at the wrong offset.
    pub fn advance(&mut self) {
        self.samples.copy_within(L_SUBFR.., 0);
    }

    /// Build the adaptive codebook vector for this subframe — `Pred_lt_3or6`.
    ///
    /// Interpolates the past excitation at `lag` with a 20-tap FIR and leaves
    /// the result in [`subframe_mut`](Self::subframe_mut), Q0. The excitation
    /// is Q0 and the filter Q15, so the accumulator is Q16 and rounds back to
    /// Q0; the accumulator saturation is part of the defined output and shows
    /// up on loud voiced frames.
    ///
    /// A zero fraction is not a copy. The filter's DC gain is 0.9986, not 1,
    /// and even the zero phase is a full low-pass — short-circuiting it detunes
    /// the codebook.
    ///
    /// # Panics
    ///
    /// If `lag.integer` is outside what any pitch index can decode to. Every
    /// field width in the standard, paired with the window
    /// [`delta_window`] derives, bounds the integer lag at 144, so this cannot
    /// fire on a bitstream however corrupted — it is here so that a wiring
    /// mistake in the caller fails loudly rather than reading the wrong
    /// history.
    pub fn predict(&mut self, ctx: &mut DspContext, lag: PitchLag) {
        // The filter phase is the negation of the coded fraction; the coded
        // value counts forward, the phase counts back into the history.
        let mut phase = negate(ctx, lag.frac);
        if lag.resolution == LagResolution::OneThird {
            // One-third taps are the even one-sixth taps, so doubling the phase
            // reuses the single table.
            phase = shl(ctx, phase, 1);
        }

        // Plain signed compare on the value, as in the reference — a negative
        // phase is folded to a positive one by stepping the read position one
        // sample further back.
        let back = if phase.0 < 0 {
            phase = add(ctx, phase, Word16(PHASE_COUNT));
            1
        } else {
            0
        };

        let reach = usize::try_from(lag.integer.0)
            .ok()
            .and_then(|integer| integer.checked_add(back))
            .filter(|reach| (TAPS_PER_SIDE..=HISTORY - TAPS_PER_SIDE + 1).contains(reach))
            .unwrap_or_else(|| {
                panic!("pitch lag {} is outside the decodable range", lag.integer.0)
            });
        // Index of `exc[-T0]`, or one lower when the phase was folded.
        let base = HISTORY - reach;

        let lead = usize::try_from(phase.0).expect("interpolation phase is 0..=5");

        for j in 0..L_SUBFR {
            // The reference recomputes this every output sample. It is
            // loop-invariant, but it also clears the saturation flag that the
            // accumulator may have raised, so it stays where it was.
            let trail =
                usize::try_from(sub(ctx, Word16(PHASE_COUNT), phase).0).expect("phase complement");

            let backward = base + j;
            let forward = backward + 1;

            let mut acc = Word32(0);
            for i in 0..TAPS_PER_SIDE {
                // Six coefficients per sample of delay: plain index
                // arithmetic, which the reference carries as a second loop
                // counter rather than a multiply.
                let k = i * UP_SAMP_MAX;
                acc = l_mac(
                    ctx,
                    acc,
                    self.samples[backward - i],
                    Word16(INTER_6_PRED[lead + k]),
                );
                acc = l_mac(
                    ctx,
                    acc,
                    self.samples[forward + i],
                    Word16(INTER_6_PRED[trail + k]),
                );
            }

            // Written into the buffer the next iterations read from. See the
            // module documentation: this is the short-lag recursion.
            self.samples[HISTORY + j] = round(ctx, acc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::decoder_tables::{INTER_6_PRED, INTER_6_SEARCH};
    use super::super::vectors;
    use super::{
        absolute_lag, delta_coding, delta_lag_1_3, delta_lag_1_6, delta_window, is_delta_coded,
        DeltaCoding, Excitation, LagResolution, LagWindow, PitchLag, EXC_LEN, FIR_SIZE, HISTORY,
        MODE_MR795, TAPS_PER_SIDE, UP_SAMP_MAX,
    };
    use super::{L_INTERPOL, L_SUBFR, PIT_MAX, PIT_MIN};
    use crate::fixed_point::types::{DspContext, Word16};

    /// `Enc_lag3`'s index, in plain integer arithmetic.
    ///
    /// Deliberately not a transcription of the reference's operator calls: an
    /// independent derivation of the inverse mapping is what makes the
    /// round-trip below evidence rather than a tautology. Nothing here comes
    /// near a 16-bit boundary.
    fn encode_lag_1_3(lag: PitchLag, window: LagWindow, coding: Option<DeltaCoding>) -> i32 {
        let t0 = i32::from(lag.integer.0);
        let frac = i32::from(lag.frac.0);
        match coding {
            None => {
                if t0 <= 85 {
                    3 * t0 - 58 + frac
                } else {
                    t0 + 112
                }
            }
            Some(DeltaCoding::Uniform) => 3 * (t0 - i32::from(window.min.0)) + 2 + frac,
            Some(DeltaCoding::Anchored { previous_lag }) => {
                let min = i32::from(window.min.0);
                let max = i32::from(window.max.0);
                let mut anchor = i32::from(previous_lag.0);
                if anchor - min > 5 {
                    anchor = min + 5;
                }
                if max - anchor > 4 {
                    anchor = max - 4;
                }
                let uplag = 3 * t0 + frac;
                let low = 3 * (anchor - 2);
                if low >= uplag {
                    t0 - anchor + 5
                } else if 3 * (anchor + 1) > uplag {
                    uplag - low + 3
                } else {
                    t0 - anchor + 11
                }
            }
        }
    }

    /// `Enc_lag6`'s index, likewise derived rather than transcribed.
    fn encode_lag_1_6(lag: PitchLag, min: Option<i32>) -> i32 {
        let t0 = i32::from(lag.integer.0);
        let frac = i32::from(lag.frac.0);
        min.map_or_else(
            || {
                if t0 <= 94 {
                    6 * t0 - 105 + frac
                } else {
                    t0 + 368
                }
            },
            |min| 6 * (t0 - min) + 3 + frac,
        )
    }

    fn window(min: i16, max: i16) -> LagWindow {
        LagWindow {
            min: Word16(min),
            max: Word16(max),
        }
    }

    #[test]
    fn one_third_lag_decoding_is_bit_exact_against_ts26073() {
        let rows = vectors::rows("lag3");
        assert_eq!(rows.len() % 2, 0, "lag3 rows come in case/out pairs");

        let mut ctx = DspContext::default();
        let mut cases = 0;
        let mut compared = 0;

        for pair in rows.chunks_exact(2) {
            assert_eq!(pair[0].label, "case");
            assert_eq!(pair[1].label, "out");
            let c = pair[0].i16s();
            let (four_bit, delta, previous, min, max, count) =
                (c[0] != 0, c[1] != 0, c[2], c[3], c[4], c[5]);
            let out = pair[1].i16s();
            let count = usize::try_from(count).expect("index count");
            assert_eq!(out.len(), 2 * count, "case {cases}: out length");

            let win = window(min, max);
            let coding = if four_bit {
                DeltaCoding::Anchored {
                    previous_lag: Word16(previous),
                }
            } else {
                DeltaCoding::Uniform
            };

            for index in 0..count {
                let i = i16::try_from(index).expect("index fits");
                let got = if delta {
                    delta_lag_1_3(&mut ctx, Word16(i), win, coding)
                } else {
                    absolute_lag(&mut ctx, Word16(i), LagResolution::OneThird)
                };
                assert_eq!(
                    (got.integer.0, got.frac.0),
                    (out[2 * index], out[2 * index + 1]),
                    "four_bit={four_bit} delta={delta} previous={previous} index={index}"
                );
                compared += 1;
            }
            cases += 1;
        }

        assert_eq!(cases, 20, "lag3 sweeps 20 parameter sets");
        assert_eq!(compared, 2960, "lag3 covers 2960 indices");
    }

    #[test]
    fn one_sixth_lag_decoding_is_bit_exact_against_ts26073() {
        let rows = vectors::rows("lag6");
        assert_eq!(rows.len() % 2, 0, "lag6 rows come in case/out pairs");

        let mut ctx = DspContext::default();
        let mut cases = 0;
        let mut compared = 0;

        for pair in rows.chunks_exact(2) {
            assert_eq!(pair[0].label, "case");
            assert_eq!(pair[1].label, "out");
            let c = pair[0].i16s();
            let (delta, count) = (c[0] != 0, usize::try_from(c[1]).expect("index count"));
            let out = pair[1].i16s();
            assert_eq!(out.len(), 2 * count, "case {cases}: out length");

            for index in 0..count {
                let i = i16::try_from(index).expect("index fits");
                // The oracle seeds the in/out lag to 60 before each call,
                // because the delta path reads it.
                let got = if delta {
                    delta_lag_1_6(&mut ctx, Word16(i), Word16(60))
                } else {
                    absolute_lag(&mut ctx, Word16(i), LagResolution::OneSixth)
                };
                assert_eq!(
                    (got.integer.0, got.frac.0),
                    (out[2 * index], out[2 * index + 1]),
                    "delta={delta} index={index}"
                );
                compared += 1;
            }
            cases += 1;
        }

        assert_eq!(cases, 2, "lag6 sweeps the absolute and delta paths");
        assert_eq!(compared, 576, "lag6 covers 576 indices");
    }

    fn check_predlt(section: &str, resolution: LagResolution, cases_want: usize) {
        let rows = vectors::rows(section);
        assert_eq!(rows.len() % 2, 0, "{section} rows come in case/out pairs");

        let mut ctx = DspContext::default();
        let mut cases = 0;
        let mut compared = 0;

        for pair in rows.chunks_exact(2) {
            assert_eq!(pair[0].label, "case");
            assert_eq!(pair[1].label, "out");
            let c = pair[0].i16s();
            let (t0, frac, seed) = (c[0], c[1], c[2]);
            let want = pair[1].i16s();
            assert_eq!(want.len(), L_SUBFR, "{section}: 40 output samples");

            let mut exc = Excitation::new();
            // The oracle fills the whole buffer from this seed before calling,
            // history and current subframe alike.
            let noise = vectors::noise(seed, EXC_LEN, 3);
            exc.all_mut().copy_from_slice(&noise);

            exc.predict(
                &mut ctx,
                PitchLag {
                    integer: Word16(t0),
                    frac: Word16(frac),
                    resolution,
                },
            );

            for (j, (&got, &w)) in exc.subframe().iter().zip(want.iter()).enumerate() {
                assert_eq!(got.0, w, "{section}: T0={t0} frac={frac} sample {j}");
                compared += 1;
            }
            cases += 1;
        }

        assert_eq!(cases, cases_want, "{section} case count");
        assert_eq!(compared, cases_want * L_SUBFR, "{section} sample count");
    }

    #[test]
    fn one_third_adaptive_codebook_is_bit_exact_against_ts26073() {
        check_predlt("predlt3", LagResolution::OneThird, 24);
    }

    #[test]
    fn one_sixth_adaptive_codebook_is_bit_exact_against_ts26073() {
        check_predlt("predlt6", LagResolution::OneSixth, 48);
    }

    #[test]
    fn every_index_round_trips_through_the_encoders_mapping() {
        // The stage vectors prove agreement with `Dec_lag3`/`Dec_lag6`. This
        // proves agreement with the *inverse* mapping the encoder uses, which
        // no shared assumption between oracle and decoder can produce.
        let mut ctx = DspContext::default();
        let mut checked = 0;

        for index in 0..256i16 {
            let lag = absolute_lag(&mut ctx, Word16(index), LagResolution::OneThird);
            assert_eq!(
                encode_lag_1_3(lag, window(0, 0), None),
                i32::from(index),
                "absolute 1/3 index {index}"
            );
            checked += 1;
        }

        for index in 0..512i16 {
            let lag = absolute_lag(&mut ctx, Word16(index), LagResolution::OneSixth);
            assert_eq!(
                encode_lag_1_6(lag, None),
                i32::from(index),
                "absolute 1/6 index {index}"
            );
            checked += 1;
        }

        for previous in [PIT_MIN, 40, 90, PIT_MAX] {
            for mode_index in [4u8, MODE_MR795, 6] {
                let win = delta_window(&mut ctx, mode_index, Word16(previous));
                for index in 0..64i16 {
                    let lag = delta_lag_1_3(&mut ctx, Word16(index), win, DeltaCoding::Uniform);
                    assert_eq!(
                        encode_lag_1_3(lag, win, Some(DeltaCoding::Uniform)),
                        i32::from(index),
                        "uniform delta mode {mode_index} previous {previous} index {index}"
                    );
                    checked += 1;
                }
            }

            let win = delta_window(&mut ctx, 0, Word16(previous));
            let coding = DeltaCoding::Anchored {
                previous_lag: Word16(previous),
            };
            for index in 0..16i16 {
                let lag = delta_lag_1_3(&mut ctx, Word16(index), win, coding);
                assert_eq!(
                    encode_lag_1_3(lag, win, Some(coding)),
                    i32::from(index),
                    "anchored delta previous {previous} index {index}"
                );
                checked += 1;
            }

            for index in 0..64i16 {
                let lag = delta_lag_1_6(&mut ctx, Word16(index), Word16(previous));
                let min = i32::from(
                    delta_lag_1_6(&mut ctx, Word16(3), Word16(previous))
                        .integer
                        .0,
                );
                assert_eq!(
                    encode_lag_1_6(lag, Some(min)),
                    i32::from(index),
                    "delta 1/6 previous {previous} index {index}"
                );
                checked += 1;
            }
        }

        assert_eq!(checked, 256 + 512 + 4 * (3 * 64 + 16 + 64));
    }

    #[test]
    fn lag_decoding_is_strictly_monotone_in_the_index() {
        // Every code space is an ordered ramp of lags. A decoder that mixed up
        // the branch boundaries or the fraction would still produce lags in
        // range but would fold the ramp back on itself.
        let mut ctx = DspContext::default();

        let mut previous = i32::MIN;
        for index in 0..256i16 {
            let lag = absolute_lag(&mut ctx, Word16(index), LagResolution::OneThird);
            assert!(lag.units() > previous, "absolute 1/3 index {index}");
            previous = lag.units();
        }

        let mut previous = i32::MIN;
        for index in 0..512i16 {
            let lag = absolute_lag(&mut ctx, Word16(index), LagResolution::OneSixth);
            assert!(lag.units() > previous, "absolute 1/6 index {index}");
            previous = lag.units();
        }

        let win = delta_window(&mut ctx, 0, Word16(60));
        let coding = DeltaCoding::Anchored {
            previous_lag: Word16(60),
        };
        let mut previous = i32::MIN;
        for index in 0..16i16 {
            let lag = delta_lag_1_3(&mut ctx, Word16(index), win, coding);
            assert!(lag.units() > previous, "anchored index {index}");
            previous = lag.units();
        }
    }

    #[test]
    fn the_four_bit_codes_span_exactly_the_search_window() {
        // Sixteen codes over a ten-sample window: the outer codes step whole
        // samples so the ends are reachable, the middle eight resolve thirds.
        // If the two anchor clamps were dropped the span would drift with the
        // previous lag instead of pinning to the window.
        let mut ctx = DspContext::default();
        for previous in 1..=PIT_MAX {
            let win = delta_window(&mut ctx, 0, Word16(previous));
            let coding = delta_coding(0, Word16(previous));
            let lowest = delta_lag_1_3(&mut ctx, Word16(0), win, coding);
            let highest = delta_lag_1_3(&mut ctx, Word16(15), win, coding);
            assert_eq!(lowest.integer, win.min, "previous {previous}");
            assert_eq!(lowest.frac.0, 0);
            assert_eq!(highest.integer, win.max, "previous {previous}");
            assert_eq!(highest.frac.0, 0);
            // Code 9 sits on the anchor, which these windows always place five
            // samples above the bottom.
            let middle = delta_lag_1_3(&mut ctx, Word16(9), win, coding);
            assert_eq!(middle.integer.0, win.min.0 + 5, "previous {previous}");
            assert_eq!(middle.frac.0, 0);
        }
    }

    #[test]
    fn the_delta_window_keeps_its_width_inside_the_lag_range() {
        let mut ctx = DspContext::default();
        for mode_index in 0..8u8 {
            if mode_index == 7 {
                continue; // 12.2 kbit/s derives its own window.
            }
            let width = if mode_index == MODE_MR795 { 19 } else { 9 };
            for previous in 1..=200i16 {
                let win = delta_window(&mut ctx, mode_index, Word16(previous));
                assert_eq!(
                    win.max.0 - win.min.0,
                    width,
                    "mode {mode_index} previous {previous}"
                );
                assert!(
                    win.min.0 >= PIT_MIN,
                    "mode {mode_index} previous {previous}"
                );
                assert!(
                    win.max.0 <= PIT_MAX,
                    "mode {mode_index} previous {previous}"
                );
            }
        }
    }

    #[test]
    fn only_the_two_lowest_rates_delta_code_the_third_subframe() {
        for mode_index in 0..8u8 {
            assert!(!is_delta_coded(mode_index, 0));
            assert!(is_delta_coded(mode_index, 1));
            assert!(is_delta_coded(mode_index, 3));
            assert_eq!(
                is_delta_coded(mode_index, 2),
                mode_index <= 1,
                "mode {mode_index} subframe 2"
            );
        }
    }

    /// Interpolate a subframe from a frozen copy of the buffer.
    ///
    /// This is the plausible-looking wrong implementation: it never sees its
    /// own output, so it cannot reproduce a period shorter than a subframe.
    /// Written independently, in wide arithmetic, so that a shared mistake with
    /// [`Excitation::predict`] would have to be made twice.
    fn snapshot_filter(
        frozen: &[Word16],
        t0: i16,
        frac: i16,
        resolution: LagResolution,
    ) -> Vec<i16> {
        let mut phase = i32::from(-frac);
        if resolution == LagResolution::OneThird {
            phase *= 2;
        }
        // A negative fraction is a fraction of the *previous* sample, so it is
        // folded into the phase and paid for by stepping the base back one.
        let back = if phase < 0 {
            phase += 6;
            1i32
        } else {
            0
        };
        let base = i32::try_from(HISTORY).expect("fits") - i32::from(t0) - back;
        let tap = |p: i32, k: usize| -> i64 {
            i64::from(INTER_6_PRED[usize::try_from(p).expect("phase") + k * 6])
        };

        (0..L_SUBFR)
            .map(|j| {
                let centre = base + i32::try_from(j).expect("fits");
                let mut acc = 0i64;
                for k in 0..TAPS_PER_SIDE {
                    let lo = usize::try_from(centre - i32::try_from(k).expect("fits"))
                        .expect("in range");
                    let hi = usize::try_from(centre + 1 + i32::try_from(k).expect("fits"))
                        .expect("in range");
                    acc += 2 * i64::from(frozen[lo].0) * tap(phase, k);
                    acc += 2 * i64::from(frozen[hi].0) * tap(6 - phase, k);
                }
                let acc = acc.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
                i16::try_from(((acc + 0x8000) >> 16).clamp(-32768, 32767)).expect("fits")
            })
            .collect()
    }

    #[test]
    fn the_short_lag_recursion_is_observable() {
        // The whole reason this module owns one buffer. Against a filter that
        // reads a frozen snapshot of the history:
        //
        // - at or above a lag of 50 the read span cannot reach the output, so
        //   the two must agree exactly;
        // - at or below 39 the lag is shorter than the subframe, so every
        //   output from sample T0 onward is a filtered copy of an earlier
        //   output and the two must differ.
        //
        // Lags of 40 to 49 are deliberately left out: there the overlap is one
        // or two outermost taps, and whether it changes anything depends on the
        // phase — the last tap is zero at phase 0.
        let mut ctx = DspContext::default();
        let noise = vectors::noise(4321, EXC_LEN, 3);

        for &(t0, frac, expect_same) in &[
            (20i16, 0i16, false),
            (20, -1, false),
            (25, 1, false),
            (39, 0, false),
            (50, 0, true),
            (50, 1, true),
            (90, -1, true),
            (PIT_MAX, 0, true),
        ] {
            let lag = PitchLag {
                integer: Word16(t0),
                frac: Word16(frac),
                resolution: LagResolution::OneThird,
            };

            let mut inplace = Excitation::new();
            inplace.all_mut().copy_from_slice(&noise);
            inplace.predict(&mut ctx, lag);

            let frozen = snapshot_filter(&noise, t0, frac, LagResolution::OneThird);
            let got: Vec<i16> = inplace.subframe().iter().map(|s| s.0).collect();

            assert_eq!(
                got == frozen,
                expect_same,
                "T0={t0} frac={frac}: in-place and snapshot filtering agreed={}",
                got == frozen
            );
        }
    }

    #[test]
    fn a_constant_history_comes_back_scaled_by_the_filters_dc_gain() {
        // The 20 taps at phase zero sum to 32723 in Q15, so a flat excitation
        // comes back very slightly attenuated — and identically for all 40
        // samples, which would not survive an off-by-one in the tap indexing.
        let mut ctx = DspContext::default();
        let mut exc = Excitation::new();
        for s in exc.all_mut() {
            *s = Word16(1000);
        }
        // A lag of 143 keeps the whole read span inside the history, so no
        // output feeds back and every sample sees the same input.
        exc.predict(
            &mut ctx,
            PitchLag::integral(Word16(PIT_MAX), LagResolution::OneThird),
        );

        let dc_gain: i32 = (0..TAPS_PER_SIDE)
            .map(|i| {
                i32::from(INTER_6_PRED[i * UP_SAMP_MAX])
                    + i32::from(INTER_6_PRED[6 + i * UP_SAMP_MAX])
            })
            .sum();
        assert_eq!(dc_gain, 32723);
        let want = i16::try_from((2 * 1000 * dc_gain + 0x8000) >> 16).expect("fits");

        for (j, s) in exc.subframe().iter().enumerate() {
            assert_eq!(s.0, want, "sample {j}");
        }
    }

    #[test]
    fn this_is_not_the_encoders_interpolation_filter() {
        // The reference has two tables called `inter_6`: this 61-entry one,
        // file-local to `pred_lt.c`, and a 25-entry one in `inter_36.tab` that
        // belongs to the encoder's closed-loop pitch search. Using the search
        // filter here would low-pass the adaptive codebook with the wrong
        // response and still produce speech.
        assert_eq!(FIR_SIZE, 61);
        assert_eq!(INTER_6_SEARCH.len(), 25);
        assert_ne!(INTER_6_PRED[0], INTER_6_SEARCH[0]);
        assert_eq!(INTER_6_PRED[0], 29443);
        assert_eq!(INTER_6_SEARCH[0], 29519);
    }

    #[test]
    fn the_buffer_spans_the_whole_reachable_history() {
        assert_eq!(
            HISTORY,
            usize::try_from(PIT_MAX).expect("positive") + L_INTERPOL
        );
        assert_eq!(EXC_LEN, HISTORY + L_SUBFR);

        // The worst case the standard allows: 7.95 kbit/s delta index 63 with
        // the window pinned to the top of the range. It decodes to a lag of 144
        // with a positive fraction, which folds the read one sample further
        // back and touches the very first slot of the buffer.
        let mut ctx = DspContext::default();
        let win = delta_window(&mut ctx, MODE_MR795, Word16(PIT_MAX));
        let lag = delta_lag_1_3(&mut ctx, Word16(63), win, DeltaCoding::Uniform);
        assert_eq!((lag.integer.0, lag.frac.0), (144, 1));

        // It must filter rather than panic.
        let mut exc = Excitation::new();
        exc.all_mut()
            .copy_from_slice(&vectors::noise(77, EXC_LEN, 3));
        exc.predict(&mut ctx, lag);
    }

    #[test]
    fn advancing_moves_the_subframe_into_the_history() {
        let mut exc = Excitation::new();
        for (i, s) in exc.all_mut().iter_mut().enumerate() {
            *s = Word16(i16::try_from(i).expect("fits"));
        }
        let last = exc.subframe().to_vec();
        exc.advance();
        // The subframe now sits at the very end of the history.
        assert_eq!(&exc.all()[HISTORY - L_SUBFR..HISTORY], last.as_slice());
    }

    #[test]
    fn resetting_clears_the_history() {
        let mut exc = Excitation::new();
        for s in exc.all_mut() {
            *s = Word16(-5000);
        }
        exc.reset();
        assert!(exc.all().iter().all(|s| s.0 == 0));
    }
}
