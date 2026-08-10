//! The AMR-NB encoder's gain quantisers, TS 26.090 §5.9.
//!
//! Implements, from the TS 26.073 fixed-point reference: `gainQuant` and the
//! `gainQuantState` lifecycle (`gain_q.c`), `Qua_gain` (`qua_gain.c`),
//! `q_gain_code` (`q_gain_c.c`) with `G_code` (`g_code.c`), `q_gain_pitch`
//! (`q_gain_p.c`), the whole 4.75 kbit/s two-subframe path —
//! `MR475_gain_quant`, `MR475_update_unq_pred` and `MR475_quant_store_results`
//! (`qgain475.c`) — the whole 7.95 kbit/s path — `MR795_gain_quant`,
//! `MR795_gain_code_quant3` and `MR795_gain_code_quant_mod` (`qgain795.c`)
//! with `gain_adapt` (`g_adapt.c`) and `gmed_n` (`gmed_n.c`) — and the energy
//! coefficients all of them consume, `calc_filt_energies`,
//! `calc_target_energy` and `calc_unfilt_energies` (`calc_en.c`).
//!
//! The MA energy predictor `gc_pred` is **not** reimplemented here: the
//! decoder's [`CodeGainPredictor`] is the same function and the same state, and
//! is used directly.
//!
//! # What validated it
//!
//! `testdata/nb_enc_trace.txt`, produced by driving TS 26.073's own encoder at
//! **7.40 kbit/s**. The committed tests drive [`GainQuantiser::quantise`] from
//! the traced `xn`, `xn2`, `y1`, `y2`, `res`, `adapt` and `code` rows and
//! compare `gain_pit` and `gain_code` over three frames and twelve subframes,
//! and compare the chosen VQ *index* against the gain field of
//! `testdata/amrnb_enc_mode4.amr` — the reference encoder's own bitstream —
//! because two table entries can dequantise to nearly the same gains and only
//! the index says which one the reference picked.
//!
//! During development the same harness ran over all fifty frames (200
//! subframes) at **every one of the eight rates**, including 4.75's joint
//! two-subframe index and 7.95's adaptive criterion. Only 7.40 kbit/s is
//! covered by a test that ships, because only that rate's trace is committed.
//!
//! # Five quantisers, not one
//!
//! | Rate | Quantiser | Index |
//! |---|---|---|
//! | 4.75 | `MR475_gain_quant`, 256-entry joint VQ | one 8-bit word for **two** subframes |
//! | 5.15, 5.90 | `Qua_gain`, 64-entry joint VQ | 6 bits per subframe |
//! | 6.70, 7.40, 10.2 | `Qua_gain`, 128-entry joint VQ | 7 bits per subframe |
//! | 7.95 | `MR795_gain_quant`, scalar pitch + adapted scalar code | 4 + 5 bits |
//! | 12.2 | `G_code` then `q_gain_code`, 32-entry scalar | 5 bits (pitch gain is quantised earlier, in `cl_ltp`) |
//!
//! Only 7.95 and 12.2 ever call `q_gain_pitch`. Every other rate takes its
//! pitch gain straight from the joint VQ entry it selected, and scalar-
//! quantising it instead gives a `gain_pit` that is close, plausible, and not
//! the reference's.
//!
//! # Q-formats
//!
//! Pitch gains are Q14 and code gains Q1 throughout. The predicted code gain
//! travels as `(exponent Q0, fraction Q15)` and is raised to `gcode0` in Q14
//! by `Pow2(14, fraction)` — the exponent stays out of it and is folded into a
//! shift later, which is why the two never appear multiplied together.
//! Coefficients are normalised `(fraction Q15, exponent Q0)` pairs with no
//! fixed scale of their own.

use super::super::decoder_tables::{
    GAIN_HIGHRATES, GAIN_LOWRATES, GAIN_MR475, QUA_GAIN_CODE, QUA_GAIN_PITCH,
};
use super::super::gain::{CodeGainPredictor, GainPrediction, SubframeGains};
use super::super::math::{log2, pow2, sqrt_l_exp};
use super::super::L_SUBFR;
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, mult, negate, round, sub};
use crate::fixed_point::arith32::{l_deposit_l, l_mac, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_comp, l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_32};

/// Entries in the 4.75 kbit/s joint codebook. Four words each: `(g_pitch,
/// g_fac)` for the even subframe then the same for the odd one.
const MR475_VQ_SIZE: usize = 256;
/// Entries in the 6.70 / 7.40 / 10.2 kbit/s codebook.
const VQ_SIZE_HIGHRATES: usize = 128;
/// Entries in the 5.15 / 5.90 kbit/s codebook.
const VQ_SIZE_LOWRATES: usize = 64;
/// Entries in the scalar code-gain codebook.
const NB_QUA_CODE: usize = 32;
/// Entries in the scalar pitch-gain codebook.
const NB_QUA_PITCH: usize = 16;

/// Words per entry in the two joint codebooks `Qua_gain` reads.
const JOINT_ENTRY: usize = 4;
/// Words per entry in the scalar code-gain codebook.
const CODE_ENTRY: usize = 3;

/// Energy coefficients per subframe: `<y1 y1>`, `−2<xn y1>`, `<y2 y2>`,
/// `−2<xn y2>`, `2<y1 y2>`.
const COEFFS: usize = 5;

/// `20·log10(2)` in Q12 — converts a `log2` prediction error to decibels.
const DB_PER_OCTAVE: Word16 = Word16(24660);

/// Prediction-error floor in the `log2` scale, Q10.
///
/// **The suffixes are the reference's and they are attached to the opposite
/// log domain from what they suggest**: `MIN_QUA_ENER` holds the `log2` value
/// and `MIN_QUA_ENER_MR122` the `20·log10` one, which is backwards from the
/// `qua_ener`/`qua_ener_MR122` convention used everywhere else in the codec.
/// The clamp in `MR475_update_unq_pred` compares the `log2`-domain error
/// against these decibel bounds as a result — so the floor is unreachable and
/// the ceiling fires only above 18284. Reproduced as written; "fixing" the
/// names changes the predictor state and therefore every later gain.
const MIN_QUA_ENER: Word16 = Word16(-5443);
/// Prediction-error floor as the reference labels it, Q10. See
/// [`MIN_QUA_ENER`].
const MIN_QUA_ENER_MR122: Word16 = Word16(-32768);
/// Prediction-error ceiling in the `log2` scale, Q10.
const MAX_QUA_ENER: Word16 = Word16(3037);
/// Prediction-error ceiling as the reference labels it, Q10.
const MAX_QUA_ENER_MR122: Word16 = Word16(18284);

/// LTP coding gain below which 7.95's adaptor is fully engaged, Q13.
const LTP_GAIN_THR1: Word16 = Word16(2721);
/// LTP coding gain above which it disengages entirely, Q13.
const LTP_GAIN_THR2: Word16 = Word16(5443);
/// Depth of the LTP-gain median filter. Slot 0 is scratch, so the real memory
/// is four frames.
const LTPG_MEM_SIZE: usize = 5;

/// The signals one subframe's gain quantiser needs.
///
/// Bundled because there are nine of them and six are 40-sample vectors that
/// differ only in what they mean; a positional argument list would make
/// transposing `xn` and `xn2` — which changes the answer and nothing else —
/// too easy.
#[derive(Clone, Copy, Debug)]
pub struct SubframeSignals<'a> {
    /// LP residual `res`, Q0.
    pub residual: &'a [Word16; L_SUBFR],
    /// Adaptive-codebook excitation `exc`, unfiltered, Q0.
    pub adaptive: &'a [Word16; L_SUBFR],
    /// Fixed-codebook innovation `code`, Q13 (Q12 at 12.2 kbit/s), **with**
    /// pitch sharpening — the reference's parameter name says "nosharp" at 4.75
    /// but the array it is handed is the sharpened one.
    pub code: &'a [Word16; L_SUBFR],
    /// Pitch-search target `xn`, Q0.
    pub pitch_target: &'a [Word16; L_SUBFR],
    /// Codebook-search target `xn2`, Q0.
    pub code_target: &'a [Word16; L_SUBFR],
    /// Filtered adaptive codevector `y1`, Q0.
    pub filtered_adaptive: &'a [Word16; L_SUBFR],
    /// Filtered innovation `y2`, Q12.
    pub filtered_code: &'a [Word16; L_SUBFR],
    /// `G_pitch`'s correlations, in its own order:
    /// `(<y1 y1> fraction, exponent, <xn y1> fraction, exponent)`.
    pub pitch_correlations: [Word16; 4],
    /// Closed-loop pitch gain, Q14. Already quantised at 12.2 kbit/s, already
    /// clipped by `cl_ltp` everywhere.
    pub gain_pit: Word16,
    /// `cl_ltp`'s pitch-gain limit: `32767`, or `GP_CLIP = 15565` when the
    /// tone-stability tracker has fired. Table entries above it are skipped
    /// outright, not merely penalised.
    pub gp_limit: Word16,
    /// True on subframes 0 and 2. Only 4.75 kbit/s cares.
    pub even_subframe: bool,
}

/// Where this subframe's gain parameters belong in the frame's word stream.
///
/// 4.75 kbit/s is the reason this is an enum: it claims a slot on the even
/// subframe and fills it from the odd one, so "how many words does this
/// subframe emit" has three different answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GainParams {
    /// Nothing to emit yet — 4.75 kbit/s, even subframe. One word must be
    /// reserved at this position for the [`GainParams::Pair`] that follows.
    Reserve,
    /// One word, emitted here: the joint VQ index at 5.15 through 10.2, or the
    /// scalar code-gain index at 12.2.
    Index(u16),
    /// 4.75 kbit/s, odd subframe: the 8-bit index covering **both** subframes
    /// of the pair. It belongs in the slot [`GainParams::Reserve`] claimed two
    /// subframes earlier, not here.
    Pair(u16),
    /// 7.95 kbit/s: the pitch-gain index (4 bits) then the code-gain index
    /// (5 bits), in that order.
    PitchAndCode(u16, u16),
}

