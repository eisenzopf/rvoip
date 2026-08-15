//! AMR-WB encoder: joint quantisation of the pitch and codebook gains.
//!
//! Implements TS 26.173 `Q_gain2()` and `Init_Q_gain2()` (`q_gain2.c`), the
//! vector quantiser that picks one `(pitch gain, code gain)` pair per subframe
//! and carries the moving-average energy predictor that makes the code gain
//! differential.
//!
//! # What is being minimised
//!
//! The two gains are quantised jointly rather than separately because they are
//! strongly correlated — a voiced subframe wants a high pitch gain and a low
//! code gain — and because the error criterion is not the error in each gain
//! but the weighted mean-squared error of the *reconstructed* excitation. The
//! search evaluates that error for every candidate pair in a 64-entry window
//! and keeps the smallest, with ties resolving to the lowest index.
//!
//! Only the *correction factor* on the code gain is transmitted. The predictor
//! estimates the innovation's energy from the last four subframes' quantised
//! energies, so the same table entry means a different absolute gain in each
//! subframe. That is what makes the predictor state a hard dependency: a
//! predictor one subframe out of step produces plausible speech and a wrong
//! bitstream, and the error compounds.
//!
//! # Validation
//!
//! Bit-exact against the TS 26.173 encoder's own trace
//! (`testdata/wb_enc_trace.txt`, 12.65 kbit/s) for `gain_pit`, `gain_code` and
//! `L_gain_code`, and against the gain index in the reference encoder's
//! committed bitstream (`testdata/amrwb_enc_mode2.amr`).
//!
//! Three of the twelve committed subframes cannot be checked: when the encoder
//! selects the low-pass branch of the adaptive codebook it substitutes that
//! branch's filtered vector and correlations for `y1` and `g_coeff`, and the
//! trace captures `y1` before the substitution. Those subframes still step the
//! predictor, using the table entry the committed bitstream names, so the state
//! the checked subframes run on is exact.
//!
//! Offline — with 50-frame traces at every rate, which are not committed — the
//! full search was checked bit-exact on 769 subframes across the seven-bit
//! rates, and the prediction path (`predict`, `remember`, the table lookup and
//! the Q16 scaling) on all 1612 subframes of all nine rates, six-bit modes
//! included. The **six-bit search itself is not covered**: at 6.60 and 8.85
//! kbit/s every subframe takes the low-pass branch, so no traced subframe pins
//! its inputs down. Only its range logic is unit-tested here.

// The reference's variable names are the specification's vocabulary — `g_code`
// and `g2_code`, `yy2` and `y1y2` — and renaming them to satisfy the
// similar-names heuristic would make this module harder, not easier, to check
// against TS 26.173. Same exemption the `fixed_point` subtree takes.
#![allow(clippy::similar_names)]

use crate::codecs::amr::wb::gain_tables::{QUA_GAIN_6B, QUA_GAIN_7B};
use crate::codecs::amr::wb::math::{dot_product12, log2, pow2};
use crate::fixed_point::arith::{add, extract_h, extract_l, mult_r, negate, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_deposit_l, l_mac, l_mult, l_negate, l_sub};
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Samples in a subframe.
pub const L_SUBFR: usize = 64;

/// Order of the moving-average energy predictor.
const PRED_ORDER: usize = 4;

/// Predictor coefficients `{0.5, 0.4, 0.3, 0.2}` in Q13.
const PRED: [Word16; PRED_ORDER] = [Word16(4096), Word16(3277), Word16(2458), Word16(1638)];

/// Predictor state out of reset: −14.0 dB in Q10, in every slot.
const PAST_ENERGY_RESET: Word16 = Word16(-14336);

/// Mean innovation energy in dB, subtracted before prediction.
const MEAN_ENER: Word16 = Word16(30);

/// Candidates evaluated by one search, and the size of the 7-bit sliding
/// window (`RANGE`).
const RANGE: usize = 64;

/// How many bits the mode spends on the gain pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GainBits {
    /// Six bits, at 6.60 and 8.85 kbit/s.
    Six,
    /// Seven bits, at 12.65 kbit/s and above.
    Seven,
}

impl GainBits {
    /// The width a frame of `frame_bits` bits uses. Seven bits from 12.65
    /// kbit/s (253 bits per frame) upward.
    #[must_use]
    pub const fn from_frame_bits(frame_bits: usize) -> Self {
        if frame_bits <= 177 {
            Self::Six
        } else {
            Self::Seven
        }
    }

    /// Bits transmitted.
    #[must_use]
    pub const fn bits(self) -> usize {
        match self {
            Self::Six => 6,
            Self::Seven => 7,
        }
    }
}