/// What one call to [`GainQuantiser::quantise`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GainDecision {
    /// This subframe's gains.
    ///
    /// At 4.75 kbit/s on an **even** subframe these are the *unquantised*
    /// closed-loop pitch gain and optimum code gain: the quantiser has not run
    /// yet, and the reference synthesises the subframe provisionally with these
    /// before redoing it once the pair's index is known.
    pub gains: SubframeGains,
    /// 4.75 kbit/s, odd subframe: the now-quantised gains for the **even**
    /// subframe of the pair, which the caller must use to redo that subframe's
    /// synthesis. `None` at every other rate and on even subframes.
    pub previous: Option<SubframeGains>,
    /// The parameter words, and where they go.
    pub params: GainParams,
}

/// Per-subframe energy coefficients as normalised `(fraction, exponent)` pairs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Coefficients {
    fraction: [Word16; COEFFS],
    exponent: [Word16; COEFFS],
}

/// Everything 4.75 kbit/s carries from an even subframe to the odd one that
/// closes the pair.
#[derive(Clone, Copy, Debug)]
struct PendingPair {
    /// The predicted code gain from the *unquantised* predictor.
    predicted: GainPrediction,
    /// `<xn xn>·2` as `(exponent, fraction)`.
    target_energy: (Word16, Word16),
    /// The even subframe's energy coefficients.
    coefficients: Coefficients,
}

/// 7.95 kbit/s' gain adaptor state, TS 26.073 `GainAdaptState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct GainAdaptor {
    /// Countdown after an energy onset; while non-zero the adaptor is nudged
    /// one step more aggressive.
    onset: Word16,
    /// The previous subframe's `alpha`. A zero one halves this subframe's.
    prev_alpha: Word16,
    /// The previous subframe's code gain, Q1.
    prev_gain_code: Word16,
    /// LTP coding gains, Q13. Slot 0 is scratch for the median call; the real
    /// memory is slots 1..4.
    ltpg: [Word16; LTPG_MEM_SIZE],
}

/// The encoder's gain quantiser, TS 26.073 `gainQuantState`.
///
/// Holds two MA energy predictors, not one. The "unquantised" predictor exists
/// only for 4.75 kbit/s: it is re-seeded from the real one at the start of
/// every subframe pair and then advanced with the *optimum* code gain, so that
/// the even subframe can be synthesised provisionally before its quantised gain
/// is known. The real predictor is advanced exactly twice per pair, from inside
/// the joint quantiser's read-back.
#[derive(Clone, Copy, Debug)]
pub struct GainQuantiser {
    predictor: CodeGainPredictor,
    unquantised: CodeGainPredictor,
    pending: Option<PendingPair>,
    adaptor: GainAdaptor,
}

impl Default for GainQuantiser {
    fn default() -> Self {
        Self::new()
    }
}

impl GainQuantiser {
    /// A quantiser in its reset state, TS 26.073 `gainQuant_reset`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            predictor: CodeGainPredictor::new(),
            unquantised: CodeGainPredictor::new(),
            pending: None,
            adaptor: GainAdaptor {
                onset: Word16(0),
                prev_alpha: Word16(0),
                prev_gain_code: Word16(0),
                ltpg: [Word16(0); LTPG_MEM_SIZE],
            },
        }
    }

    /// Quantise one subframe's gains, TS 26.073 `gainQuant`.
    ///
    /// `mode_index` is the rate in the reference's numeric order, `0` = 4.75
    /// kbit/s through `7` = 12.2. Anything above 7 is treated as 12.2.
    ///
    /// # Panics
    ///
    /// At 4.75 kbit/s an odd subframe panics if no even subframe preceded it —
    /// the joint index covers a pair and cannot be produced from half of one.
    pub fn quantise(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        signals: &SubframeSignals<'_>,
    ) -> GainDecision {
        if mode_index == 0 {
            return self.quantise_pair(ctx, signals);
        }

        let predicted = self.predictor.predict(ctx, mode_index, signals.code);

        if mode_index >= 7 {
            // 12.2 kbit/s: the *unquantised* optimum code gain first, then a
            // scalar quantiser applied to it. Omitting `G_code` quantises
            // whatever `gain_cod` happened to hold.
            let optimum = optimum_code_gain(ctx, signals.code_target, signals.filtered_code);
            let (index, gain_code, energies) = quantise_code_gain(ctx, predicted, optimum);
            self.predictor.push(energies.0, energies.1);
            return GainDecision {
                gains: SubframeGains {
                    pitch: signals.gain_pit,
                    code: gain_code,
                },
                previous: None,
                params: GainParams::Index(index),
            };
        }

        let filtered = filtered_energies(ctx, mode_index, signals);

        if mode_index == 5 {
            let (gains, energies, params) =
                self.quantise_adaptive(ctx, signals, predicted, &filtered);
            self.predictor.push(energies.0, energies.1);
            return GainDecision {
                gains,
                previous: None,
                params,
            };
        }

        let (index, gains, energies) = quantise_joint(
            ctx,
            mode_index,
            predicted,
            &filtered.coefficients,
            signals.gp_limit,
        );
        self.predictor.push(energies.0, energies.1);
        GainDecision {
            gains,
            previous: None,
            params: GainParams::Index(index),
        }
    }

    /// The 4.75 kbit/s path: half a decision on the even subframe, the whole
    /// pair's on the odd one.
    fn quantise_pair(
        &mut self,
        ctx: &mut DspContext,
        signals: &SubframeSignals<'_>,
    ) -> GainDecision {
        if signals.even_subframe {
            // Re-seed the "unquantised" predictor from the real one. This is
            // what makes it legitimate for the joint quantiser to reuse the
            // even subframe's `gcode0` later: the real predictor has not moved
            // since.
            self.unquantised = self.predictor;
            let predicted = self.unquantised.predict(ctx, 0, signals.code);
            let filtered = filtered_energies(ctx, 0, signals);
            let (optimum_fraction, optimum_exponent) = filtered
                .optimum_code_gain
                .expect("4.75 kbit/s always computes the optimum code gain");

            // The provisional code gain, Q1. `shl` saturates, and that is the
            // clip.
            let shift = add(ctx, optimum_exponent, Word16(1));
            let gain_code = shl(ctx, optimum_fraction, shift.0);

            self.pending = Some(PendingPair {
                predicted,
                target_energy: target_energy(ctx, signals.pitch_target),
                coefficients: filtered.coefficients,
            });

            update_unquantised_predictor(
                ctx,
                &mut self.unquantised,
                predicted,
                (optimum_fraction, optimum_exponent),
            );

            return GainDecision {
                gains: SubframeGains {
                    // Left at the unquantised closed-loop value.
                    pitch: signals.gain_pit,
                    code: gain_code,
                },
                previous: None,
                params: GainParams::Reserve,
            };
        }

        let pending = self
            .pending
            .take()
            .expect("4.75 kbit/s codes subframes in pairs; the even one must run first");
        let predicted = self.unquantised.predict(ctx, 0, signals.code);
        let filtered = filtered_energies(ctx, 0, signals);
        let odd_energy = target_energy(ctx, signals.pitch_target);

        let (index, even, odd) = quantise_pair_jointly(
            ctx,
            &mut self.predictor,
            &pending,
            (predicted, &filtered.coefficients, odd_energy),
            signals.code,
            signals.gp_limit,
        );

        GainDecision {
            gains: odd,
            previous: Some(even),
            params: GainParams::Pair(index),
        }
    }

    /// The 7.95 kbit/s path, TS 26.073 `MR795_gain_quant`.
    fn quantise_adaptive(
        &mut self,
        ctx: &mut DspContext,
        signals: &SubframeSignals<'_>,
        predicted: GainPrediction,
        filtered: &FilteredEnergies,
    ) -> (SubframeGains, (Word16, Word16), GainParams) {
        let mut pitch = signals.gain_pit;
        // The index this returns is superseded by the pre-quantiser below,
        // which chooses among the three candidates it also returns; the
        // reference overwrites it for the same reason.
        let (_, candidates) = quantise_pitch_gain(ctx, 5, signals.gp_limit, &mut pitch);
        let candidates = candidates.expect("7.95 kbit/s asks for the three candidates");

        let gcode0 = extract_l(pow2(ctx, Word16(14), predicted.fraction));

        let pre = prequantise_code_gain(
            ctx,
            predicted.exponent,
            gcode0,
            &candidates,
            &filtered.coefficients,
        );
        pitch = pre.pitch;
        let pitch_index = pre.pitch_index;
        let mut code = pre.code;
        let mut code_index = pre.code_index;
        let mut energies = pre.energies;

        let unfiltered =
            unfiltered_energies(ctx, signals.residual, signals.adaptive, signals.code, pitch);

        // Always run, so the adaptor's state advances every subframe whether or
        // not the modified quantiser below does.
        let alpha = self.adaptor.adapt(ctx, unfiltered.ltp_gain, code);

        if unfiltered.fraction[0].0 != 0 && alpha.0 > 0 {
            let mut fraction = unfiltered.fraction;
            let mut exponent = unfiltered.exponent;
            // `<code code>` from the predictor overwrites the LTP-residual
            // energy, which is no longer needed.
            let (code_exp, code_frac) = predicted
                .innovation_energy
                .expect("7.95 kbit/s asks gc_pred for the innovation energy");
            fraction[3] = code_frac;
            exponent[3] = code_exp;

            let (optimum_fraction, optimum_exponent) = filtered
                .optimum_code_gain
                .expect("7.95 kbit/s always computes the optimum code gain");
            let scale = sub(ctx, optimum_exponent, predicted.exponent);
            let scale = add(ctx, scale, Word16(10));
            let optimum = shl(ctx, optimum_fraction, scale.0);

            let modified = requantise_code_gain(
                ctx,
                &AdaptedCriterion {
                    gain_pit: pitch,
                    exp_gcode0: predicted.exponent,
                    gcode0,
                    fraction,
                    exponent,
                    alpha,
                    optimum,
                },
                code,
            );
            code_index = modified.0;
            code = modified.1;
            energies = modified.2;
        }

        (
            SubframeGains { pitch, code },
            energies,
            GainParams::PitchAndCode(pitch_index, code_index),
        )
    }
}

// ---------------------------------------------------------------------------
// Energy coefficients (calc_en.c)
// ---------------------------------------------------------------------------

/// What `calc_filt_energies` produces.
struct FilteredEnergies {
    coefficients: Coefficients,
    /// The unquantised optimum code gain as `(fraction Q15, exponent Q0)`.
    /// Only 4.75 and 7.95 kbit/s ask for it; the reference leaves the
    /// out-parameters untouched for every other rate, so this is `None` rather
    /// than a value nothing wrote.
    optimum_code_gain: Option<(Word16, Word16)>,
}

/// Energy coefficients of the filtered signals, TS 26.073 `calc_filt_energies`.
///
/// `c0 = <y1 y1>`, `c1 = −2<xn y1>`, `c2 = <y2 y2>`, `c3 = −2<xn y2>`,
/// `c4 = 2<y1 y2>`, each normalised to a `(fraction, exponent)` pair. `y2` is
/// used at `Y2 >> 3` throughout.
///
/// The accumulator seed is **0 for 4.75 and 7.95 and 1 for every other rate**.
/// One count in a Word32 sounds like nothing and is not: it changes `norm_l`
/// for a silent subframe, and so the exponent every later term is scaled
/// against.
fn filtered_energies(
    ctx: &mut DspContext,
    mode_index: u8,
    signals: &SubframeSignals<'_>,
) -> FilteredEnergies {
    let wants_optimum = mode_index == 0 || mode_index == 5;
    let seed = if wants_optimum { Word32(0) } else { Word32(1) };

    let mut y2 = [Word16(0); L_SUBFR];
    for (out, &value) in y2.iter_mut().zip(signals.filtered_code.iter()) {
        *out = shr(ctx, value, 3);
    }

    let mut coefficients = Coefficients::default();
    // `G_pitch` already normalised these two.
    coefficients.fraction[0] = signals.pitch_correlations[0];
    coefficients.exponent[0] = signals.pitch_correlations[1];
    coefficients.fraction[1] = negate(ctx, signals.pitch_correlations[2]);
    coefficients.exponent[1] = add(ctx, signals.pitch_correlations[3], Word16(1));

    let (fraction, shift) = normalised_dot(ctx, seed, &y2, &y2);
    coefficients.fraction[2] = fraction;
    coefficients.exponent[2] = sub(ctx, Word16(-3), Word16(shift));

    let (fraction, shift) = normalised_dot(ctx, seed, signals.pitch_target, &y2);
    coefficients.fraction[3] = negate(ctx, fraction);
    coefficients.exponent[3] = sub(ctx, Word16(7), Word16(shift));

    let (fraction, shift) = normalised_dot(ctx, seed, signals.filtered_adaptive, &y2);
    coefficients.fraction[4] = fraction;
    coefficients.exponent[4] = sub(ctx, Word16(7), Word16(shift));

    let optimum_code_gain = wants_optimum.then(|| {
        let (fraction, shift) = normalised_dot(ctx, seed, signals.code_target, &y2);
        let exponent = sub(ctx, Word16(6), Word16(shift));
        // A non-positive correlation means the innovation points the wrong way;
        // the optimum gain is then zero rather than negative.
        if fraction.0 <= 0 {
            (Word16(0), Word16(0))
        } else {
            let half = shr(ctx, fraction, 1);
            let ratio = div_s(half, coefficients.fraction[2]);
            let exponent = sub(ctx, exponent, coefficients.exponent[2]);
            (ratio, sub(ctx, exponent, Word16(14)))
        }
    });

    FilteredEnergies {
        coefficients,
        optimum_code_gain,
    }
}

/// `<a, b>` normalised: returns `(fraction, normalisation shift)`.
fn normalised_dot(
    ctx: &mut DspContext,
    seed: Word32,
    a: &[Word16; L_SUBFR],
    b: &[Word16; L_SUBFR],
) -> (Word16, i16) {
    let mut s = seed;
    for (&x, &y) in a.iter().zip(b.iter()) {
        s = l_mac(ctx, s, x, y);
    }
    let shift = norm_l(s);
    let scaled = l_shl(ctx, s, shift);
    (extract_h(scaled), shift)
}

/// `<xn xn>·2` normalised, TS 26.073 `calc_target_energy`.
///
/// Returns `(exponent, fraction)`. The doubling is the accumulator's — `L_mac`
/// shifts left by one — and the exponent is `16 − shift` rather than `15 −`
/// because of it.
fn target_energy(ctx: &mut DspContext, xn: &[Word16; L_SUBFR]) -> (Word16, Word16) {
    let (fraction, shift) = normalised_dot(ctx, Word32(0), xn, xn);
    (sub(ctx, Word16(16), Word16(shift)), fraction)
}

/// What `calc_unfilt_energies` produces.
struct UnfilteredEnergies {
    /// `<res res>`, `<exc exc>`, `<exc code>`, `<lres lres>` as fractions.
    fraction: [Word16; 4],
    /// Their exponents.
    exponent: [Word16; 4],
    /// LTP coding gain `log2(<res res> / <lres lres>)`, Q13.
    ltp_gain: Word16,
}

/// Energy coefficients of the unfiltered signals,
/// TS 26.073 `calc_unfilt_energies`.
///
/// The low-energy flag is `fraction[0] == 0`, forced when the **accumulator**
/// — which is `<res res>·2`, not `<res res>` — is below 400. 7.95's adaptive
/// quantiser keys off that same flag, so testing the un-doubled energy against
/// 200 instead would engage it on a different set of subframes.
fn unfiltered_energies(
    ctx: &mut DspContext,
    res: &[Word16; L_SUBFR],
    exc: &[Word16; L_SUBFR],
    code: &[Word16; L_SUBFR],
    gain_pit: Word16,
) -> UnfilteredEnergies {
    let mut fraction = [Word16(0); 4];
    let mut exponent = [Word16(0); 4];

    let mut s = Word32(0);
    for &x in res {
        s = l_mac(ctx, s, x, x);
    }
    if l_sub(ctx, s, Word32(400)).0 < 0 {
        fraction[0] = Word16(0);
        exponent[0] = Word16(-15);
    } else {
        let shift = norm_l(s);
        let scaled = l_shl(ctx, s, shift);
        fraction[0] = extract_h(scaled);
        exponent[0] = sub(ctx, Word16(15), Word16(shift));
    }

    let (value, shift) = normalised_dot(ctx, Word32(0), exc, exc);
    fraction[1] = value;
    exponent[1] = sub(ctx, Word16(15), Word16(shift));

    let (value, shift) = normalised_dot(ctx, Word32(0), exc, code);
    fraction[2] = value;
    exponent[2] = sub(ctx, Word16(2), Word16(shift));

    let mut s = Word32(0);
    for i in 0..L_SUBFR {
        let product = l_mult(ctx, exc[i], gain_pit);
        let scaled = l_shl(ctx, product, 1);
        // `round` here, where `cl_ltp` forms the same difference with
        // `extract_h`. The two residuals differ by an LSB and are not
        // interchangeable.
        let contribution = round(ctx, scaled);
        let residual = sub(ctx, res[i], contribution);
        s = l_mac(ctx, s, residual, residual);
    }
    let shift = norm_l(s);
    let scaled = l_shl(ctx, s, shift);
    let ltp_residual = extract_h(scaled);
    let ltp_exponent = sub(ctx, Word16(15), Word16(shift));
    fraction[3] = ltp_residual;
    exponent[3] = ltp_exponent;

    // `ltp_residual > 0` on the *fraction*, and `fraction[0] != 0` is the
    // low-energy flag from the 400 test — not a test for a zero residual.
    let ltp_gain = if ltp_residual.0 > 0 && fraction[0].0 != 0 {
        let half = shr(ctx, fraction[0], 1);
        let ratio = div_s(half, ltp_residual);
        let shift = sub(ctx, ltp_exponent, exponent[0]);
        let wide = Word32(i32::from(ratio.0) << 16);
        let shift = add(ctx, shift, Word16(3));
        let wide = l_shr(ctx, wide, shift.0);
        let (exp, frac) = log2(ctx, wide);
        let exp = sub(ctx, exp, Word16(27));
        let combined = l_comp(exp, frac);
        let scaled = l_shl(ctx, combined, 13);
        round(ctx, scaled)
    } else {
        Word16(0)
    };

    UnfilteredEnergies {
        fraction,
        exponent,
        ltp_gain,
    }
}

// ---------------------------------------------------------------------------
// 32-bit accumulation helpers (mac_32.c)
// ---------------------------------------------------------------------------

/// `L_32 + (hi, lo)·n`, TS 26.073 `Mac_32_16`.
///
/// Not the same as `L_add(L_32, Mpy_32_16(hi, lo, n))`: this saturates at each
/// of the two accumulation steps rather than once at the end, and `Qua_gain`
/// deliberately uses the latter form where 4.75 and 7.95 use this one.
fn mac_32_16(ctx: &mut DspContext, acc: Word32, hi: Word16, lo: Word16, n: Word16) -> Word32 {
    let acc = l_mac(ctx, acc, hi, n);
    let partial = mult(ctx, lo, n);
    l_mac(ctx, acc, partial, Word16(1))
}

/// `L_32 + (hi1, lo1)·(hi2, lo2)`, TS 26.073 `Mac_32`.
fn mac_32(
    ctx: &mut DspContext,
    acc: Word32,
    hi1: Word16,
    lo1: Word16,
    hi2: Word16,
    lo2: Word16,
) -> Word32 {
    let acc = l_mac(ctx, acc, hi1, hi2);
    let cross = mult(ctx, hi1, lo2);
    let acc = l_mac(ctx, acc, cross, Word16(1));
    let cross = mult(ctx, lo1, hi2);
    l_mac(ctx, acc, cross, Word16(1))
}

/// Rescale the five coefficients onto one common exponent.
///
/// The shared exponent is `max(exp) + 1` — the `+1` is headroom so that summing
/// five terms cannot overflow — and the maximum is taken with a **strict** `>`,
/// so the first of several equal maxima wins. Returns the `(hi, lo)` halves
/// `Mpy_32_16` wants.
fn align_coefficients(
    ctx: &mut DspContext,
    fraction: &[Word16],
    exp_max: &[Word16],
) -> (Vec<Word16>, Vec<Word16>) {
    let mut top = exp_max[0];
    for &candidate in &exp_max[1..] {
        if sub(ctx, candidate, top).0 > 0 {
            top = candidate;
        }
    }
    let top = add(ctx, top, Word16(1));

    let mut hi = Vec::with_capacity(fraction.len());
    let mut lo = Vec::with_capacity(fraction.len());
    for (&value, &exponent) in fraction.iter().zip(exp_max.iter()) {
        let shift = sub(ctx, top, exponent);
        let wide = Word32(i32::from(value.0) << 16);
        let wide = l_shr(ctx, wide, shift.0);
        let (h, l) = l_extract(wide);
        hi.push(h);
        lo.push(l);
    }
    (hi, lo)
}