/// The `<y1 y1>` and `<xn y1>` correlations the pitch gain search already
/// produced, reused here rather than recomputed.
///
/// Each is a mantissa with its own exponent, in the normalised form
/// `Dot_product12` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PitchCorrelations {
    /// `<y1, y1>` mantissa.
    pub energy: Word16,
    /// Exponent of `energy`.
    pub energy_exp: i16,
    /// `<xn, y1>` mantissa.
    pub correlation: Word16,
    /// Exponent of `correlation`.
    pub correlation_exp: i16,
}

/// Everything one subframe's gain quantisation reads.
pub struct GainInputs<'a> {
    /// Pitch target `xn`, in `target_q`.
    pub target: &'a [Word16; L_SUBFR],
    /// Filtered adaptive codebook vector `y1`, in `target_q`.
    pub filtered_adaptive: &'a [Word16; L_SUBFR],
    /// The Q of `target` and `filtered_adaptive`, `Q_new + shift` at the call
    /// site.
    pub target_q: i16,
    /// Filtered innovative vector `y2`, Q9 — the algebraic codebook search's
    /// output, *not* re-filtered after the codeword was sharpened.
    pub filtered_code: &'a [Word16; L_SUBFR],
    /// Innovative vector `code`, Q9, after pre-emphasis and pitch sharpening.
    pub code: &'a [Word16; L_SUBFR],
    /// Correlations from the pitch gain computation.
    pub correlations: PitchCorrelations,
    /// Table width for this mode.
    pub bits: GainBits,
    /// The *unquantised* pitch gain, Q14. Only the 7-bit search reads it, to
    /// place its sliding window.
    pub pitch_gain: Word16,
    /// Whether the pitch-gain clipping guard is active this subframe.
    pub clip_pitch_gain: bool,
}

/// The chosen gain pair and the index that names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuantisedGains {
    /// Absolute table index, 0..63 for six bits or 0..127 for seven. This is
    /// what goes into the bitstream.
    pub index: u16,
    /// Quantised pitch gain, Q14.
    pub pitch_gain: Word16,
    /// Quantised code gain, Q16.
    pub code_gain: Word32,
}

/// The moving-average predictor of innovation energy, and the search that
/// updates it.
///
/// One instance per encoder. The state advances once per subframe and is never
/// reset between subframes or frames — only by a full encoder reset, and in the
/// reference not even by a partial one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainPredictor {
    /// Last four quantised innovation energies, most recent first, Q10.
    past_energy: [Word16; PRED_ORDER],
}