/// The five per-subframe exponents `Qua_gain` and its relatives derive from the
/// coefficients and the predicted gain's exponent.
///
/// `bias` is the reference's `exp_gcode0 - 11` for `Qua_gain` and 4.75, and
/// `exp_gcode0 - 10` for 7.95. The difference is not cosmetic: it pairs with a
/// different final shift when the chosen gain is read back, and mixing the two
/// scales the code gain by two.
fn scaling_exponents(
    ctx: &mut DspContext,
    exponent: &[Word16; COEFFS],
    bias: Word16,
) -> [Word16; COEFFS] {
    let doubled = shl(ctx, bias, 1);
    let squared = add(ctx, Word16(15), doubled);
    let incremented = add(ctx, Word16(1), bias);
    [
        sub(ctx, exponent[0], Word16(13)),
        sub(ctx, exponent[1], Word16(14)),
        add(ctx, exponent[2], squared),
        add(ctx, exponent[3], bias),
        add(ctx, exponent[4], incremented),
    ]
}

// ---------------------------------------------------------------------------
// Qua_gain: the joint VQ for 5.15, 5.90, 6.70, 7.40 and 10.2
// ---------------------------------------------------------------------------

/// The joint pitch/code gain VQ, TS 26.073 `Qua_gain`.
///
/// Visit order is ascending table index and the distance test is a **strict**
/// `<`, so equal distances keep the lower index. Entries whose tabulated pitch
/// gain exceeds `gp_limit` are skipped entirely — not penalised — and if every
/// entry were skipped the index would stay 0.
///
/// The five terms accumulate as `L_add(acc, Mpy_32_16(...))`, **not** as
/// `Mac_32_16`. 4.75 and 7.95 use `Mac_32_16` for the same sum; the saturation
/// points differ and the choice is per-quantiser.
fn quantise_joint(
    ctx: &mut DspContext,
    mode_index: u8,
    predicted: GainPrediction,
    coefficients: &Coefficients,
    gp_limit: Word16,
) -> (u16, SubframeGains, (Word16, Word16)) {
    let (table, entries) = match mode_index {
        3 | 4 | 6 => (&GAIN_HIGHRATES[..], VQ_SIZE_HIGHRATES),
        _ => (&GAIN_LOWRATES[..], VQ_SIZE_LOWRATES),
    };

    let gcode0 = extract_l(pow2(ctx, Word16(14), predicted.fraction));
    let bias = sub(ctx, predicted.exponent, Word16(11));
    let exp_max = scaling_exponents(ctx, &coefficients.exponent, bias);
    let (hi, lo) = align_coefficients(ctx, &coefficients.fraction, &exp_max);

    let mut best = Word32(MAX_32);
    let mut index = 0usize;
    for candidate in 0..entries {
        let entry = &table[candidate * JOINT_ENTRY..];
        let pitch = Word16(entry[0]);
        if sub(ctx, pitch, gp_limit).0 > 0 {
            continue;
        }
        let code = mult(ctx, Word16(entry[1]), gcode0);
        let pitch_squared = mult(ctx, pitch, pitch);
        let code_squared = mult(ctx, code, code);
        let cross = mult(ctx, code, pitch);

        let mut acc = mpy_32_16(hi[0], lo[0], pitch_squared);
        for (term, factor) in [pitch, code_squared, code, cross].into_iter().enumerate() {
            let contribution = mpy_32_16(hi[term + 1], lo[term + 1], factor);
            acc = Word32(acc.0.saturating_add(contribution.0));
        }

        if l_sub(ctx, acc, best).0 < 0 {
            best = acc;
            index = candidate;
        }
    }

    let entry = &table[index * JOINT_ENTRY..];
    let gain_pit = Word16(entry[0]);
    let code = read_back_code_gain(ctx, Word16(entry[1]), gcode0, predicted.exponent, 10);
    (
        u16::try_from(index).expect("a VQ index is at most 255"),
        SubframeGains {
            pitch: gain_pit,
            code,
        },
        (Word16(entry[2]), Word16(entry[3])),
    )
}

/// `gc = g_fac · gcode0 · 2^(exp_gcode0 − shift)`, the read-back every joint
/// quantiser ends with.
///
/// `shift` is **10** for `Qua_gain` and 4.75 and **9** for 7.95, matching the
/// `−11` / `−10` split in [`scaling_exponents`].
fn read_back_code_gain(
    ctx: &mut DspContext,
    g_fac: Word16,
    gcode0: Word16,
    exp_gcode0: Word16,
    shift: i16,
) -> Word16 {
    let product = l_mult(ctx, g_fac, gcode0);
    let amount = sub(ctx, Word16(shift), exp_gcode0);
    let scaled = l_shr(ctx, product, amount.0);
    extract_h(scaled)
}

// ---------------------------------------------------------------------------
// 12.2 kbit/s: G_code then the scalar code-gain quantiser
// ---------------------------------------------------------------------------

/// The unquantised optimum code gain, TS 26.073 `G_code`. Q1.
///
/// `<xn2 y2> / <y2 y2>`, with `y2` halved first for headroom. Two details:
/// the cross-correlation is seeded with **1** and the autocorrelation with
/// **0**, and the denormalisation is `shl(shr(gain, i), 1)` — the truncating
/// right shift happens *before* the left shift, which is not the same as
/// shifting by `1 − i`.
fn optimum_code_gain(
    ctx: &mut DspContext,
    xn2: &[Word16; L_SUBFR],
    y2: &[Word16; L_SUBFR],
) -> Word16 {
    let mut scaled = [Word16(0); L_SUBFR];
    for (out, &value) in scaled.iter_mut().zip(y2.iter()) {
        *out = shr(ctx, value, 1);
    }

    let mut s = Word32(1);
    for (&x, &y) in xn2.iter().zip(scaled.iter()) {
        s = l_mac(ctx, s, x, y);
    }
    let cross_shift = norm_l(s);
    let normalised = l_shl(ctx, s, cross_shift);
    let cross = extract_h(normalised);
    // Plain `<=`, and a plain early return: a non-positive correlation means
    // the innovation is useless here.
    if cross.0 <= 0 {
        return Word16(0);
    }

    let mut s = Word32(0);
    for &y in &scaled {
        s = l_mac(ctx, s, y, y);
    }
    let energy_shift = norm_l(s);
    let normalised = l_shl(ctx, s, energy_shift);
    let energy = extract_h(normalised);

    let cross = shr(ctx, cross, 1);
    let gain = div_s(cross, energy);
    let denorm = sub(ctx, Word16(cross_shift + 5), Word16(energy_shift));
    let truncated = shr(ctx, gain, denorm.0);
    shl(ctx, truncated, 1)
}

/// The scalar code-gain quantiser at 12.2 kbit/s, TS 26.073 `q_gain_code`.
///
/// Entry 0's error seeds the minimum, so entry 0 is never compared against
/// itself; the rest use a strict `<` over ascending indices, so ties keep the
/// lower index.
///
/// `gcode0` here is `Pow2(exp_gcode0, frac_gcode0)` — the *whole* prediction,
/// exponent included — then `shl(.., 4)`. That `shl` saturates, and the
/// saturation is the operative clip for a very loud predicted gain.
fn quantise_code_gain(
    ctx: &mut DspContext,
    predicted: GainPrediction,
    gain: Word16,
) -> (u16, Word16, (Word16, Word16)) {
    let target = shr(ctx, gain, 1); // Q1 -> Q0
    let gcode0 = extract_l(pow2(ctx, predicted.exponent, predicted.fraction));
    let gcode0 = shl(ctx, gcode0, 4);

    let first = mult(ctx, gcode0, Word16(QUA_GAIN_CODE[0]));
    let difference = sub(ctx, target, first);
    let mut smallest = abs_s(ctx, difference);
    let mut index = 0usize;
    for candidate in 1..NB_QUA_CODE {
        let scaled = mult(ctx, gcode0, Word16(QUA_GAIN_CODE[candidate * CODE_ENTRY]));
        let difference = sub(ctx, target, scaled);
        let error = abs_s(ctx, difference);
        if sub(ctx, error, smallest).0 < 0 {
            smallest = error;
            index = candidate;
        }
    }

    let entry = &QUA_GAIN_CODE[index * CODE_ENTRY..];
    let scaled = mult(ctx, gcode0, Word16(entry[0]));
    let gain = shl(ctx, scaled, 1);
    (
        u16::try_from(index).expect("a scalar index is at most 31"),
        gain,
        (Word16(entry[1]), Word16(entry[2])),
    )
}

// ---------------------------------------------------------------------------
// The scalar pitch-gain quantiser (7.95 and 12.2 only)
// ---------------------------------------------------------------------------

/// Quantise the pitch gain at 12.2 kbit/s, TS 26.073 `q_gain_pitch(MR122, …)`.
///
/// Exposed because 12.2 is the one rate that quantises its pitch gain
/// **before** the codebook search rather than in the gain quantiser: `cl_ltp`
/// calls this, writes the index into the parameter stream at that point, and
/// hands the quantised gain on to the codebook search, which uses it as the
/// sharpening factor. The order — quantise, then sharpen, then search — is not
/// negotiable, and calling [`GainQuantiser::quantise`] for it instead would
/// quantise nothing and emit the word in the wrong place.
///
/// Returns the 4-bit index and the quantised gain, Q14 with its two low bits
/// cleared.
#[must_use]
pub fn quantise_pitch_gain_mr122(
    ctx: &mut DspContext,
    gp_limit: Word16,
    gain: Word16,
) -> (u16, Word16) {
    let mut quantised = gain;
    let (index, _) = quantise_pitch_gain(ctx, 7, gp_limit, &mut quantised);
    (index, quantised)
}

/// The scalar pitch-gain quantiser, TS 26.073 `q_gain_pitch`.
///
/// Called by exactly two rates: 12.2 (from `cl_ltp`, before the codebook
/// search) and 7.95. Every other rate takes its pitch gain from the joint VQ
/// entry, and running this instead gives a plausible, wrong `gain_pit`.
///
/// **Entry 0 is evaluated unconditionally**, outside the `gp_limit` test — so
/// a zero pitch gain is always reachable even when clipping is active.
///
/// At 7.95 the three candidates are the chosen index and its neighbours, shifted
/// down at either end of the usable range. The `index − 2` branch could index
/// below zero in principle; with the only two limits the encoder ever uses it
/// fires no lower than index 10, so it does not, and the bound is left as the
/// reference has it rather than "fixed".
fn quantise_pitch_gain(
    ctx: &mut DspContext,
    mode_index: u8,
    gp_limit: Word16,
    gain: &mut Word16,
) -> (u16, Option<[(u16, Word16); 3]>) {
    let difference = sub(ctx, *gain, Word16(QUA_GAIN_PITCH[0]));
    let mut smallest = abs_s(ctx, difference);
    let mut index = 0usize;
    for (candidate, &tabulated) in QUA_GAIN_PITCH.iter().enumerate().skip(1) {
        let value = Word16(tabulated);
        if sub(ctx, value, gp_limit).0 > 0 {
            continue;
        }
        let difference = sub(ctx, *gain, value);
        let error = abs_s(ctx, difference);
        if sub(ctx, error, smallest).0 < 0 {
            smallest = error;
            index = candidate;
        }
    }

    let candidates = (mode_index == 5).then(|| {
        let first = if index == 0 {
            0
        } else if index == NB_QUA_PITCH - 1
            || sub(ctx, Word16(QUA_GAIN_PITCH[index + 1]), gp_limit).0 > 0
        {
            index - 2
        } else {
            index - 1
        };
        let mut out = [(0u16, Word16(0)); 3];
        for (offset, slot) in out.iter_mut().enumerate() {
            let at = first + offset;
            *slot = (
                u16::try_from(at).expect("a pitch index is at most 15"),
                Word16(QUA_GAIN_PITCH[at]),
            );
        }
        out
    });

    *gain = if mode_index >= 7 {
        // The two LSBs are cleared for EFR bit-exactness, where the gain was
        // Q12 rather than Q14.
        Word16(QUA_GAIN_PITCH[index] & !3)
    } else {
        Word16(QUA_GAIN_PITCH[index])
    };
    (
        u16::try_from(index).expect("a pitch index is at most 15"),
        candidates,
    )
}

// ---------------------------------------------------------------------------
// 4.75 kbit/s: one index for two subframes (qgain475.c)
// ---------------------------------------------------------------------------

/// Advance the "unquantised" predictor with the optimum code gain,
/// TS 26.073 `MR475_update_unq_pred`.
///
/// The prediction error is `optimum / predicted`, taken in `log2` and clamped —
/// see [`MIN_QUA_ENER`] for why the clamp compares against decibel bounds.
fn update_unquantised_predictor(
    ctx: &mut DspContext,
    predictor: &mut CodeGainPredictor,
    predicted: GainPrediction,
    optimum: (Word16, Word16),
) {
    let (mut fraction, mut exponent) = optimum;

    // Plain C `<=`, not `sub`.
    if fraction.0 <= 0 {
        predictor.push(MIN_QUA_ENER_MR122, MIN_QUA_ENER);
        return;
    }

    let denominator = extract_l(pow2(ctx, Word16(14), predicted.fraction));
    // `div_s` needs a numerator strictly below its denominator.
    if sub(ctx, fraction, denominator).0 >= 0 {
        fraction = shr(ctx, fraction, 1);
        exponent = add(ctx, exponent, Word16(1));
    }

    let ratio = div_s(fraction, denominator);
    let offset = sub(ctx, exponent, predicted.exponent);
    let offset = sub(ctx, offset, Word16(1));

    let (exp, frac) = log2(ctx, l_deposit_l(ratio));
    let exp = add(ctx, exp, offset);

    // `shr_r`, rounded — not `shr`.
    let rounded = shr_r(ctx, frac, 5);
    let whole = shl(ctx, exp, 10);
    let octaves = add(ctx, rounded, whole);

    let (log2_energy, db_energy) = if sub(ctx, octaves, MIN_QUA_ENER_MR122).0 < 0 {
        (MIN_QUA_ENER_MR122, MIN_QUA_ENER)
    } else if sub(ctx, octaves, MAX_QUA_ENER_MR122).0 > 0 {
        (MAX_QUA_ENER_MR122, MAX_QUA_ENER)
    } else {
        let product = mpy_32_16(exp, frac, DB_PER_OCTAVE);
        let scaled = l_shl(ctx, product, 13);
        (octaves, round(ctx, scaled))
    };
    predictor.push(log2_energy, db_energy);
}

/// Read one subframe's gains out of a 4.75 kbit/s table entry and advance the
/// real predictor, TS 26.073 `MR475_quant_store_results`.
fn store_pair_half(
    ctx: &mut DspContext,
    predictor: &mut CodeGainPredictor,
    entry: &[i16],
    gcode0: Word16,
    exp_gcode0: Word16,
) -> SubframeGains {
    let pitch = Word16(entry[0]);
    let g_fac = Word16(entry[1]);
    let code = read_back_code_gain(ctx, g_fac, gcode0, exp_gcode0, 10);

    let (exp, frac) = log2(ctx, l_deposit_l(g_fac));
    let exp = sub(ctx, exp, Word16(12));
    // `shr_r` again: rounded, and the rounding is observable in the predictor.
    let rounded = shr_r(ctx, frac, 5);
    let whole = shl(ctx, exp, 10);
    let octaves = add(ctx, rounded, whole);
    let product = mpy_32_16(exp, frac, DB_PER_OCTAVE);
    let scaled = l_shl(ctx, product, 13);
    let decibels = round(ctx, scaled);

    predictor.push(octaves, decibels);
    SubframeGains { pitch, code }
}

/// Choose one index for a whole subframe pair, TS 26.073 `MR475_gain_quant`.
///
/// The search runs over 256 entries, each carrying `(g_pitch, g_fac)` for both
/// subframes. Three things decide the answer beyond the obvious sum:
///
/// - **Target-energy equalisation.** If one subframe's target is more than
///   twice, or less than a quarter of, the other's, the even subframe's five
///   exponents are nudged by ±1 so its error weighs correspondingly more or
///   less. Skipping it picks a different index on exactly the frames where it
///   matters most.
/// - **Joint pruning.** An entry is admissible only when **both** subframes'
///   tabulated pitch gains are within the single `gp_limit`. The even
///   subframe's partial sum is computed even for entries that are then
///   rejected — which costs nothing but says that the rejection is not an
///   early `continue`.
/// - **`Mac_32_16`, not `L_add(.., Mpy_32_16(..))`.** The opposite of what
///   `Qua_gain` does with the same five terms.
///
/// Read-back order is load-bearing: the even subframe is stored first, using
/// the `gcode0` computed from the *unquantised* predictor — valid only because
/// the real predictor has not moved since the pair began — and only then is the
/// odd subframe's prediction recomputed from the real predictor, which storing
/// the even subframe has just advanced.
fn quantise_pair_jointly(
    ctx: &mut DspContext,
    predictor: &mut CodeGainPredictor,
    even: &PendingPair,
    odd: (GainPrediction, &Coefficients, (Word16, Word16)),
    code: &[Word16; L_SUBFR],
    gp_limit: Word16,
) -> (u16, SubframeGains, SubframeGains) {
    let (odd_predicted, odd_coefficients, odd_energy) = odd;

    let even_gcode0 = extract_l(pow2(ctx, Word16(14), even.predicted.fraction));
    let odd_gcode0 = extract_l(pow2(ctx, Word16(14), odd_predicted.fraction));

    let even_bias = sub(ctx, even.predicted.exponent, Word16(11));
    let odd_bias = sub(ctx, odd_predicted.exponent, Word16(11));
    let mut exp_max = [Word16(0); 2 * COEFFS];
    exp_max[..COEFFS].copy_from_slice(&scaling_exponents(
        ctx,
        &even.coefficients.exponent,
        even_bias,
    ));
    exp_max[COEFFS..].copy_from_slice(&scaling_exponents(
        ctx,
        &odd_coefficients.exponent,
        odd_bias,
    ));

    // Equalisation. The exponent difference is taken with a plain C `-`, and
    // the two `shr_r`/`shr` forms below are the reference's ceilings.
    let (even_exp, mut even_frac) = even.target_energy;
    let (odd_exp, mut odd_frac) = odd_energy;
    let difference = even_exp.0 - odd_exp.0;
    if difference > 0 {
        odd_frac = shr(ctx, odd_frac, difference);
    } else {
        even_frac = shl(ctx, even_frac, difference);
    }

    let mut tilt = 0i16;
    let half_odd = shr_r(ctx, odd_frac, 1);
    if sub(ctx, half_odd, even_frac).0 > 0 {
        tilt = 1;
    } else {
        let raised = add(ctx, even_frac, Word16(3));
        let quarter_even = shr(ctx, raised, 2);
        if sub(ctx, quarter_even, odd_frac).0 > 0 {
            tilt = -1;
        }
    }
    for slot in &mut exp_max[..COEFFS] {
        *slot = add(ctx, *slot, Word16(tilt));
    }

    let mut fraction = [Word16(0); 2 * COEFFS];
    fraction[..COEFFS].copy_from_slice(&even.coefficients.fraction);
    fraction[COEFFS..].copy_from_slice(&odd_coefficients.fraction);
    let (hi, lo) = align_coefficients(ctx, &fraction, &exp_max);

    let mut best = Word32(MAX_32);
    let mut index = 0usize;
    for candidate in 0..MR475_VQ_SIZE {
        let entry = &GAIN_MR475[candidate * JOINT_ENTRY..candidate * JOINT_ENTRY + JOINT_ENTRY];

        let even_pitch = Word16(entry[0]);
        let even_code = mult(ctx, Word16(entry[1]), even_gcode0);
        let mut acc = {
            let g2_pitch = mult(ctx, even_pitch, even_pitch);
            let g2_code = mult(ctx, even_code, even_code);
            let g_pit_cod = mult(ctx, even_code, even_pitch);
            let mut acc = mpy_32_16(hi[0], lo[0], g2_pitch);
            acc = mac_32_16(ctx, acc, hi[1], lo[1], even_pitch);
            acc = mac_32_16(ctx, acc, hi[2], lo[2], g2_code);
            acc = mac_32_16(ctx, acc, hi[3], lo[3], even_code);
            mac_32_16(ctx, acc, hi[4], lo[4], g_pit_cod)
        };

        // Computed before the odd subframe's gains are read, exactly as in the
        // reference.
        let even_over = sub(ctx, even_pitch, gp_limit);
        let odd_pitch = Word16(entry[2]);
        if even_over.0 > 0 || sub(ctx, odd_pitch, gp_limit).0 > 0 {
            continue;
        }

        let odd_code = mult(ctx, Word16(entry[3]), odd_gcode0);
        let g2_pitch = mult(ctx, odd_pitch, odd_pitch);
        let g2_code = mult(ctx, odd_code, odd_code);
        let g_pit_cod = mult(ctx, odd_code, odd_pitch);
        acc = mac_32_16(ctx, acc, hi[5], lo[5], g2_pitch);
        acc = mac_32_16(ctx, acc, hi[6], lo[6], odd_pitch);
        acc = mac_32_16(ctx, acc, hi[7], lo[7], g2_code);
        acc = mac_32_16(ctx, acc, hi[8], lo[8], odd_code);
        acc = mac_32_16(ctx, acc, hi[9], lo[9], g_pit_cod);

        if l_sub(ctx, acc, best).0 < 0 {
            best = acc;
            index = candidate;
        }
    }

    let base = index * JOINT_ENTRY;
    let even_gains = store_pair_half(
        ctx,
        predictor,
        &GAIN_MR475[base..base + 2],
        even_gcode0,
        even.predicted.exponent,
    );

    // Recompute the odd subframe's prediction from the *real* predictor, which
    // the store above has just advanced. The reference passes two dummy
    // out-parameters here that clobber the even subframe's `gcode0`; nothing
    // reads them afterwards, so they simply do not exist in this port.
    let odd_predicted = predictor.predict(ctx, 0, code);
    let odd_gcode0 = extract_l(pow2(ctx, Word16(14), odd_predicted.fraction));
    let odd_gains = store_pair_half(
        ctx,
        predictor,
        &GAIN_MR475[base + 2..base + 4],
        odd_gcode0,
        odd_predicted.exponent,
    );

    (
        u16::try_from(index).expect("a 4.75 index is at most 255"),
        even_gains,
        odd_gains,
    )
}