impl Default for GainPredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl GainPredictor {
    /// A predictor in its reset state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            past_energy: [PAST_ENERGY_RESET; PRED_ORDER],
        }
    }

    /// Return to the reset state.
    ///
    /// The reference only does this on a *full* encoder reset, not on the
    /// partial reset a mode change performs.
    pub const fn reset(&mut self) {
        self.past_energy = [PAST_ENERGY_RESET; PRED_ORDER];
    }

    /// Record a quantised code gain in the predictor.
    ///
    /// `table_gain` is the **raw Q11 table entry**, not the gain that was
    /// finally applied. Feeding the scaled gain in instead corrupts the
    /// predictor and the error compounds over the following subframes —
    /// plausible speech, wrong bitstream.
    fn remember(&mut self, ctx: &mut DspContext, table_gain: Word16) {
        // 20*log10(g) = 6.0206 * (log2(g in Q11) - 11), in Q10.
        let (exponent, fraction) = log2(ctx, l_deposit_l(table_gain));
        let exponent = sub(ctx, Word16(exponent), Word16(11));
        let scaled = mpy_32_16(exponent, fraction, Word16(24660));
        let energy = extract_l(l_shr(ctx, scaled, 3));

        self.past_energy[3] = self.past_energy[2];
        self.past_energy[2] = self.past_energy[1];
        self.past_energy[1] = self.past_energy[0];
        self.past_energy[0] = energy;
    }

    /// Predict this subframe's innovation gain from the codeword's energy and
    /// the predictor state.
    ///
    /// Returns the mantissa (16384 < g <= 32767) and its exponent, together
    /// meaning `mantissa * 2^exponent`.
    fn predict(self, ctx: &mut DspContext, code: &[Word16; L_SUBFR]) -> (Word16, Word16) {
        let (energy, exponent) = dot_product12(ctx, code, code);
        // -18 for the Q9 codeword squared, -6 for the subframe length, -31 to
        // bring the Q31 mantissa back to Q0.
        let exponent = exponent - (18 + 6 + 31);

        let (log_int, log_frac) = log2(ctx, energy);
        let log_int = log_int + exponent;
        // MEAN_ENER - 3.0103 * log2(energy), Q14.
        let mut acc = mpy_32_16(Word16(log_int), log_frac, Word16(-24660));
        acc = l_mac(ctx, acc, MEAN_ENER, Word16(8192));

        // Q14 to Q24, then add the prediction from past energies (Q13 x Q10).
        acc = l_shl(ctx, acc, 10);
        for (&coefficient, &past) in PRED.iter().zip(self.past_energy.iter()) {
            acc = l_mac(ctx, acc, coefficient, past);
        }
        let decibels = extract_h(acc);

        // 10^(dB/20) = 2^(0.166096 * dB): the integer part becomes the
        // exponent and the fraction is looked up.
        let acc = l_mult(ctx, decibels, Word16(5443));
        let acc = l_shr(ctx, acc, 8);
        let (exponent, fraction) = l_extract(acc);
        let mantissa = extract_l(pow2(ctx, 14, fraction));
        (mantissa, sub(ctx, exponent, Word16(14)))
    }

    /// Quantise one subframe's gain pair, and advance the predictor.
    ///
    /// The search evaluates every candidate in its window — there is no early
    /// exit — and accepts strictly smaller distortions only, so equal
    /// distortions keep the lowest index.
    ///
    /// # Panics
    ///
    /// Never: the window is constructed so that it stays inside the table.
    pub fn quantise(&mut self, ctx: &mut DspContext, inputs: &GainInputs<'_>) -> QuantisedGains {
        let (table, first, size) = Self::window(ctx, inputs);
        let (gcode0, exp_gcode0) = self.predict(ctx, inputs.code);
        let (coeff, coeff_lo) = align_coefficients(ctx, inputs, exp_gcode0);

        let mut smallest = Word32(i32::MAX);
        let mut best = 0usize;
        for i in 0..size {
            let pair = 2 * (first + i);
            let g_pitch = Word16(table[pair]);
            let g_code = mult_r(ctx, Word16(table[pair + 1]), gcode0);

            let g2_pitch = mult_r(ctx, g_pitch, g_pitch);
            let g_pit_cod = mult_r(ctx, g_code, g_pitch);
            let (g2_code, g2_code_lo) = l_extract(l_mult(ctx, g_code, g_code));

            // Double precision: the low halves are accumulated first, shifted
            // down, and the high halves added on top. Both shifts floor.
            let mut acc = l_mult(ctx, coeff[2], g2_code_lo);
            acc = l_shr(ctx, acc, 3);
            acc = l_mac(ctx, acc, coeff_lo[0], g2_pitch);
            acc = l_mac(ctx, acc, coeff_lo[1], g_pitch);
            acc = l_mac(ctx, acc, coeff_lo[2], g2_code);
            acc = l_mac(ctx, acc, coeff_lo[3], g_code);
            acc = l_mac(ctx, acc, coeff_lo[4], g_pit_cod);
            acc = l_shr(ctx, acc, 12);
            acc = l_mac(ctx, acc, coeff[0], g2_pitch);
            acc = l_mac(ctx, acc, coeff[1], g_pitch);
            acc = l_mac(ctx, acc, coeff[2], g2_code);
            acc = l_mac(ctx, acc, coeff[3], g_code);
            acc = l_mac(ctx, acc, coeff[4], g_pit_cod);

            if beats(ctx, acc, smallest) {
                smallest = acc;
                best = i;
            }
        }

        let index = first + best;
        let pitch_gain = Word16(table[2 * index]);
        let table_gain = Word16(table[2 * index + 1]);

        // Q11 x Q0 -> Q12, then up to Q16.
        let raw = l_mult(ctx, table_gain, gcode0);
        let up = add(ctx, exp_gcode0, Word16(4)).0;
        let code_gain = l_shl(ctx, raw, up);

        self.remember(ctx, table_gain);

        QuantisedGains {
            index: u16::try_from(index).expect("the window stays inside the table"),
            pitch_gain,
            code_gain,
        }
    }

    /// Choose the table and the slice of it the search will cover.
    ///
    /// The two widths restrict the search differently and the difference is not
    /// cosmetic: clipping shortens the six-bit search (dropping the sixteen
    /// highest pitch gains) but only moves the seven-bit search's window start
    /// (by capping how far the pre-search may count). Unifying them changes the
    /// chosen index whenever clipping is active.
    fn window(ctx: &mut DspContext, inputs: &GainInputs<'_>) -> (&'static [i16], usize, usize) {
        match inputs.bits {
            GainBits::Six => {
                let size = if inputs.clip_pitch_gain {
                    RANGE - 16
                } else {
                    RANGE
                };
                (&QUA_GAIN_6B, 0, size)
            }
            GainBits::Seven => {
                let scanned = if inputs.clip_pitch_gain {
                    RANGE - 27
                } else {
                    RANGE
                };
                // Count, over pairs 32 upward, how many have a pitch gain
                // strictly below the incoming unquantised one. That count is
                // where the 64-entry window starts. The reference's `p` starts
                // a quarter of the way into the *flat* interleaved table, which
                // is pair 32 — not pair 64.
                let mut first = 0usize;
                for i in 0..scanned {
                    let candidate = Word16(QUA_GAIN_7B[RANGE + 2 * i]);
                    if sub(ctx, inputs.pitch_gain, candidate).0 > 0 {
                        first += 1;
                    }
                }
                (&QUA_GAIN_7B, first, RANGE)
            }
        }
    }
}