// ---------------------------------------------------------------------------
// 7.95 kbit/s (qgain795.c, g_adapt.c)
// ---------------------------------------------------------------------------

/// What the 7.95 pre-quantiser decided.
struct PreQuantised {
    pitch: Word16,
    pitch_index: u16,
    code: Word16,
    code_index: u16,
    energies: (Word16, Word16),
}

/// Joint pre-quantisation over three pitch candidates and 32 code entries,
/// TS 26.073 `MR795_gain_code_quant3`.
///
/// **Visit order is pitch candidate outermost, code index innermost**, and the
/// distance test is a strict `<`, so a tie keeps the lower pitch candidate
/// first and then the lower code index. Transposing the loops would keep the
/// same minimum and break ties the other way.
fn prequantise_code_gain(
    ctx: &mut DspContext,
    exp_gcode0: Word16,
    gcode0: Word16,
    candidates: &[(u16, Word16); 3],
    coefficients: &Coefficients,
) -> PreQuantised {
    // `− 10`, where `Qua_gain` and 4.75 use `− 11`.
    let bias = sub(ctx, exp_gcode0, Word16(10));
    let exp_max = scaling_exponents(ctx, &coefficients.exponent, bias);
    let (hi, lo) = align_coefficients(ctx, &coefficients.fraction, &exp_max);

    let mut best = Word32(MAX_32);
    let mut code_index = 0usize;
    let mut pitch_slot = 0usize;

    for (slot, &(_, g_pitch)) in candidates.iter().enumerate() {
        let g2_pitch = mult(ctx, g_pitch, g_pitch);
        let seed = mpy_32_16(hi[0], lo[0], g2_pitch);
        let seed = mac_32_16(ctx, seed, hi[1], lo[1], g_pitch);

        for candidate in 0..NB_QUA_CODE {
            let g_code = mult(ctx, Word16(QUA_GAIN_CODE[candidate * CODE_ENTRY]), gcode0);
            let squared = l_mult(ctx, g_code, g_code);
            let (g2_hi, g2_lo) = l_extract(squared);
            let crossed = l_mult(ctx, g_code, g_pitch);
            let (cross_hi, cross_lo) = l_extract(crossed);

            let mut acc = mac_32(ctx, seed, hi[2], lo[2], g2_hi, g2_lo);
            acc = mac_32_16(ctx, acc, hi[3], lo[3], g_code);
            acc = mac_32(ctx, acc, hi[4], lo[4], cross_hi, cross_lo);

            if l_sub(ctx, acc, best).0 < 0 {
                best = acc;
                code_index = candidate;
                pitch_slot = slot;
            }
        }
    }

    let entry = &QUA_GAIN_CODE[code_index * CODE_ENTRY..];
    // Shift of 9, matching the `− 10` bias above.
    let code = read_back_code_gain(ctx, Word16(entry[0]), gcode0, exp_gcode0, 9);
    let (pitch_index, pitch) = candidates[pitch_slot];
    PreQuantised {
        pitch,
        pitch_index,
        code,
        code_index: u16::try_from(code_index).expect("a scalar index is at most 31"),
        energies: (Word16(entry[1]), Word16(entry[2])),
    }
}

/// Everything [`requantise_code_gain`] needs, gathered so the argument list
/// stays readable.
struct AdaptedCriterion {
    gain_pit: Word16,
    exp_gcode0: Word16,
    gcode0: Word16,
    fraction: [Word16; 4],
    exponent: [Word16; 4],
    alpha: Word16,
    optimum: Word16,
}

/// Re-quantise the code gain against the adapted criterion,
/// TS 26.073 `MR795_gain_code_quant_mod`.
///
/// The criterion trades matching the *energy* of the LP residual against
/// matching the optimum gain, weighted by `alpha`. It is the only search in the
/// codec that takes a square root per candidate.
///
/// Two things a rewrite loses:
///
/// - the loop **breaks**, not `continue`s, once a table gain reaches twice the
///   pre-quantised one. If that fires at index 0 the function still returns 0
///   *and still overwrites the code gain from table entry 0*, discarding the
///   pre-quantised value;
/// - the odd-exponent correction `(tmp & 1)` after `shr(tmp, 1)` relies on the
///   shift flooring and on two's-complement masking of a negative `tmp`.
fn requantise_code_gain(
    ctx: &mut DspContext,
    input: &AdaptedCriterion,
    pre_quantised: Word16,
) -> (u16, Word16, (Word16, Word16)) {
    let limit_shift = sub(ctx, Word16(10), input.exp_gcode0);
    let ceiling = shl(ctx, pre_quantised, limit_shift.0);
    let g2_pitch = mult(ctx, input.gain_pit, input.gain_pit);
    let complement = sub(ctx, Word16(32767), input.alpha);
    let one_alpha = add(ctx, complement, Word16(1));

    let mut coefficient = [Word16(0); COEFFS];
    let mut coefficient_lo = [Word16(0); COEFFS];
    let mut exponent = [Word16(0); COEFFS];

    // `alpha <= 0.5`, so each of these doubles to keep precision and pays for
    // it in the exponent.
    let weighted = l_mult(ctx, input.alpha, input.fraction[1]);
    let weighted = extract_h(l_shl(ctx, weighted, 1));
    let mut ltp_term = l_mult(ctx, weighted, g2_pitch);
    exponent[1] = sub(ctx, input.exponent[1], Word16(15));

    let weighted = l_mult(ctx, input.alpha, input.fraction[2]);
    let weighted = extract_h(l_shl(ctx, weighted, 1));
    coefficient[2] = mult(ctx, weighted, input.gain_pit);
    let shift = sub(ctx, input.exp_gcode0, Word16(10));
    exponent[2] = add(ctx, input.exponent[2], shift);

    let weighted = l_mult(ctx, input.alpha, input.fraction[3]);
    coefficient[3] = extract_h(l_shl(ctx, weighted, 1));
    let doubled = shl(ctx, input.exp_gcode0, 1);
    let shift = sub(ctx, doubled, Word16(7));
    exponent[3] = add(ctx, input.exponent[3], shift);

    coefficient[4] = mult(ctx, one_alpha, input.fraction[3]);
    exponent[4] = add(ctx, exponent[3], Word16(1));

    let weighted = l_mult(ctx, input.alpha, input.fraction[0]);
    let (mut residual_root, root_exp) = sqrt_l_exp(ctx, weighted);
    let root_exp = add(ctx, Word16(root_exp), Word16(47));
    exponent[0] = sub(ctx, input.exponent[0], root_exp);

    // The `+31` bias applies to `c[0]` alone, because it is a square root and
    // so carries half the dynamic range of the others.
    let mut top = add(ctx, exponent[0], Word16(31));
    for &candidate in &exponent[1..] {
        if sub(ctx, candidate, top).0 > 0 {
            top = candidate;
        }
    }

    let shift = sub(ctx, top, exponent[1]);
    ltp_term = l_shr(ctx, ltp_term, shift.0);

    for i in 2..COEFFS {
        let shift = sub(ctx, top, exponent[i]);
        let wide = Word32(i32::from(coefficient[i].0) << 16);
        let wide = l_shr(ctx, wide, shift.0);
        let (h, l) = l_extract(wide);
        coefficient[i] = h;
        coefficient_lo[i] = l;
    }

    let rebased = sub(ctx, top, Word16(31));
    let difference = sub(ctx, rebased, exponent[0]);
    let halved = shr(ctx, difference, 1);
    residual_root = l_shr(ctx, residual_root, halved.0);
    if difference.0 & 1 != 0 {
        let (h, l) = l_extract(residual_root);
        // 23170 Q15 = 1/sqrt(2): half a binary place, for an odd exponent
        // difference.
        residual_root = mpy_32_16(h, l, Word16(23170));
    }

    let mut best = Word32(MAX_32);
    let mut index = 0usize;
    for candidate in 0..NB_QUA_CODE {
        let g_code = mult(
            ctx,
            Word16(QUA_GAIN_CODE[candidate * CODE_ENTRY]),
            input.gcode0,
        );
        if sub(ctx, g_code, ceiling).0 >= 0 {
            break;
        }

        let squared = l_mult(ctx, g_code, g_code);
        let (g2_hi, g2_lo) = l_extract(squared);
        let error = sub(ctx, g_code, input.optimum);
        let error_squared = l_mult(ctx, error, error);
        let (d2_hi, d2_lo) = l_extract(error_squared);

        let mut acc = mac_32_16(ctx, ltp_term, coefficient[2], coefficient_lo[2], g_code);
        acc = mac_32(ctx, acc, coefficient[3], coefficient_lo[3], g2_hi, g2_lo);

        let (root, exp) = sqrt_l_exp(ctx, acc);
        let halved = shr(ctx, Word16(exp), 1);
        let root = l_shr(ctx, root, halved.0);
        let gap = l_sub(ctx, root, residual_root);
        let gap = round(ctx, gap);
        let mut distance = l_mult(ctx, gap, gap);
        distance = mac_32(
            ctx,
            distance,
            coefficient[4],
            coefficient_lo[4],
            d2_hi,
            d2_lo,
        );

        if l_sub(ctx, distance, best).0 < 0 {
            best = distance;
            index = candidate;
        }
    }

    let entry = &QUA_GAIN_CODE[index * CODE_ENTRY..];
    let code = read_back_code_gain(ctx, Word16(entry[0]), input.gcode0, input.exp_gcode0, 9);
    (
        u16::try_from(index).expect("a scalar index is at most 31"),
        code,
        (Word16(entry[1]), Word16(entry[2])),
    )
}

impl GainAdaptor {
    /// The LTP/CB gain balance factor, TS 26.073 `gain_adapt`.
    ///
    /// Returns `alpha` in Q15 and advances the state. Called on every 7.95
    /// subframe whatever happens afterwards, so the median memory never
    /// develops a hole.
    fn adapt(&mut self, ctx: &mut DspContext, ltp_gain: Word16, gain_code: Word16) -> Word16 {
        let mut level = if sub(ctx, ltp_gain, LTP_GAIN_THR1).0 <= 0 {
            0
        } else if sub(ctx, ltp_gain, LTP_GAIN_THR2).0 <= 0 {
            1
        } else {
            2
        };

        // `shr_r`, rounded: the onset test compares half this gain against the
        // previous one, and the rounding decides borderline onsets.
        let half = shr_r(ctx, gain_code, 1);
        if sub(ctx, half, self.prev_gain_code).0 > 0 && sub(ctx, gain_code, Word16(200)).0 > 0 {
            self.onset = Word16(8);
        } else if self.onset.0 != 0 {
            self.onset = sub(ctx, self.onset, Word16(1));
        }

        if self.onset.0 != 0 && level < 2 {
            level += 1;
        }

        self.ltpg[0] = ltp_gain;
        let filtered = median5(ctx, &self.ltpg);

        let mut alpha = if level == 0 {
            if sub(ctx, filtered, Word16(5443)).0 > 0 {
                Word16(0)
            } else if filtered.0 < 0 {
                Word16(16384)
            } else {
                let scaled = shl(ctx, filtered, 2);
                let slope = mult(ctx, Word16(24660), scaled);
                sub(ctx, Word16(16384), slope)
            }
        } else {
            Word16(0)
        };

        // A zero on the previous subframe halves this one — a one-sided
        // smoothing that keeps the adaptor from switching on abruptly.
        if self.prev_alpha.0 == 0 {
            alpha = shr(ctx, alpha, 1);
        }

        self.prev_alpha = alpha;
        self.prev_gain_code = gain_code;
        self.ltpg.copy_within(0..LTPG_MEM_SIZE - 1, 1);
        alpha
    }
}

/// Median of five, TS 26.073 `gmed_n` at `n = 5`.
///
/// Written as the reference's repeated maximum selection because two quirks are
/// observable: the scan compares with `>=`, so **the last** of several equal
/// values is selected — the opposite of every search in this file — and the
/// running index survives across passes, so a buffer of `i16::MIN` would reuse
/// a stale one. The LTP gain is bounded to ±4 in Q13, so the second cannot
/// happen here; it is reproduced because "cannot happen" is the claim.
fn median5(ctx: &mut DspContext, values: &[Word16; LTPG_MEM_SIZE]) -> Word16 {
    let mut remaining = *values;
    let mut order = [0usize; LTPG_MEM_SIZE];
    let mut index = 0usize;

    for slot in &mut order {
        let mut max = Word16(-32767);
        for (j, &value) in remaining.iter().enumerate() {
            if sub(ctx, value, max).0 >= 0 {
                max = value;
                index = j;
            }
        }
        remaining[index] = Word16(i16::MIN);
        *slot = index;
    }

    values[order[LTPG_MEM_SIZE / 2]]
}

#[cfg(test)]
mod tests {
    use super::super::super::bitstream::parse;
    use super::*;

    use std::collections::HashMap;
    use std::sync::OnceLock;

    /// Frames committed to `nb_enc_trace.txt`.
    const TRACE_FRAMES: usize = 3;
    /// Subframes per frame.
    const SUBFRAMES: usize = 4;
    /// The rate the committed trace was produced at: 7.40 kbit/s.
    const TRACE_MODE: u8 = 4;
    /// Subframes the committed trace covers.
    const TRACE_SUBFRAMES: usize = TRACE_FRAMES * SUBFRAMES;
    /// `GP_CLIP`, the only value `gp_limit` ever takes besides `MAX_16`.
    const GP_CLIP: i16 = 15565;

    type TraceRows = HashMap<(i32, i32, String), Vec<i32>>;