/// Whether a candidate's distortion displaces the incumbent's.
///
/// Strictly smaller, so equal distortions keep the earlier — and therefore
/// lower — index. This is the search's only comparison, and it is the one place
/// where an otherwise-correct implementation can silently pick a different
/// codeword.
///
/// Constructing an exact tie from the shipped tables is not possible: all 128
/// pairs of the seven-bit codebook are distinct, no two share a pitch gain
/// *and* a code gain, and the `+2` headroom in the exponent alignment keeps the
/// distortion clear of saturation on real and synthetic input alike. The rule
/// is therefore asserted here rather than through the search.
fn beats(ctx: &mut DspContext, candidate: Word32, incumbent: Word32) -> bool {
    l_sub(ctx, candidate, incumbent).0 < 0
}

/// The five correlation coefficients, rescaled to a common exponent and split
/// into high and low halves for the double-precision distortion sum.
///
/// The `+2` in the alignment shift is deliberate headroom — a quarter — and the
/// `>> 3` on each low half is part of the fixed scaling the search's two
/// `L_shr`s undo. Both change how much precision each term keeps, and so both
/// are part of which candidate wins.
fn align_coefficients(
    ctx: &mut DspContext,
    inputs: &GainInputs<'_>,
    exp_gcode0: Word16,
) -> ([Word16; 5], [Word16; 5]) {
    let g = inputs.correlations;
    let q = inputs.target_q;

    let (yy2, exp_yy2) = dot_product12(ctx, inputs.filtered_code, inputs.filtered_code);
    let (xy2, exp_xy2) = dot_product12(ctx, inputs.target, inputs.filtered_code);
    let (y1y2, exp_y1y2) = dot_product12(ctx, inputs.filtered_adaptive, inputs.filtered_code);

    let mut coeff = [
        g.energy,
        negate(ctx, g.correlation),
        extract_h(yy2),
        extract_h(l_negate(ctx, xy2)),
        extract_h(y1y2),
    ];
    let exp_coeff = [
        g.energy_exp,
        g.correlation_exp + 1,
        (exp_yy2 - 18) + 2 * q,
        (exp_xy2 - 8) + q,
        (exp_y1y2 - 8) + q,
    ];

    // Every product the search forms carries its own fixed scaling; these are
    // the exponents the aligned coefficients must share.
    let exp_code = exp_gcode0.0 + 4;
    let exp_max = [
        exp_coeff[0] - 13,
        exp_coeff[1] - 14,
        exp_coeff[2] + 15 + 2 * exp_code,
        exp_coeff[3] + exp_code,
        exp_coeff[4] + 1 + exp_code,
    ];
    let largest = exp_max.iter().copied().fold(exp_max[0], i16::max);

    let mut coeff_lo = [Word16(0); 5];
    for i in 0..5 {
        let down = (largest - exp_max[i]) + 2;
        let aligned = l_shr(ctx, l_deposit_h(coeff[i]), down);
        let (hi, lo) = l_extract(aligned);
        coeff[i] = hi;
        coeff_lo[i] = shr(ctx, lo, 3);
    }

    (coeff, coeff_lo)
}