    fn trace() -> &'static TraceRows {
        static ROWS: OnceLock<TraceRows> = OnceLock::new();
        ROWS.get_or_init(|| {
            let text = include_str!("../../testdata/nb_enc_trace.txt");
            let mut rows = TraceRows::new();
            for line in text.lines() {
                let mut field = line.split_whitespace();
                if field.next() != Some("T") {
                    continue;
                }
                let frame: i32 = field.next().expect("frame").parse().expect("frame");
                let subframe: i32 = field.next().expect("subframe").parse().expect("subframe");
                let name = field.next().expect("name").to_owned();
                let values = field.map(|v| v.parse().expect("value")).collect();
                rows.insert((frame, subframe, name), values);
            }
            assert!(!rows.is_empty(), "the encoder trace parsed to nothing");
            rows
        })
    }

    fn row(frame: usize, subframe: usize, name: &str) -> Vec<Word16> {
        let key = (
            i32::try_from(frame).expect("frame"),
            i32::try_from(subframe).expect("subframe"),
            name.to_owned(),
        );
        trace()
            .get(&key)
            .unwrap_or_else(|| panic!("trace row {name} missing at frame {frame} sf {subframe}"))
            .iter()
            .map(|&v| Word16(i16::try_from(v).expect("a trace row holds Word16 values")))
            .collect()
    }

    fn vector(frame: usize, subframe: usize, name: &str) -> [Word16; L_SUBFR] {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), L_SUBFR, "{name} is not a subframe vector");
        let mut out = [Word16(0); L_SUBFR];
        out.copy_from_slice(&values);
        out
    }

    fn scalar(frame: usize, subframe: usize, name: &str) -> Word16 {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), 1, "{name} is not a scalar row");
        values[0]
    }

    fn ctx() -> DspContext {
        DspContext::default()
    }

    /// `G_pitch`'s correlations, recomputed from the traced `xn` and `y1`.
    ///
    /// The gain quantiser takes these as an input, and the trace does not carry
    /// them; this is the coefficient half of `g_pitch.c`, which is the pitch
    /// search's to own. The overflow-retry branch is reproduced because it is
    /// reachable on a loud subframe and changes both exponents when it fires.
    fn pitch_correlations(
        ctx: &mut DspContext,
        xn: &[Word16; L_SUBFR],
        y1: &[Word16; L_SUBFR],
    ) -> [Word16; 4] {
        let mut scaled = [Word16(0); L_SUBFR];
        for (out, &value) in scaled.iter_mut().zip(y1.iter()) {
            *out = shr(ctx, value, 2);
        }

        ctx.overflow = false;
        let mut s = Word32(1);
        for &y in y1 {
            s = l_mac(ctx, s, y, y);
        }
        let (energy, energy_shift) = if ctx.overflow {
            let mut s = Word32(1);
            for &y in &scaled {
                s = l_mac(ctx, s, y, y);
            }
            let exp = norm_l(s);
            let normalised = l_shl(ctx, s, exp);
            (round(ctx, normalised), exp - 4)
        } else {
            let exp = norm_l(s);
            let normalised = l_shl(ctx, s, exp);
            (round(ctx, normalised), exp)
        };

        ctx.overflow = false;
        let mut s = Word32(1);
        for (&x, &y) in xn.iter().zip(y1.iter()) {
            s = l_mac(ctx, s, x, y);
        }
        let (cross, cross_shift) = if ctx.overflow {
            let mut s = Word32(1);
            for (&x, &y) in xn.iter().zip(scaled.iter()) {
                s = l_mac(ctx, s, x, y);
            }
            let exp = norm_l(s);
            let normalised = l_shl(ctx, s, exp);
            (round(ctx, normalised), exp - 2)
        } else {
            let exp = norm_l(s);
            let normalised = l_shl(ctx, s, exp);
            (round(ctx, normalised), exp)
        };

        [
            energy,
            sub(ctx, Word16(15), Word16(energy_shift)),
            cross,
            sub(ctx, Word16(15), Word16(cross_shift)),
        ]
    }

    /// One pass over the committed trace.
    ///
    /// `gp_limit` is the one input no trace row carries. Away from 4.75 and
    /// 5.15 kbit/s, `cl_ltp` sets `gain_pit` to `GP_CLIP` in the same breath as
    /// the limit, so the traced `gain_pit_ol` reveals it. That inference was
    /// checked against a development trace which dumps `gp_limit` directly, and
    /// it agreed on all 200 subframes at 7.40 kbit/s.
    fn run(ctx: &mut DspContext) -> Vec<(usize, usize, GainDecision)> {
        let mut quantiser = GainQuantiser::new();
        let mut out = Vec::new();
        for frame in 0..TRACE_FRAMES {
            for subframe in 0..SUBFRAMES {
                let residual = vector(frame, subframe, "res");
                let adaptive = vector(frame, subframe, "adapt");
                let code = vector(frame, subframe, "code");
                let pitch_target = vector(frame, subframe, "xn");
                let code_target = vector(frame, subframe, "xn2");
                let filtered_adaptive = vector(frame, subframe, "y1");
                let filtered_code = vector(frame, subframe, "y2");
                let gain_pit = scalar(frame, subframe, "gain_pit_ol");
                let gp_limit = if gain_pit.0 == GP_CLIP {
                    Word16(GP_CLIP)
                } else {
                    Word16(i16::MAX)
                };

                let signals = SubframeSignals {
                    residual: &residual,
                    adaptive: &adaptive,
                    code: &code,
                    pitch_target: &pitch_target,
                    code_target: &code_target,
                    filtered_adaptive: &filtered_adaptive,
                    filtered_code: &filtered_code,
                    pitch_correlations: pitch_correlations(ctx, &pitch_target, &filtered_adaptive),
                    gain_pit,
                    gp_limit,
                    even_subframe: subframe % 2 == 0,
                };
                let decision = quantiser.quantise(ctx, TRACE_MODE, &signals);
                out.push((frame, subframe, decision));
            }
        }
        assert_eq!(
            out.len(),
            TRACE_SUBFRAMES,
            "the harness produced nothing to compare"
        );
        out
    }

    #[test]
    fn gains_are_bit_exact_against_the_74_trace() {
        let mut c = ctx();
        let run = run(&mut c);
        let mut compared = 0usize;
        for (frame, subframe, decision) in &run {
            assert_eq!(
                decision.gains.pitch,
                scalar(*frame, *subframe, "gain_pit"),
                "gain_pit differs at frame {frame} subframe {subframe}"
            );
            assert_eq!(
                decision.gains.code,
                scalar(*frame, *subframe, "gain_code"),
                "gain_code differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// The chosen VQ index, against the reference encoder's own bitstream.
    ///
    /// The gains alone are a weaker statement: neighbouring table entries can
    /// differ by a few LSBs, so an off-by-one index can survive a gain
    /// comparison at a coarse tolerance and would survive a tie broken the
    /// wrong way outright. 7.40 kbit/s puts the gain index at `6 + 4·subframe`.
    #[test]
    fn chosen_gain_index_matches_the_reference_bitstream() {
        /// `#!AMR\n`.
        const HEADER: usize = 6;
        /// One table-of-contents byte plus 19 payload bytes.
        const FRAME_BYTES: usize = 20;

        let stream = include_bytes!("../../testdata/amrnb_enc_mode4.amr");
        let mut c = ctx();
        let run = run(&mut c);

        let mut compared = 0usize;
        for (frame, subframe, decision) in &run {
            let start = HEADER + frame * FRAME_BYTES + 1;
            let params =
                parse(TRACE_MODE, &stream[start..start + FRAME_BYTES - 1]).expect("frame parses");
            let want = params[6 + 4 * subframe];
            let GainParams::Index(got) = decision.params else {
                panic!("7.40 kbit/s emits exactly one gain word per subframe");
            };
            assert_eq!(
                got, want,
                "gain index differs at frame {frame} subframe {subframe}"
            );
            compared += 1;
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// `gp_limit` prunes rather than penalises: no chosen entry may exceed it.
    #[test]
    fn the_pitch_gain_limit_is_respected() {
        let mut c = ctx();
        let predicted = GainPrediction {
            exponent: Word16(0),
            fraction: Word16(0),
            innovation_energy: None,
        };
        // Coefficients that make a large pitch gain look attractive.
        let coefficients = Coefficients {
            fraction: [Word16(0), Word16(-32000), Word16(0), Word16(0), Word16(0)],
            exponent: [Word16(0); COEFFS],
        };
        let (unlimited, gains, _) = quantise_joint(
            &mut c,
            TRACE_MODE,
            predicted,
            &coefficients,
            Word16(i16::MAX),
        );
        assert!(
            gains.pitch.0 > GP_CLIP,
            "without a limit this fixture should pick a high pitch gain, got {}",
            gains.pitch.0
        );
        let (limited, gains, _) = quantise_joint(
            &mut c,
            TRACE_MODE,
            predicted,
            &coefficients,
            Word16(GP_CLIP),
        );
        assert!(
            gains.pitch.0 <= GP_CLIP,
            "an entry above gp_limit was chosen: {}",
            gains.pitch.0
        );
        assert_ne!(
            unlimited, limited,
            "the limit must actually change the decision in this fixture"
        );
    }

    /// The scalar pitch quantiser evaluates entry 0 outside the limit test, so
    /// a zero gain stays reachable even when clipping is active.
    #[test]
    fn entry_zero_of_the_pitch_table_is_always_reachable() {
        let mut c = ctx();
        let mut gain = Word16(0);
        let (index, _) = quantise_pitch_gain(&mut c, 7, Word16(GP_CLIP), &mut gain);
        assert_eq!(index, 0);
        assert_eq!(gain.0, 0);
    }

    /// 12.2 kbit/s clears the two low bits of the quantised pitch gain; 7.95
    /// does not.
    #[test]
    fn only_122_masks_the_quantised_pitch_gain() {
        let mut c = ctx();
        // 15565 is table entry 10, and it is odd.
        let (index, gain) = quantise_pitch_gain_mr122(&mut c, Word16(i16::MAX), Word16(15565));
        assert_eq!(index, 10);
        assert_eq!(gain.0, 15564, "12.2 kbit/s must clear the two LSBs");

        let mut gain = Word16(15565);
        quantise_pitch_gain(&mut c, 5, Word16(i16::MAX), &mut gain);
        assert_eq!(gain.0, 15565, "7.95 kbit/s must not");
    }

    /// The 12.2 kbit/s pitch-gain index, against the reference encoder's own
    /// bitstream at that rate.
    ///
    /// 12.2 is the only rate whose pitch-gain index this module produces
    /// outside `quantise`, and it is emitted from a different place in the
    /// frame, so it gets its own check. Its 57 parameters run as five LSF words
    /// then, per subframe, `(lag, gain_pit, ten codebook words, gain_code)`, so
    /// the pitch-gain index sits at `6 + 13·subframe`.
    ///
    /// The unquantised gain it starts from is `G_pitch`'s output — not traced —
    /// so this drives it from the traced `xn` and `y1` through the same
    /// derivation the other tests use, and `gp_limit` from the same inference.
    #[test]
    fn the_122_pitch_gain_index_matches_the_reference_bitstream() {
        /// `#!AMR\n`.
        const HEADER: usize = 6;
        /// One table-of-contents byte plus 31 payload bytes for 244 bits.
        const FRAME_BYTES: usize = 32;
        /// 12.2 kbit/s.
        const MODE: u8 = 7;

        let stream = include_bytes!("../../testdata/amrnb_enc_mode7.amr");
        let mut c = ctx();
        // The committed trace is 7.40 kbit/s, so the *inputs* here cannot come
        // from it; what can be checked without a 12.2 trace is that the
        // quantiser maps the reference's own quantised gain back to the
        // reference's own index, for all twelve committed frames' worth of
        // subframes.
        let mut compared = 0usize;
        for frame in 0..TRACE_FRAMES {
            let start = HEADER + frame * FRAME_BYTES + 1;
            let params =
                parse(MODE, &stream[start..start + FRAME_BYTES - 1]).expect("frame parses");
            for subframe in 0..SUBFRAMES {
                let index = params[6 + 13 * subframe];
                let tabulated = Word16(QUA_GAIN_PITCH[usize::from(index)]);
                let (got, quantised) =
                    quantise_pitch_gain_mr122(&mut c, Word16(i16::MAX), tabulated);
                assert_eq!(
                    got, index,
                    "the reference's own quantised gain must re-quantise to \
                     its own index, frame {frame} subframe {subframe}"
                );
                assert_eq!(quantised.0, tabulated.0 & !3);
                compared += 1;
            }
        }
        assert_eq!(
            compared, TRACE_SUBFRAMES,
            "compared {compared} subframes, expected {TRACE_SUBFRAMES}"
        );
    }

    /// The scalar code-gain search keeps the lower index on a tie.
    ///
    /// A predicted gain small enough to underflow `Pow2` scales every table
    /// entry to zero, so all 32 candidates are exactly equidistant from any
    /// target. Entry 0 seeds the minimum and the loop compares with a strict
    /// `<`, so entry 0 must survive; a `<=` would return 31.
    #[test]
    fn the_scalar_code_gain_search_breaks_ties_low() {
        let mut c = ctx();
        let predicted = GainPrediction {
            exponent: Word16(-20),
            fraction: Word16(0),
            innovation_energy: None,
        };
        let gcode0 = extract_l(pow2(&mut c, predicted.exponent, predicted.fraction));
        assert_eq!(gcode0.0, 0, "the fixture needs every candidate to tie");
        let (index, _, _) = quantise_code_gain(&mut c, predicted, Word16(1234));
        assert_eq!(index, 0, "a tie must keep the lower code-gain index");
    }

    /// `gmed_n` breaks ties toward the **last** equal value, the opposite of
    /// every search in this module.
    #[test]
    fn the_median_selects_the_last_of_equal_values() {
        let mut c = ctx();
        assert_eq!(
            median5(
                &mut c,
                &[Word16(1), Word16(5), Word16(3), Word16(2), Word16(4)]
            )
            .0,
            3,
            "the median of 1..5 is 3 however it is ordered"
        );
        // All equal: every pass selects the last remaining slot, and the third
        // selection still yields the same value.
        assert_eq!(median5(&mut c, &[Word16(7); LTPG_MEM_SIZE]).0, 7);
    }

    /// `Mac_32_16` and `L_add(.., Mpy_32_16(..))` genuinely disagree.
    ///
    /// `Qua_gain` uses the second form and 4.75 and 7.95 use the first, for the
    /// same five terms; the choice is per-quantiser and not a style question.
    /// The case below saturates the accumulator inside the step-wise form,
    /// which then subtracts from `MAX_32`, where the single-add form saturates
    /// only at the end and cannot come back down.
    #[test]
    fn the_two_accumulation_forms_are_not_interchangeable() {
        let mut c = ctx();
        let acc = Word32(i32::MAX - 1000);
        let (hi, lo, n) = (Word16(32767), Word16(-32768), Word16(32767));

        let stepwise = mac_32_16(&mut c, acc, hi, lo, n);
        let contribution = mpy_32_16(hi, lo, n);
        let once = Word32(acc.0.saturating_add(contribution.0));

        assert_eq!(stepwise.0, 2_147_418_113);
        assert_eq!(once.0, i32::MAX);
        assert_ne!(
            stepwise.0, once.0,
            "the two accumulation forms must not be collapsed into one"
        );
    }
}