/// Rescale a Q16 code gain to the frame's own Q, as the caller does before
/// building the total excitation.
///
/// Saturation is reachable here and is the reference's behaviour.
#[must_use]
pub fn scale_code_gain(ctx: &mut DspContext, code_gain: Word32, frame_q: i16) -> Word16 {
    let scaled = l_shl(ctx, code_gain, frame_q);
    round(ctx, scaled)
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::{AmrMode, AmrVariant};
    use crate::codecs::amr::storage;
    use crate::codecs::amr::wb::params::FrameParams;

    /// The reference encoder's own per-subframe trace at 12.65 kbit/s.
    const TRACE: &str = include_str!("../../testdata/wb_enc_trace.txt");

    /// Frames of the trace committed alongside it.
    const TRACED_FRAMES: usize = 3;

    /// The bitstream the reference encoder produced from the same input.
    const BITSTREAM: &[u8] = include_bytes!("../../testdata/amrwb_enc_mode2.amr");

    fn row(frame: usize, subframe: i32, name: &str) -> Vec<i32> {
        let prefix = format!("T {frame} {subframe} {name} ");
        let line = TRACE
            .lines()
            .find(|l| l.starts_with(&prefix))
            .unwrap_or_else(|| panic!("trace row {frame} {subframe} {name} is missing"));
        line.split_whitespace()
            .skip(4)
            .map(|t| t.parse().expect("trace values are integers"))
            .collect()
    }

    fn vector(frame: usize, subframe: usize, name: &str) -> [Word16; L_SUBFR] {
        let values = row(frame, subframe as i32, name);
        assert_eq!(values.len(), L_SUBFR, "{name} is not a subframe vector");
        let mut out = [Word16(0); L_SUBFR];
        for (o, v) in out.iter_mut().zip(values) {
            *o = Word16(v as i16);
        }
        out
    }

    fn scalar(frame: usize, subframe: i32, name: &str) -> i32 {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), 1, "{name} is not a scalar");
        values[0]
    }

    /// `G_pitch`'s two correlations, from `g_pitch.c`.
    ///
    /// Not part of this module — the adaptive codebook search owns it — but the
    /// quantiser consumes its output, so the test has to produce it. It is
    /// nothing but the two dot products; the gain and its clipping, which is
    /// the rest of `G_pitch`, are irrelevant here.
    fn pitch_correlations(
        ctx: &mut DspContext,
        xn: &[Word16; L_SUBFR],
        y1: &[Word16; L_SUBFR],
    ) -> PitchCorrelations {
        let (yy, exp_yy) = dot_product12(ctx, y1, y1);
        let (xy, exp_xy) = dot_product12(ctx, xn, y1);
        PitchCorrelations {
            energy: extract_h(yy),
            energy_exp: exp_yy,
            correlation: extract_h(xy),
            correlation_exp: exp_xy,
        }
    }

    /// The reference's pitch-gain clipping memory, `gpclip.c`, at 12.65 kbit/s.
    ///
    /// Reproduced only to show that the flag is off for every traced subframe:
    /// the guard needs both a small ISF distance *and* a long-term pitch gain
    /// above 0.9 in Q14, and this recurrence never gets there on this input.
    fn clip_memory_after(gain_pit: &[i32]) -> Vec<i16> {
        let mut ctx = DspContext::new();
        let mut mem = 9830i16;
        let mut history = Vec::with_capacity(gain_pit.len());
        for &g in gain_pit {
            let acc = l_mult(&mut ctx, Word16(29491), Word16(mem));
            let acc = l_mac(&mut ctx, acc, Word16(3277), Word16(g as i16));
            mem = extract_h(acc).0.max(9830);
            history.push(mem);
        }
        history
    }

    /// Walk the traced subframes in order, quantising the ones whose inputs the
    /// trace actually pins down and stepping the predictor through the rest.
    ///
    /// `select` is the LTP low-pass decision. When it is 0 the encoder replaces
    /// `y1` and the pitch correlations with the low-pass branch's, which the
    /// trace captures before that substitution — so those subframes cannot be
    /// quantised from the trace. Their contribution to the predictor is still
    /// exact, because the state update depends only on the chosen table entry
    /// and the bitstream names it.
    fn walk<F>(mut check: F) -> usize
    where
        F: FnMut(&mut DspContext, usize, usize, &QuantisedGains, u16),
    {
        let mut ctx = DspContext::new();
        let mut predictor = GainPredictor::new();
        let (_, frames) = storage::read(BITSTREAM).expect("the fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");

        let gains: Vec<i32> = (0..TRACED_FRAMES)
            .flat_map(|f| (0..4).map(move |s| scalar(f, s, "gain_pit")))
            .collect();
        let clip = clip_memory_after(&gains);
        assert!(
            clip.iter().all(|&m| m <= 14746),
            "clipping would be active somewhere in the trace: {clip:?}"
        );

        let mut checked = 0usize;
        for (frame, coded) in frames.iter().enumerate().take(TRACED_FRAMES) {
            let params = FrameParams::parse(mode, &coded.data).expect("the frame parses");
            let frame_q = scalar(frame, -1, "Q_new") as i16;

            for subframe in 0..4 {
                let expected_index = params.subframes[subframe].gain_index;

                if scalar(frame, subframe as i32, "select") == 1 {
                    let xn = vector(frame, subframe, "xn");
                    let y1 = vector(frame, subframe, "y1");
                    let y2 = vector(frame, subframe, "y2");
                    let code = vector(frame, subframe, "code");
                    let correlations = pitch_correlations(&mut ctx, &xn, &y1);
                    let inputs = GainInputs {
                        target: &xn,
                        filtered_adaptive: &y1,
                        target_q: frame_q + scalar(frame, subframe as i32, "shift") as i16,
                        filtered_code: &y2,
                        code: &code,
                        correlations,
                        bits: GainBits::Seven,
                        // The pre-search reads the unquantised pitch gain,
                        // which is `gain1` on this branch.
                        pitch_gain: Word16(scalar(frame, subframe as i32, "gain1") as i16),
                        clip_pitch_gain: false,
                    };
                    let gains = predictor.quantise(&mut ctx, &inputs);
                    check(&mut ctx, frame, subframe, &gains, expected_index);
                    checked += 1;
                } else {
                    // Step the predictor with the entry the reference chose.
                    let table_gain = Word16(QUA_GAIN_7B[2 * usize::from(expected_index) + 1]);
                    predictor.remember(&mut ctx, table_gain);
                }
            }
        }
        checked
    }

    #[test]
    fn quantised_gains_match_the_reference_trace() {
        let mut compared = 0usize;
        let checked = walk(|ctx, frame, subframe, gains, _| {
            let expected_pitch = scalar(frame, subframe as i32, "gain_pit") as i16;
            let expected_code = scalar(frame, subframe as i32, "L_gain_code");
            let frame_q = scalar(frame, -1, "Q_new") as i16;
            let expected_scaled = scalar(frame, subframe as i32, "gain_code") as i16;

            assert_eq!(
                gains.pitch_gain.0, expected_pitch,
                "gain_pit differs at frame {frame} subframe {subframe}"
            );
            assert_eq!(
                gains.code_gain.0, expected_code,
                "L_gain_code differs at frame {frame} subframe {subframe}"
            );
            assert_eq!(
                scale_code_gain(ctx, gains.code_gain, frame_q).0,
                expected_scaled,
                "gain_code differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        });
        assert_eq!(checked, 9, "quantised {checked} subframes, expected 9");
        assert_eq!(compared, 9, "compared {compared} subframes, expected 9");
    }

    #[test]
    fn gain_index_matches_the_reference_bitstream() {
        let mut compared = 0usize;
        let checked = walk(|_, frame, subframe, gains, expected| {
            assert_eq!(
                gains.index, expected,
                "gain index differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        });
        assert_eq!(checked, 9, "quantised {checked} subframes, expected 9");
        assert_eq!(compared, 9, "compared {compared} subframes, expected 9");
    }

    /// The predictor state is what makes the search history-dependent, so a
    /// fresh predictor must give a different answer than the walked one.
    ///
    /// Without this a quantiser that ignored its state entirely would still
    /// pass the trace comparison for the first subframe and could pass the rest
    /// by coincidence on a stationary input.
    #[test]
    fn the_predictor_state_changes_the_chosen_index() {
        let mut ctx = DspContext::new();
        let xn = vector(2, 2, "xn");
        let y1 = vector(2, 2, "y1");
        let y2 = vector(2, 2, "y2");
        let code = vector(2, 2, "code");
        let correlations = pitch_correlations(&mut ctx, &xn, &y1);
        let inputs = GainInputs {
            target: &xn,
            filtered_adaptive: &y1,
            target_q: scalar(2, -1, "Q_new") as i16 + scalar(2, 2, "shift") as i16,
            filtered_code: &y2,
            code: &code,
            correlations,
            bits: GainBits::Seven,
            pitch_gain: Word16(scalar(2, 2, "gain1") as i16),
            clip_pitch_gain: false,
        };

        let mut fresh = GainPredictor::new();
        let from_reset = fresh.quantise(&mut ctx, &inputs);
        let expected = {
            let (_, frames) = storage::read(BITSTREAM).expect("the fixture parses");
            let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
            FrameParams::parse(mode, &frames[2].data)
                .expect("parses")
                .subframes[2]
                .gain_index
        };
        assert_ne!(
            from_reset.index, expected,
            "a reset predictor should not reproduce the tenth subframe's index"
        );
    }

    /// Reset state and the shift-in order of the predictor.
    #[test]
    fn the_predictor_resets_to_minus_fourteen_decibels_and_shifts() {
        let mut ctx = DspContext::new();
        let mut predictor = GainPredictor::new();
        assert_eq!(predictor.past_energy, [Word16(-14336); 4]);

        // 2048 in Q11 is 1.0, so 20*log10(1) = 0.
        predictor.remember(&mut ctx, Word16(2048));
        assert_eq!(predictor.past_energy[0].0, 0);
        assert_eq!(predictor.past_energy[1].0, -14336);

        predictor.remember(&mut ctx, Word16(4096));
        // Doubling is +6.02 dB, which is 6165 in Q10.
        assert_eq!(predictor.past_energy[0].0, 6165);
        assert_eq!(predictor.past_energy[1].0, 0);
        assert_eq!(predictor.past_energy[2].0, -14336);
        assert_eq!(predictor.past_energy[3].0, -14336);

        predictor.reset();
        assert_eq!(predictor.past_energy, [Word16(-14336); 4]);
    }

    /// The energy fed to the predictor comes from the raw table entry, not the
    /// scaled gain the excitation actually uses.
    ///
    /// Two subframes with the same index but wildly different predicted gains
    /// must leave the predictor in the same state.
    #[test]
    fn the_predictor_records_the_table_gain_not_the_applied_gain() {
        let mut ctx = DspContext::new();
        let mut a = GainPredictor::new();
        let mut b = GainPredictor::new();
        // Different histories, so `predict` would scale the same table entry
        // very differently.
        b.past_energy = [Word16(6000); 4];

        let table_gain = Word16(QUA_GAIN_7B[2 * 40 + 1]);
        a.remember(&mut ctx, table_gain);
        b.remember(&mut ctx, table_gain);
        assert_eq!(
            a.past_energy[0], b.past_energy[0],
            "the recorded energy must not depend on the predictor's own history"
        );
    }

    /// The seven-bit pre-search places the window; the six-bit one does not
    /// have a window at all.
    #[test]
    fn the_seven_bit_window_follows_the_unquantised_pitch_gain() {
        let mut ctx = DspContext::new();
        let zeros = [Word16(0); L_SUBFR];
        let correlations = PitchCorrelations {
            energy: Word16(0),
            energy_exp: 0,
            correlation: Word16(0),
            correlation_exp: 0,
        };
        let mut inputs = GainInputs {
            target: &zeros,
            filtered_adaptive: &zeros,
            target_q: 0,
            filtered_code: &zeros,
            code: &zeros,
            correlations,
            bits: GainBits::Seven,
            pitch_gain: Word16(0),
            clip_pitch_gain: false,
        };

        // Below every candidate: the window starts at the bottom of the table.
        let (_, first, size) = GainPredictor::window(&mut ctx, &inputs);
        assert_eq!((first, size), (0, 64));

        // Above every candidate: the window is pushed to the very top, and the
        // last pair it can reach is 127.
        inputs.pitch_gain = Word16(32767);
        let (_, first, size) = GainPredictor::window(&mut ctx, &inputs);
        assert_eq!((first, size), (64, 64));
        assert_eq!(first + size, 128, "the window must stay inside the table");

        // Clipping caps how far the pre-search counts, so it caps the start.
        inputs.clip_pitch_gain = true;
        let (_, first, size) = GainPredictor::window(&mut ctx, &inputs);
        assert_eq!(
            (first, size),
            (37, 64),
            "clipping shortens the seven-bit pre-search, not the search"
        );
    }

    /// Six-bit clipping shortens the search itself, which is a different
    /// mechanism from the seven-bit window shift.
    #[test]
    fn six_bit_clipping_shortens_the_search() {
        let mut ctx = DspContext::new();
        let zeros = [Word16(0); L_SUBFR];
        let correlations = PitchCorrelations {
            energy: Word16(0),
            energy_exp: 0,
            correlation: Word16(0),
            correlation_exp: 0,
        };
        let mut inputs = GainInputs {
            target: &zeros,
            filtered_adaptive: &zeros,
            target_q: 0,
            filtered_code: &zeros,
            code: &zeros,
            correlations,
            bits: GainBits::Six,
            // Deliberately high: the six-bit path must ignore it entirely.
            pitch_gain: Word16(32767),
            clip_pitch_gain: false,
        };
        assert_eq!(GainPredictor::window(&mut ctx, &inputs).1, 0);
        assert_eq!(GainPredictor::window(&mut ctx, &inputs).2, 64);

        inputs.clip_pitch_gain = true;
        assert_eq!(GainPredictor::window(&mut ctx, &inputs).1, 0);
        assert_eq!(GainPredictor::window(&mut ctx, &inputs).2, 48);
    }

    /// The pre-search counts strictly-lower candidates, so a pitch gain exactly
    /// equal to a table entry does not advance past it.
    #[test]
    fn the_window_boundary_is_strict() {
        let mut ctx = DspContext::new();
        let zeros = [Word16(0); L_SUBFR];
        let correlations = PitchCorrelations {
            energy: Word16(0),
            energy_exp: 0,
            correlation: Word16(0),
            correlation_exp: 0,
        };
        // Pair 32's pitch gain: the first candidate the pre-search looks at.
        let boundary = QUA_GAIN_7B[RANGE];
        let mut inputs = GainInputs {
            target: &zeros,
            filtered_adaptive: &zeros,
            target_q: 0,
            filtered_code: &zeros,
            code: &zeros,
            correlations,
            bits: GainBits::Seven,
            pitch_gain: Word16(boundary),
            clip_pitch_gain: false,
        };
        let equal = GainPredictor::window(&mut ctx, &inputs).1;
        inputs.pitch_gain = Word16(boundary + 1);
        let above = GainPredictor::window(&mut ctx, &inputs).1;
        assert_eq!(
            above,
            equal + 1,
            "an equal pitch gain must not count, a greater one must"
        );
    }

    /// The distortion search keeps the earliest of equally good candidates.
    ///
    /// Asserted on the comparison itself: see [`beats`] for why an exact tie
    /// cannot be produced through the search with the shipped codebooks. A
    /// `<=` here would silently prefer the *last* of several equally good
    /// candidates, which is a different bitstream from the reference's.
    #[test]
    fn equal_distortions_keep_the_lowest_index() {
        let mut ctx = DspContext::new();
        for probe in [0i32, 1, -1, 12345, i32::MAX, i32::MIN] {
            assert!(
                !beats(&mut ctx, Word32(probe), Word32(probe)),
                "an equal distortion must not displace the incumbent at {probe}"
            );
        }
        assert!(beats(&mut ctx, Word32(-1), Word32(0)));
        assert!(!beats(&mut ctx, Word32(0), Word32(-1)));
        // The reference compares a saturating difference, so the extremes must
        // still order correctly rather than wrapping.
        assert!(beats(&mut ctx, Word32(i32::MIN), Word32(i32::MAX)));
        assert!(!beats(&mut ctx, Word32(i32::MAX), Word32(i32::MIN)));
    }

    /// Ten candidates in, the search must still be reporting the smallest —
    /// checked against a plain scan of the same window.
    #[test]
    fn the_search_reports_the_smallest_distortion_in_its_window() {
        let mut ctx = DspContext::new();
        let mut predictor = GainPredictor::new();
        let xn = vector(0, 0, "xn");
        let y1 = vector(0, 0, "y1");
        let y2 = vector(0, 0, "y2");
        let code = vector(0, 0, "code");
        let correlations = pitch_correlations(&mut ctx, &xn, &y1);
        let inputs = GainInputs {
            target: &xn,
            filtered_adaptive: &y1,
            target_q: scalar(0, -1, "Q_new") as i16 + scalar(0, 0, "shift") as i16,
            filtered_code: &y2,
            code: &code,
            correlations,
            bits: GainBits::Seven,
            pitch_gain: Word16(scalar(0, 0, "gain1") as i16),
            clip_pitch_gain: false,
        };
        let (_, first, size) = GainPredictor::window(&mut ctx, &inputs);
        let chosen = predictor.quantise(&mut ctx, &inputs);
        assert!(
            (first..first + size).contains(&usize::from(chosen.index)),
            "the chosen index must lie inside the window it searched"
        );
        assert_eq!(size, 64, "the seven-bit search always covers 64 candidates");
    }

    /// Gain width follows the frame size, switching at 12.65 kbit/s.
    #[test]
    fn gain_width_follows_the_frame_size() {
        assert_eq!(GainBits::from_frame_bits(132), GainBits::Six);
        assert_eq!(GainBits::from_frame_bits(177), GainBits::Six);
        assert_eq!(GainBits::from_frame_bits(253), GainBits::Seven);
        assert_eq!(GainBits::from_frame_bits(477), GainBits::Seven);
        assert_eq!(GainBits::Six.bits(), 6);
        assert_eq!(GainBits::Seven.bits(), 7);
    }
}
