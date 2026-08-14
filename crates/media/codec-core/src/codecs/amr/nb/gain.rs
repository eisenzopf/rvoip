//! AMR-NB gain decoding and concealment, 3GPP TS 26.090 §5.7 / §6.1 as
//! specified by the TS 26.073 fixed-point reference.
//!
//! Implements the reference functions `Dec_gain` (`dec_gain.c`),
//! `d_gain_pitch` (`d_gain_p.c`), `d_gain_code` (`d_gain_c.c`), the MA
//! code-gain predictor `gc_pred` / `gc_pred_update` /
//! `gc_pred_average_limited` (`gc_pred.c`), the concealment generators
//! `ec_gain_pitch` / `ec_gain_code` and their `_update` companions
//! (`ec_gains.c`), `Cb_gain_average` (`c_g_aver.c`) and the median helper
//! `gmed_n` (`gmed_n.c`).
//!
//! Validated bit-exactly against the `gains`, `conceal` and `cbgainav`
//! sections of `testdata/nb_stages.txt`, produced by
//! `tools/amrnb_stage_oracle.c` driving TS 26.073's own functions.
//!
//! # Two gain paths, not one
//!
//! Six of the eight rates quantise the pitch and code gains *jointly*, as one
//! VQ index into a table of `(g_pitch, g_fac, …)` tuples, and decode both with
//! [`decode_joint`]. The other two — 7.95 and 12.2 kbit/s — quantise them
//! separately and use [`decode_pitch_gain`] plus [`decode_code_gain`]. The two
//! paths read different tables *with different scalings*, which is the single
//! most effective way to produce a decoder that sounds almost right:
//!
//! | | joint tables | split table |
//! |---|---|---|
//! | `g_fac` | Q12 | **Q11** |
//! | denormalising shift | `10 - exp` | **`9 - exp`** |
//!
//! One bit apart, and the error is a constant factor of two on the innovation —
//! audible as a wrong balance between the periodic and noise-like parts of the
//! excitation rather than as noise.
//!
//! # Q formats
//!
//! - pitch gain: **Q14** (16384 is 1.0; the quantiser tops out at 19661 = 1.2)
//! - code gain: **Q1**, at every rate including 12.2
//! - predictor history: **Q10** in two different scales at once —
//!   `20·log10(g)` for seven rates and `log2(g)` for 12.2, both kept on every
//!   frame regardless of the rate in use, because the rate can change per frame
//! - the innovation fed to the predictor: **Q12** at 12.2, **Q13** elsewhere
//!
//! # State the caller owns
//!
//! [`CodeGainPredictor`] survives comfort-noise frames; [`PitchGainConcealer`],
//! [`CodeGainConcealer`] and [`CodeGainSmoother`] do not. See
//! [`CodeGainSmoother::reset_for_comfort_noise`] and
//! [`CodeGainPredictor::reseed_from_sid`].

use super::decoder_tables::{
    GAIN_HIGHRATES, GAIN_LOWRATES, GAIN_MR475, QUA_GAIN_CODE, QUA_GAIN_PITCH,
};
use super::math::{log2, log2_norm, pow2};
use super::L_SUBFR;
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_deposit_l, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_comp, l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, norm_s, shl, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Rate indices in the reference's numeric order.
///
/// The order is load-bearing rather than cosmetic: `Cb_gain_average`'s mode
/// gate is `mode <= MR67`, an ordered comparison, so renumbering these would
/// silently change which rates smooth their code gain.
mod rate {
    /// 4.75 kbit/s.
    pub const R4_75: u8 = 0;
    /// 5.15 kbit/s.
    pub const R5_15: u8 = 1;
    /// 5.90 kbit/s.
    pub const R5_90: u8 = 2;
    /// 6.70 kbit/s.
    pub const R6_70: u8 = 3;
    /// 7.40 kbit/s.
    pub const R7_40: u8 = 4;
    /// 7.95 kbit/s.
    pub const R7_95: u8 = 5;
    /// 10.2 kbit/s.
    pub const R10_2: u8 = 6;
    /// 12.2 kbit/s.
    pub const R12_2: u8 = 7;
}

use rate::{R10_2, R12_2, R4_75, R5_15, R5_90, R6_70, R7_40, R7_95};

/// Taps of the MA energy predictor.
const NPRED: usize = 4;

/// Prediction coefficients for every rate but 12.2, Q13.
///
/// Inline in `gc_pred.c` rather than in a `.tab` file, so the table generator
/// does not produce it.
const PRED_DB: [i16; NPRED] = [5571, 4751, 2785, 1556];

/// Prediction coefficients for 12.2 kbit/s, Q6 (`gc_pred.c`).
const PRED_LOG2: [i16; NPRED] = [44, 37, 22, 12];

/// Mean innovation energy at 12.2 kbit/s: `36 / (20·log10 2)` in Q17.
const MEAN_ENER_LOG2: i32 = 783_741;

/// Predictor floor, −14 dB in Q10.
const MIN_ENERGY_DB: i16 = -14336;

/// The same floor in the `log2` scale 12.2 kbit/s uses, Q10.
const MIN_ENERGY_LOG2: i16 = -2381;

/// Pitch-gain attenuation by concealment state, Q15 (`ec_gains.c`).
const PDOWN: [i16; 7] = [32767, 32112, 32112, 26214, 9830, 6553, 6553];

/// Code-gain attenuation by concealment state, Q15 (`ec_gains.c`).
///
/// Note how much flatter this is than [`PDOWN`]: a long erasure kills the
/// periodic part of the excitation while leaving most of the noise-like part,
/// which is what stops a lost burst from turning into a tone.
const CDOWN: [i16; 7] = [32767, 32112, 32112, 32112, 32112, 32112, 22937];

/// Gains kept in each concealment history.
const GAIN_BUFFER: usize = 5;

/// Code gains kept by [`CodeGainSmoother`] (`L_CBGAINHIST`).
const GAIN_HISTORY: usize = 7;

/// LP order, and so the length of the LSF vectors [`CodeGainSmoother`] compares.
const LSF_ORDER: usize = 10;

/// One subframe's decoded gains.
///
/// The two are separate Q formats and are easy to transpose, so they travel
/// named rather than as a bare pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubframeGains {
    /// Adaptive-codebook (pitch) gain, Q14 — 16384 is 1.0.
    pub pitch: Word16,
    /// Fixed-codebook (innovation) gain, Q1.
    pub code: Word16,
}

/// The predicted code gain, as a base-2 logarithm split into its parts.
///
/// It is kept split rather than evaluated because the two consumers want
/// different things from it: [`decode_joint`] folds the exponent into a shift,
/// while 12.2 kbit/s raises two to the whole thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GainPrediction {
    /// Integer part, Q0. Signed — a quiet frame predicts a gain below one.
    pub exponent: Word16,
    /// Fractional part, Q15, always non-negative.
    pub fraction: Word16,
    /// Innovation energy as `(exponent Q0, fraction Q15)`.
    ///
    /// Only 7.95 kbit/s computes it — it needs the energy for its own gain
    /// quantiser — so every other rate leaves this `None` rather than
    /// returning a value the reference never wrote.
    pub innovation_energy: Option<(Word16, Word16)>,
}

/// A per-frame condition together with its value on the preceding frame.
///
/// Concealment keys off both: a good frame that *follows* a bad one is limited
/// against the last known-good gain, where a good frame in a run of good frames
/// is not. Keeping the pair together makes it impossible to pass this frame's
/// flag where the previous frame's was meant.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WithPrevious {
    /// The flag for the frame being decoded.
    pub current: bool,
    /// The same flag for the frame before it.
    pub previous: bool,
}

impl WithPrevious {
    /// Set on either frame.
    #[must_use]
    pub const fn either(self) -> bool {
        self.current || self.previous
    }

    /// Set on both frames.
    #[must_use]
    pub const fn both(self) -> bool {
        self.current && self.previous
    }
}

/// Everything [`CodeGainSmoother`] needs to know about the frame's condition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameQuality {
    /// Bad-frame indication, this frame and the last.
    pub bad: WithPrevious,
    /// Potentially-degraded-frame indication, this frame and the last.
    pub degraded: WithPrevious,
    /// The background-noise detector's verdict for this frame.
    pub background_noise: bool,
    /// Frames elapsed since the last voiced frame.
    pub voiced_hangover: i16,
}

/// The MA predictor of the fixed-codebook gain, TS 26.073 `gc_predState`.
///
/// Four frames of quantised code-gain energy, kept in **two** scales at once:
/// `20·log10(g)` for seven of the rates and `log2(g)` for 12.2. Both are
/// updated on every subframe whatever the current rate, because the rate may
/// change from frame to frame and the predictor must be usable immediately.
///
/// This is the state that makes gain decoding history-dependent, and therefore
/// the state whose loss produces the characteristic slow divergence from the
/// encoder rather than an outright failure. It persists across subframes and
/// across frames, and — unlike the concealment states — it survives a
/// comfort-noise frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeGainPredictor {
    /// `20·log10(g)` of the last four quantised gains, Q10, newest first.
    past_db: [Word16; NPRED],
    /// `log2(g)` of the same four, Q10, newest first.
    past_log2: [Word16; NPRED],
}

impl Default for CodeGainPredictor {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGainPredictor {
    /// A predictor in its reset state.
    ///
    /// Seeded at the −14 dB floor rather than at zero: zero would mean "the
    /// last four frames were at 0 dB", which makes the first frames after a
    /// reset far too loud.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            past_db: [Word16(MIN_ENERGY_DB); NPRED],
            past_log2: [Word16(MIN_ENERGY_LOG2); NPRED],
        }
    }

    /// Predict the code gain from the innovation and the energy history,
    /// TS 26.073 `gc_pred`.
    ///
    /// `code` is the innovation **after pitch sharpening** — Q12 at 12.2
    /// kbit/s, Q13 at every other rate. Feeding the raw codevector here is a
    /// classic plausible-but-wrong bug: the output stays in range and the
    /// speech stays intelligible.
    ///
    /// Returns the predicted gain as `2^(exponent + fraction)`.
    #[must_use]
    pub fn predict(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        code: &[Word16; L_SUBFR],
    ) -> GainPrediction {
        // Sum of squares. The reference seeds this with `L_mac(0, c0, c0)` and
        // loops from one; folding from zero is the same word for every input,
        // since `L_add(0, x)` cannot saturate.
        //
        // The accumulator itself *can* saturate, and that is not an edge case
        // to guard against: a fully saturated energy is a legal state and the
        // normalisation below is defined on it.
        let mut energy = Word32(0);
        for &sample in code {
            energy = l_mac(ctx, energy, sample, sample);
        }

        if mode_index == R12_2 {
            self.predict_log2_domain(ctx, energy)
        } else {
            self.predict_db_domain(ctx, mode_index, energy)
        }
    }

    /// The 12.2 kbit/s branch of `gc_pred`, which works in `log2` throughout.
    fn predict_log2_domain(&self, ctx: &mut DspContext, energy: Word32) -> GainPrediction {
        // Q25 rounded down to Q9, then scaled by 1/40 (26214 in Q20) -> Q30.
        let rounded = round(ctx, energy);
        let mean_energy = l_mult(ctx, rounded, Word16(26214));
        let (exponent, fraction) = log2(ctx, mean_energy);

        // `L_Comp` packs the pair back into one word. Read as Q16 that word is
        // log2(energy); read as Q17 it is half of it, which is what the
        // prediction wants. No further shift — the reinterpretation *is* the
        // halving.
        let measured = l_comp(sub(ctx, exponent, Word16(30)), fraction);

        let mut predicted = Word32(MEAN_ENER_LOG2);
        for (&past, &coeff) in self.past_log2.iter().zip(PRED_LOG2.iter()) {
            predicted = l_mac(ctx, predicted, past, Word16(coeff));
        }

        let surplus = l_sub(ctx, predicted, measured);
        let excess = l_shr(ctx, surplus, 1);
        let (exponent, fraction) = l_extract(excess);
        GainPrediction {
            exponent,
            fraction,
            innovation_energy: None,
        }
    }

    /// The branch every other rate takes, which works in decibels.
    fn predict_db_domain(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        energy: Word32,
    ) -> GainPrediction {
        let norm = norm_l(energy);
        let normalised = l_shl(ctx, energy, norm);
        let (exponent, fraction) = log2_norm(ctx, normalised, norm);

        // −24660 is −10/log2(10) = −3.01 in Q13; negative, because this is
        // `means_ener − 10·log10(energy)`.
        let mut acc = mpy_32_16(exponent, fraction, Word16(-24660));

        // 7.95 is the only rate that wants the innovation energy back out, and
        // it wants it from the *normalised* accumulator, not the raw sum.
        let innovation_energy = (mode_index == R7_95)
            .then(|| (sub(ctx, Word16(-11), Word16(norm)), extract_h(normalised)));

        // K = means_ener + fact·27 + 10·log10(40), Q14. The pair is an `L_mac`
        // operand pair, not a product to be pre-multiplied: 32588 and 32268 do
        // not fit the ×64 form, which is why two rates use ×32 instead.
        let (k, k_scale) = match mode_index {
            R7_40 => (32588, 32), // 30 dB
            R6_70 => (32268, 32), // 28.75 dB
            R7_95 => (17062, 64), // 36 dB
            _ => (16678, 64),     // 33 dB: 4.75, 5.15, 5.90, 10.2
        };
        acc = l_mac(ctx, acc, Word16(k), Word16(k_scale));

        // Q14 -> Q24, saturating; the saturation is reachable on loud frames
        // and is part of the defined output.
        acc = l_shl(ctx, acc, 10);
        for (&coeff, &past) in PRED_DB.iter().zip(self.past_db.iter()) {
            acc = l_mac(ctx, acc, Word16(coeff), past);
        }
        let gcode0 = extract_h(acc); // Q8

        // 5443 is 1/(20·log10 2) in Q15. 7.40 kbit/s uses 5439 instead — a
        // deliberately imprecise constant inherited from IS-641, which the
        // reference keeps for bit-exactness with it. Unifying the two diverges
        // that whole rate.
        let scale = if mode_index == R7_40 { 5439 } else { 5443 };
        let scaled = l_mult(ctx, gcode0, Word16(scale)); // Q8 · Q15 -> Q24
        let acc = l_shr(ctx, scaled, 8); // -> Q16

        let (exponent, fraction) = l_extract(acc);
        GainPrediction {
            exponent,
            fraction,
            innovation_energy,
        }
    }

    /// Push one subframe's quantised energy, TS 26.073 `gc_pred_update`.
    ///
    /// `log2_energy` is `log2(g)` and `db_energy` is `20·log10(g)`, both Q10.
    /// Both histories shift together whatever the current rate — see the type
    /// documentation.
    ///
    /// Called once per subframe, on good frames and (via
    /// [`CodeGainConcealer::conceal`]) on bad ones.
    pub fn push(&mut self, log2_energy: Word16, db_energy: Word16) {
        self.past_db.copy_within(0..NPRED - 1, 1);
        self.past_log2.copy_within(0..NPRED - 1, 1);
        self.past_log2[0] = log2_energy;
        self.past_db[0] = db_energy;
    }

    /// The floored mean of the history, TS 26.073 `gc_pred_average_limited`.
    ///
    /// Returns `(log2 scale, dB scale)`, both Q10. Used only by concealment,
    /// which feeds the result straight back in through [`Self::push`].
    ///
    /// The four values are summed with the **saturating 16-bit `add`** and only
    /// then scaled by a quarter. Averaging term by term instead would avoid the
    /// saturation and give a different — and wrong — answer whenever the
    /// history is loud.
    #[must_use]
    pub fn limited_average(&self, ctx: &mut DspContext) -> (Word16, Word16) {
        let mut log2_avg = Word16(0);
        for &past in &self.past_log2 {
            log2_avg = add(ctx, log2_avg, past);
        }
        log2_avg = mult(ctx, log2_avg, Word16(8192));
        if sub(ctx, log2_avg, Word16(MIN_ENERGY_LOG2)).0 < 0 {
            log2_avg = Word16(MIN_ENERGY_LOG2);
        }

        let mut db_avg = Word16(0);
        for &past in &self.past_db {
            db_avg = add(ctx, db_avg, past);
        }
        db_avg = mult(ctx, db_avg, Word16(8192));
        if sub(ctx, db_avg, Word16(MIN_ENERGY_DB)).0 < 0 {
            db_avg = Word16(MIN_ENERGY_DB);
        }

        (log2_avg, db_avg)
    }

    /// Overwrite both histories with one value each — the encoder's
    /// `dtx_enc` half of [`reseed_from_sid`](Self::reseed_from_sid).
    ///
    /// The decoder derives the two values from the SID's energy field; the
    /// encoder already holds them, having just computed the index. Same effect,
    /// different starting point, so they are separate entry points rather than
    /// one with a flag.
    pub const fn seed_directly(&mut self, db: Word16, log2: Word16) {
        self.past_db = [db; NPRED];
        self.past_log2 = [log2; NPRED];
    }

    /// Re-seed the history from a SID frame's logarithmic energy,
    /// TS 26.073 `dtx_dec.c`.
    ///
    /// A comfort-noise update does **not** reset the predictor to its floor —
    /// that would make the first speech frame after a silence far too quiet.
    /// It fills both histories from the SID's own level instead. `log_en` is
    /// the SID frame's decoded energy in Q10.
    ///
    /// Note that the reference derives the `log2`-scale value from the *already
    /// clamped* dB value with a single `mult`, so the two are not independently
    /// floored. (The comment above that line in the reference names the wrong
    /// array; the code is what is reproduced here.)
    pub fn reseed_from_sid(&mut self, ctx: &mut DspContext, log_en: Word16) {
        let half = shr(ctx, log_en, 1);
        let mut seed = sub(ctx, half, Word16(9000));
        if seed.0 > 0 {
            seed = Word16(0);
        }
        if sub(ctx, seed, Word16(-14436)).0 < 0 {
            seed = Word16(-14436);
        }
        self.past_db = [seed; NPRED];

        let seed = mult(ctx, Word16(5443), seed);
        self.past_log2 = [seed; NPRED];
    }
}

/// Decode a jointly quantised gain pair, TS 26.073 `Dec_gain`.
///
/// Used by 4.75, 5.15, 5.90, 6.70, 7.40 and 10.2 kbit/s. `code` is the
/// pitch-sharpened innovation in Q13; `index` is the VQ index from the
/// bitstream.
///
/// `even_subframe` is true on subframes 0 and 2. It matters only at 4.75
/// kbit/s, where one 8-bit index covers **two** subframes: the caller reads it
/// on the even subframe and passes the same value again on the odd one, and
/// this flag selects which half of the table entry to use. The odd subframe
/// still runs the whole function — prediction and history update included —
/// so skipping it desynchronises the predictor.
///
/// # Panics
///
/// If called for 7.95 or 12.2 kbit/s, which quantise the two gains separately
/// and must use [`decode_pitch_gain`] and [`decode_code_gain`]; or if `index`
/// is outside the rate's table.
pub fn decode_joint(
    ctx: &mut DspContext,
    predictor: &mut CodeGainPredictor,
    mode_index: u8,
    index: u16,
    code: &[Word16; L_SUBFR],
    even_subframe: bool,
) -> SubframeGains {
    assert!(
        mode_index != R7_95 && mode_index != R12_2,
        "{mode_index} quantises the gains separately; use decode_pitch_gain/decode_code_gain"
    );

    // Four Word16 per entry. Plain arithmetic rather than the reference's
    // saturating `shl`: the widest index is 4.75's 8 bits, so the shift cannot
    // saturate, and an out-of-range index should surface as a panic here
    // rather than silently clamp to the end of the table.
    let base = usize::from(index) * 4;

    // Read the quantised gains BEFORE predicting. The reference reuses one
    // `exp`/`frac` pair for both the table-derived logarithms and the
    // prediction, so hoisting the prediction above this would destroy the
    // 4.75 energies computed below.
    let (pitch, g_fac, log2_energy, db_energy) = match mode_index {
        R10_2 | R7_40 | R6_70 => {
            let entry = &GAIN_HIGHRATES[base..base + 4];
            (
                Word16(entry[0]),
                Word16(entry[1]),
                Word16(entry[2]),
                Word16(entry[3]),
            )
        }
        R4_75 => {
            // One entry is (g_pit, g_fac) for the even subframe followed by
            // (g_pit, g_fac) for the odd one.
            let half = base + if even_subframe { 0 } else { 2 };
            let pitch = Word16(GAIN_MR475[half]);
            let g_fac = Word16(GAIN_MR475[half + 1]);

            // 4.75's table carries no energy columns — they are derived, which
            // is how the book fits in 256 four-word entries instead of 256
            // six-word ones. Log2 of a Q12 value is log2(x) + 12.
            let (exponent, fraction) = log2(ctx, l_deposit_l(g_fac));
            let exponent = sub(ctx, exponent, Word16(12));

            // `shr_r` ROUNDS. Using a plain shift here is off by one LSB on
            // about half the table, and because the result feeds the predictor
            // the error accumulates instead of cancelling.
            let rounded_fraction = shr_r(ctx, fraction, 5); // Q15 -> Q10
            let whole = shl(ctx, exponent, 10); // Q0 -> Q10
            let log2_energy = add(ctx, rounded_fraction, whole);

            // 24660 is 20·log10(2) = 6.0206 in Q12, applied to (exp, frac) as
            // a double-precision pair holding log2(g_fac). Q13 -> Q26 -> Q10.
            let scaled = mpy_32_16(exponent, fraction, Word16(24660));
            let widened = l_shl(ctx, scaled, 13);
            let db_energy = round(ctx, widened);

            (pitch, g_fac, log2_energy, db_energy)
        }
        _ => {
            let entry = &GAIN_LOWRATES[base..base + 4];
            (
                Word16(entry[0]),
                Word16(entry[1]),
                Word16(entry[2]),
                Word16(entry[3]),
            )
        }
    };

    let prediction = predictor.predict(ctx, mode_index, code);

    // The literal 14 is not a typo for the predicted exponent: this asks for
    // 2^frac in Q14 and lets the shift below fold the exponent in. The result
    // is provably 16384..32767, so the truncating `extract_l` is a no-op here
    // (which it is emphatically not in `decode_code_gain`).
    let gcode0 = extract_l(pow2(ctx, Word16(14), prediction.fraction));

    let acc = l_mult(ctx, g_fac, gcode0); // Q12 · Q14 · 2 -> Q27

    // Folding in 2^exponent. `exp` is routinely above 10 on loud frames, making
    // this a saturating *left* shift; that saturation is the defined behaviour
    // and not something to guard against.
    let denorm = sub(ctx, Word16(10), prediction.exponent).0;
    let acc = l_shr(ctx, acc, denorm);
    let code_gain = extract_h(acc); // Q1

    predictor.push(log2_energy, db_energy);

    SubframeGains {
        pitch,
        code: code_gain,
    }
}

/// Decode a separately quantised pitch gain, TS 26.073 `d_gain_pitch`. Q14.
///
/// Used by 7.95 and 12.2 kbit/s, both of which transmit a 4-bit index into the
/// same 16-entry table. Stateless.
///
/// # Panics
///
/// If `index` is outside the 16-entry quantiser, or if called for a rate that
/// quantises the gains jointly.
#[must_use]
pub fn decode_pitch_gain(ctx: &mut DspContext, mode_index: u8, index: u16) -> Word16 {
    assert!(
        mode_index == R7_95 || mode_index == R12_2,
        "{mode_index} quantises the gains jointly; use decode_joint"
    );

    let gain = Word16(QUA_GAIN_PITCH[usize::from(index)]);
    if mode_index == R12_2 {
        // Clear the low two bits. Not an index-width matter — both rates send
        // four bits into this same table. 12.2 inherited the EFR, where the
        // pitch gain was Q12, and stays bit-exact with it only by discarding
        // the two bits of precision Q14 added.
        let dropped = shr(ctx, gain, 2);
        shl(ctx, dropped, 2)
    } else {
        gain
    }
}

/// Decode a separately quantised code gain, TS 26.073 `d_gain_code`. Q1.
///
/// Used by 7.95 and 12.2 kbit/s. `code` is the pitch-sharpened innovation —
/// Q12 at 12.2, Q13 at 7.95 — and `index` is the 5-bit gain index.
///
/// Note the order relative to [`decode_joint`]: here the prediction runs
/// *first* and the table read second. Both then update the predictor last.
///
/// # Panics
///
/// If `index` is outside the 32-entry quantiser, or if called for a rate that
/// quantises the gains jointly.
pub fn decode_code_gain(
    ctx: &mut DspContext,
    predictor: &mut CodeGainPredictor,
    mode_index: u8,
    index: u16,
    code: &[Word16; L_SUBFR],
) -> Word16 {
    assert!(
        mode_index == R7_95 || mode_index == R12_2,
        "{mode_index} quantises the gains jointly; use decode_joint"
    );

    let prediction = predictor.predict(ctx, mode_index, code);

    // Triples of (g_fac Q11, log2 energy Q10, dB energy Q10).
    let base = usize::from(index) * 3;
    let entry = &QUA_GAIN_CODE[base..base + 3];
    let g_fac = Word16(entry[0]);

    let gain = if mode_index == R12_2 {
        // The real exponent, so this is the predicted gain as a plain integer.
        // `extract_l` TRUNCATES the low sixteen bits — it does not saturate —
        // and `Pow2` can exceed sixteen bits here, so `gcode0` legitimately
        // wraps and may come out negative. A clamping cast would be a silent
        // bit-exactness bug on loud frames.
        let gcode0 = extract_l(pow2(ctx, prediction.exponent, prediction.fraction));
        let gcode0 = shl(ctx, gcode0, 4); // Q0 -> Q4, saturating, reachably so
        let scaled = mult(ctx, gcode0, g_fac); // Q4 · Q11 -> Q0
        shl(ctx, scaled, 1) // -> Q1
    } else {
        let gcode0 = extract_l(pow2(ctx, Word16(14), prediction.fraction));
        let acc = l_mult(ctx, g_fac, gcode0); // Q11 · Q14 · 2 -> Q26

        // Nine, not ten: this table's g_fac is Q11 where the joint tables
        // hold Q12. The two differ by exactly one bit and by nothing else.
        let denorm = sub(ctx, Word16(9), prediction.exponent).0;
        extract_h(l_shr(ctx, acc, denorm))
    };

    predictor.push(Word16(entry[1]), Word16(entry[2]));
    gain
}

/// Pitch-gain concealment state, TS 26.073 `ec_gain_pitchState`.
///
/// Wiped by a comfort-noise frame, unlike [`CodeGainPredictor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PitchGainConcealer {
    /// The last five gains actually used, Q14, oldest first.
    recent: [Word16; GAIN_BUFFER],
    /// The last gain, clamped to 1.0, Q14.
    last: Word16,
    /// The last gain from a *good* frame, Q14.
    last_good: Word16,
}

impl Default for PitchGainConcealer {
    fn default() -> Self {
        Self::new()
    }
}

impl PitchGainConcealer {
    /// A concealer in its reset state.
    ///
    /// The history is seeded low (1640, about 0.1) and `last_good` high
    /// (16384, exactly 1.0): an erasure before any speech has been decoded
    /// should produce almost no periodic excitation, but a good frame arriving
    /// after one should not be limited at all.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recent: [Word16(1640); GAIN_BUFFER],
            last: Word16(0),
            last_good: Word16(16384),
        }
    }

    /// Substitute a pitch gain for a lost subframe, TS 26.073 `ec_gain_pitch`.
    ///
    /// `state` is the frame-level bad-frame-handling state, 0..=6, constant
    /// across a frame's four subframes. Returns Q14.
    ///
    /// Unlike [`CodeGainConcealer::conceal`] this does **not** advance the MA
    /// predictor — the pitch gain is not predicted, so there is nothing to
    /// advance.
    ///
    /// # Panics
    ///
    /// If `state` exceeds 6.
    #[must_use]
    pub fn conceal(&self, ctx: &mut DspContext, state: u8) -> Word16 {
        // Median of the recent gains, then floored by the most recent one, so
        // a single loud subframe cannot be sustained through an erasure.
        let mut gain = median(ctx, &self.recent);
        if sub(ctx, gain, self.last).0 > 0 {
            gain = self.last;
        }
        mult(ctx, gain, Word16(PDOWN[usize::from(state)]))
    }

    /// Record the gain that was used, TS 26.073 `ec_gain_pitch_update`.
    ///
    /// Called after **every** pitch-gain decode or concealment, good frame or
    /// bad — skipping it on bad frames leaves the concealer's history full of
    /// stale gains. Returns the gain to actually use, which on the first good
    /// frame after an erasure may have been limited to the last known-good one.
    ///
    /// Two asymmetries against [`CodeGainConcealer::update`] that are both real:
    /// the *state* here is clamped to 1.0 while the returned gain is not, and
    /// it is the clamped value that enters the history.
    pub fn update(&mut self, ctx: &mut DspContext, bad: WithPrevious, gain: Word16) -> Word16 {
        let mut gain = gain;
        if !bad.current {
            if bad.previous && sub(ctx, gain, self.last_good).0 > 0 {
                gain = self.last_good;
            }
            // Only good frames define "last known good" — and only after the
            // limiting above.
            self.last_good = gain;
        }

        self.last = gain;
        if sub(ctx, self.last, Word16(16384)).0 > 0 {
            self.last = Word16(16384);
        }
        self.recent.copy_within(1..GAIN_BUFFER, 0);
        self.recent[GAIN_BUFFER - 1] = self.last;

        gain
    }
}

/// Code-gain concealment state, TS 26.073 `ec_gain_codeState`.
///
/// Wiped by a comfort-noise frame, unlike [`CodeGainPredictor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeGainConcealer {
    /// The last five gains actually used, Q1, oldest first.
    recent: [Word16; GAIN_BUFFER],
    /// The last gain, Q1. Not clamped, unlike the pitch concealer's.
    last: Word16,
    /// The last gain from a *good* frame, Q1.
    last_good: Word16,
}

impl Default for CodeGainConcealer {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeGainConcealer {
    /// A concealer in its reset state.
    ///
    /// Seeded at 1 in Q1 — effectively silence — rather than at the pitch
    /// concealer's 1640/16384. The asymmetry is deliberate: an erasure before
    /// any speech has been decoded should produce no innovation at all.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recent: [Word16(1); GAIN_BUFFER],
            last: Word16(0),
            last_good: Word16(1),
        }
    }

    /// Substitute a code gain for a lost subframe, TS 26.073 `ec_gain_code`.
    ///
    /// `state` is the frame-level bad-frame-handling state, 0..=6. Returns Q1.
    ///
    /// This **does** advance the MA predictor, with the floored average of the
    /// predictor's own history, so that a lost frame still rotates it and the
    /// first good frame afterwards predicts from four real entries. The average
    /// must be taken before the shift, which is why both happen here rather
    /// than being left to the caller.
    ///
    /// # Panics
    ///
    /// If `state` exceeds 6.
    pub fn conceal(
        &self,
        ctx: &mut DspContext,
        predictor: &mut CodeGainPredictor,
        state: u8,
    ) -> Word16 {
        let mut gain = median(ctx, &self.recent);
        if sub(ctx, gain, self.last).0 > 0 {
            gain = self.last;
        }
        let gain = mult(ctx, gain, Word16(CDOWN[usize::from(state)]));

        let (log2_avg, db_avg) = predictor.limited_average(ctx);
        predictor.push(log2_avg, db_avg);

        gain
    }

    /// Record the gain that was used, TS 26.073 `ec_gain_code_update`.
    ///
    /// Called after every code-gain decode or concealment. Returns the gain to
    /// actually use, limited to the last known-good gain on the first good
    /// frame after an erasure.
    ///
    /// No clamp anywhere, and the raw output — not a clamped copy — enters the
    /// history. Both differ from [`PitchGainConcealer::update`].
    pub fn update(&mut self, ctx: &mut DspContext, bad: WithPrevious, gain: Word16) -> Word16 {
        let mut gain = gain;
        if !bad.current {
            if bad.previous && sub(ctx, gain, self.last_good).0 > 0 {
                gain = self.last_good;
            }
            self.last_good = gain;
        }

        self.last = gain;
        self.recent.copy_within(1..GAIN_BUFFER, 0);
        self.recent[GAIN_BUFFER - 1] = gain;

        gain
    }
}

/// Code-gain smoothing across subframes, TS 26.073 `Cb_gain_averageState`.
///
/// In background noise the fixed-codebook gain jumps around from subframe to
/// subframe, which is audible as a fluttering noise floor. This mixes each
/// gain toward a running mean, by an amount that depends on how still the
/// spectrum is.
///
/// Wiped by a comfort-noise frame; see [`Self::reset_for_comfort_noise`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CodeGainSmoother {
    /// The last seven code gains, Q1, oldest first.
    history: [Word16; GAIN_HISTORY],
    /// Consecutive subframes of large spectral motion.
    hang_var: Word16,
    /// Subframes since the hangover last fired. Saturates rather than wrapping.
    hang_count: Word16,
}

impl CodeGainSmoother {
    /// A smoother in its reset state: an empty history and both counters zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            history: [Word16(0); GAIN_HISTORY],
            hang_var: Word16(0),
            hang_count: Word16(0),
        }
    }

    /// The state to enter the first speech frame after comfort noise with.
    ///
    /// Two things happen on a non-speech frame and both matter: the decoder's
    /// reset zeroes this whole struct, and the comfort-noise decoder then sets
    /// the hangover to 20. Leaving the history alone through a silence — the
    /// intuitive reading — diverges on the first frames after every
    /// comfort-noise period.
    pub const fn reset_for_comfort_noise(&mut self) {
        *self = Self::new();
        self.hang_var = Word16(20);
    }

    /// Mix one subframe's code gain toward the running mean,
    /// TS 26.073 `Cb_gain_average`. Q1 in, Q1 out.
    ///
    /// `lsf` is the interpolated LSF vector for this subframe and `lsf_avg` the
    /// running average, both Q15 and both strictly positive. (The reference
    /// names these parameters `lsp`/`lspAver`; the decoder passes LSFs. The
    /// positivity is not decoration — the division below has no defined result
    /// on a non-positive denominator.)
    ///
    /// Called once per subframe **for every rate**. The caller discards the
    /// result for 7.40, 7.95 and 12.2 — but it must still call: the history and
    /// both counters advance regardless, and an early return for those rates
    /// desynchronises a mixed-rate stream.
    ///
    /// # Panics
    ///
    /// If any `lsf_avg` entry is zero, which would make the normalised division
    /// undefined.
    pub fn smooth(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        gain_code: Word16,
        lsf: &[Word16; LSF_ORDER],
        lsf_avg: &[Word16; LSF_ORDER],
        quality: FrameQuality,
    ) -> Word16 {
        let mut mixed = gain_code;

        self.history.copy_within(1..GAIN_HISTORY, 0);
        self.history[GAIN_HISTORY - 1] = gain_code;

        let diff = spectral_motion(ctx, lsf, lsf_avg);

        // 5325 is 0.65 in Q13. (One of the two comments on this constant in
        // the reference says Q11; 0.65·2^13 = 5324.8, so Q13 is the reading
        // that reproduces the code.)
        self.hang_var = if sub(ctx, diff, Word16(5325)).0 > 0 {
            add(ctx, self.hang_var, Word16(1))
        } else {
            Word16(0)
        };
        if sub(ctx, self.hang_var, Word16(10)).0 > 0 {
            self.hang_count = Word16(0);
        }

        // 4.75, 5.15, 5.90, 6.70 and 10.2 smooth; 7.40, 7.95 and 12.2 fall
        // straight through with everything above already done.
        if mode_index <= R6_70 || mode_index == R10_2 {
            mixed = self.mix(ctx, mode_index, mixed, diff, quality);
        }

        // Outside the rate gate, and unconditional: once per subframe, so the
        // "< 40" test below is really "the first ten frames".
        self.hang_count = add(ctx, self.hang_count, Word16(1));
        mixed
    }

    /// The smoothing itself, for the rates that use it.
    fn mix(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        gain_code: Word16,
        diff: Word16,
        quality: FrameQuality,
    ) -> Word16 {
        // The stronger thresholds apply only at the three lowest rates, even
        // though 6.70 and 10.2 got through the outer gate.
        let lowest_three = matches!(mode_index, R4_75 | R5_15 | R5_90);
        let errors = quality.degraded.both() || quality.bad.either();
        let stronger = errors
            && sub(ctx, Word16(quality.voiced_hangover), Word16(1)).0 > 0
            && quality.background_noise
            && lowest_three;

        // 0.55 vs 0.40 in Q13: how much spectral motion is tolerated before
        // the gain stops being treated as background noise.
        let threshold = if stronger { 4506 } else { 3277 };

        let excess = sub(ctx, diff, Word16(threshold));
        let excess = if excess.0 > 0 { excess } else { Word16(0) };
        // 8192 is 1.0 in Q13 and means "no smoothing at all"; 2048 is 0.25,
        // the motion at which smoothing is fully off.
        let from_motion = if sub(ctx, Word16(2048), excess).0 < 0 {
            Word16(8192)
        } else {
            shl(ctx, excess, 2)
        };

        // No smoothing for the first forty subframes after the hangover reset,
        // nor while the spectrum is moving fast. Evaluated after the value
        // above, as in the reference, so the two tests see the same flags.
        let disabled =
            sub(ctx, self.hang_count, Word16(40)).0 < 0 || sub(ctx, diff, Word16(5325)).0 > 0;
        let bg_mix = if disabled { Word16(8192) } else { from_motion };

        // Five-tap mean over the newest five slots, 0.2 in Q15. The two oldest
        // are ignored unless the seven-tap branch below fires. Computed
        // unconditionally, as in the reference — the wider mean below discards
        // it, but the operator sequence is the same either way.
        let mut acc = l_mult(ctx, Word16(6554), self.history[2]);
        for &gain in &self.history[3..] {
            acc = l_mac(ctx, acc, Word16(6554), gain);
        }
        let five_tap = round(ctx, acc); // Q17 -> Q1

        // Under errors in background noise the three lowest rates widen to all
        // seven taps, 0.143 in Q15.
        let mean = if quality.bad.either() && quality.background_noise && lowest_three {
            let mut acc = l_mult(ctx, Word16(4681), self.history[0]);
            for &gain in &self.history[1..] {
                acc = l_mac(ctx, acc, Word16(4681), gain);
            }
            round(ctx, acc)
        } else {
            five_tap
        };

        // bg_mix·gain_code + (1 − bg_mix)·mean, Q13 against Q1.
        let mut acc = l_mult(ctx, bg_mix, gain_code);
        acc = l_mac(ctx, acc, Word16(8192), mean);
        acc = l_msu(ctx, acc, bg_mix, mean);
        let widened = l_shl(ctx, acc, 2); // Q15 -> Q17
        round(ctx, widened) // -> Q1
    }
}

/// How far this subframe's spectrum has moved from the running average, Q13.
///
/// A sum of ten per-coefficient relative differences, each computed by
/// normalising numerator and denominator into the fractional divider's domain
/// and undoing the normalisation afterwards.
///
/// # Panics
///
/// If any `lsf_avg` entry is zero — the division is undefined there, and the
/// reference simply aborts.
fn spectral_motion(
    ctx: &mut DspContext,
    lsf: &[Word16; LSF_ORDER],
    lsf_avg: &[Word16; LSF_ORDER],
) -> Word16 {
    let mut diff = Word16(0);

    for (&current, &average) in lsf.iter().zip(lsf_avg.iter()) {
        assert!(
            average.0 > 0,
            "the LSF average must be positive to divide by"
        );

        let gap = sub(ctx, average, current);
        let numerator = abs_s(ctx, gap);
        // The −1 is what guarantees numerator <= denominator, i.e. the
        // divider's precondition. It is not slack to be optimised away.
        let num_shift = sub(ctx, Word16(norm_s(numerator)), Word16(1)).0;
        let numerator = shl(ctx, numerator, num_shift);

        let den_shift = norm_s(average);
        let denominator = shl(ctx, average, den_shift);

        let ratio = div_s(numerator, denominator);

        // Undo both normalisations, plus two bits, landing in Q13.
        // `num_shift` is at least −1 and `den_shift` at most 15, so the
        // negation cannot reach the value where `negate` and unary minus
        // differ.
        let biased = add(ctx, Word16(2), Word16(num_shift));
        let shift = sub(ctx, biased, Word16(den_shift)).0;
        let ratio = if shift >= 0 {
            shr(ctx, ratio, shift)
        } else {
            shl(ctx, ratio, -shift)
        };

        diff = add(ctx, diff, ratio);
    }

    diff
}

/// Median of an odd-length gain history, TS 26.073 `gmed_n`.
///
/// Written as the reference's selection sort rather than as a real sort,
/// because two of its quirks are observable:
///
/// - the running index survives across outer iterations, so if every remaining
///   entry has already been consumed the inner scan selects nothing and the
///   *previous* iteration's index is reused. Reachable when the buffer holds
///   `i16::MIN`.
/// - the scan uses `>=`, so ties go to the last index, and the initial maximum
///   is −32767 rather than −32768.
///
/// Neither changes the answer for a well-behaved buffer, and both change it for
/// the buffers a damaged stream can produce.
///
/// # Panics
///
/// If `values` has an even length or more than nine entries, neither of which
/// the reference defines.
fn median(ctx: &mut DspContext, values: &[Word16]) -> Word16 {
    /// The reference's `NMAX`, and the size of its working buffers.
    const NMAX: usize = 9;

    let n = values.len();
    assert!(n % 2 == 1 && n <= NMAX, "gmed_n is defined for odd n <= 9");

    let mut remaining = [Word16(0); NMAX];
    remaining[..n].copy_from_slice(values);
    let mut rank = [0usize; NMAX];
    // Declared once, outside the loop, exactly as in the reference.
    let mut index = 0usize;

    for slot in &mut rank[..n] {
        let mut max = Word16(-32767);
        for (j, &value) in remaining[..n].iter().enumerate() {
            if sub(ctx, value, max).0 >= 0 {
                max = value;
                index = j;
            }
        }
        remaining[index] = Word16(i16::MIN);
        *slot = index;
    }

    // `shr(n, 1)` in the reference — a plain halving of a positive count.
    values[rank[n / 2]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::nb::vectors::{next_noise, rows, Row};

    fn ctx() -> DspContext {
        DspContext::default()
    }

    /// One draw from the oracle's stream, shifted and masked the way it does.
    ///
    /// The C casts the `Word16` draw to `unsigned` before shifting, so a
    /// negative draw becomes a large positive one and the shift is logical
    /// rather than arithmetic. Reproducing that cast is the whole point: a
    /// plain `>> 3` on the `i16` gives different indices for half the stream,
    /// and the failures would look like table bugs.
    fn draw_masked(seed: &mut i16, shift: u32, mask: u32) -> i32 {
        let value = next_noise(seed);
        i32::try_from((i32::from(value).cast_unsigned() >> shift) & mask).expect("masked draw fits")
    }

    /// One subframe of the oracle's pseudo-random innovation.
    fn draw_code(seed: &mut i16) -> [Word16; L_SUBFR] {
        let mut c = ctx();
        let mut code = [Word16(0); L_SUBFR];
        for sample in &mut code {
            // The oracle draws `shr(rnd(&seed), 4)`, the basic operator.
            *sample = shr(&mut c, Word16(next_noise(seed)), 4);
        }
        code
    }

    /// Index widths from `bitno.tab`. They are not uniform, and the tables are
    /// sized to them exactly.
    const fn joint_index_mask(mode_index: u8) -> u32 {
        match mode_index {
            R4_75 => 0xFF,
            R5_15 | R5_90 => 0x3F,
            _ => 0x7F,
        }
    }

    fn parse_tag(row: &Row, n: usize) -> i32 {
        row.tag(n).parse().expect("integer token")
    }

    #[test]
    fn joint_and_split_gain_decoding_are_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        let mut seed = 0i16;
        let mut split = false;
        let mut mode_index = 0u8;

        for row in rows("gains") {
            match row.label {
                "seq" => {
                    // `seq joint <mode> <seed>` / `seq split <mode> <seed>`.
                    split = match row.tag(0) {
                        "joint" => false,
                        "split" => true,
                        other => panic!("unknown gain path {other:?}"),
                    };
                    mode_index = u8::try_from(parse_tag(&row, 1)).expect("mode index");
                    seed = i16::try_from(parse_tag(&row, 2)).expect("seed");
                    // Every sequence is replayed from the reset state, which
                    // is the only way the four-frame history is exercised.
                    predictor = CodeGainPredictor::new();
                }
                "step" => {
                    let want = row.ints();
                    if split {
                        let pitch_index = draw_masked(&mut seed, 3, 0x0F);
                        let code_index = draw_masked(&mut seed, 3, 0x1F);
                        // Regenerating the indices rather than only reading
                        // them cross-checks the two generators: if they ever
                        // diverge, every replayed section would be comparing
                        // against inputs the reference never saw.
                        assert_eq!(pitch_index, want[0], "pitch index stream diverged");
                        assert_eq!(code_index, want[1], "code index stream diverged");

                        let pitch = decode_pitch_gain(
                            &mut c,
                            mode_index,
                            u16::try_from(pitch_index).expect("index"),
                        );
                        let code = draw_code(&mut seed);
                        let gain = decode_code_gain(
                            &mut c,
                            &mut predictor,
                            mode_index,
                            u16::try_from(code_index).expect("index"),
                            &code,
                        );

                        assert_eq!(
                            i32::from(pitch.0),
                            want[2],
                            "mode {mode_index} step {compared}: gain_pit"
                        );
                        assert_eq!(
                            i32::from(gain.0),
                            want[3],
                            "mode {mode_index} step {compared}: gain_cod"
                        );
                    } else {
                        let index = draw_masked(&mut seed, 3, joint_index_mask(mode_index));
                        assert_eq!(index, want[0], "joint index stream diverged");
                        let code = draw_code(&mut seed);

                        let gains = decode_joint(
                            &mut c,
                            &mut predictor,
                            mode_index,
                            u16::try_from(index).expect("index"),
                            &code,
                            want[1] != 0,
                        );

                        assert_eq!(
                            i32::from(gains.pitch.0),
                            want[2],
                            "mode {mode_index} step {compared}: gain_pit"
                        );
                        assert_eq!(
                            i32::from(gains.code.0),
                            want[3],
                            "mode {mode_index} step {compared}: gain_cod"
                        );
                    }
                    compared += 1;
                }
                other => panic!("unexpected row {other:?} in the gains section"),
            }
        }

        // Six jointly quantised rates and two split ones, 24 subframes each.
        assert_eq!(
            compared, 192,
            "compared {compared} gain steps, expected 192"
        );
    }

    #[test]
    fn gain_concealment_is_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let mut c = ctx();
        let mut pitch_state = PitchGainConcealer::new();
        let mut code_state = CodeGainConcealer::new();
        let mut predictor = CodeGainPredictor::new();
        let mut seed = 0i16;

        for row in rows("conceal") {
            match row.label {
                "seq" => seed = i16::try_from(parse_tag(&row, 0)).expect("seed"),
                "step" => {
                    let want = row.ints();
                    let bad = WithPrevious {
                        current: want[0] != 0,
                        previous: want[1] != 0,
                    };
                    let state = u8::try_from(want[2]).expect("concealment state");

                    let (pitch, code) = if bad.current {
                        (
                            pitch_state.conceal(&mut c, state),
                            code_state.conceal(&mut c, &mut predictor, state),
                        )
                    } else {
                        // A good frame draws its gains; a bad one draws
                        // nothing, so the stream position depends on the
                        // error pattern.
                        let pitch = draw_masked(&mut seed, 2, 0x3FFF);
                        let code = draw_masked(&mut seed, 3, 0x0FFF);
                        (
                            Word16(i16::try_from(pitch).expect("gain_pit")),
                            Word16(i16::try_from(code).expect("gain_code")),
                        )
                    };

                    // Unconditional after every decode or concealment.
                    let pitch = pitch_state.update(&mut c, bad, pitch);
                    let code = code_state.update(&mut c, bad, code);

                    assert_eq!(i32::from(pitch.0), want[3], "step {compared}: gain_pit");
                    assert_eq!(i32::from(code.0), want[4], "step {compared}: gain_code");
                    compared += 1;
                }
                other => panic!("unexpected row {other:?} in the conceal section"),
            }
        }

        assert_eq!(
            compared, 40,
            "compared {compared} conceal steps, expected 40"
        );
    }

    #[test]
    fn code_gain_smoothing_is_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let mut c = ctx();
        let mut smoother = CodeGainSmoother::new();
        let mut seed = 0i16;
        let mut lsf_avg = [Word16(0); LSF_ORDER];
        let mut lsf = [Word16(0); LSF_ORDER];
        let mut gain_code = 0i32;

        for row in rows("cbgainav") {
            match row.label {
                "seed" => seed = i16::try_from(parse_tag(&row, 0)).expect("seed"),
                "lspavg" => {
                    lsf_avg.copy_from_slice(&row.words());
                }
                "lsp" => {
                    // The oracle draws the gain *before* the LSFs but prints
                    // it after them, so the draw happens here to keep the
                    // stream in step and is checked against the step row.
                    gain_code = draw_masked(&mut seed, 3, 0x0FFF);
                    let want = row.i16s();
                    for (i, slot) in lsf.iter_mut().enumerate() {
                        let jitter = draw_masked(&mut seed, 9, 0x1F);
                        let value = 1900 + i32::try_from(i).expect("index") * 2510 + jitter;
                        *slot = Word16(i16::try_from(value).expect("lsf fits"));
                        assert_eq!(i32::from(slot.0), i32::from(want[i]), "lsf stream diverged");
                    }
                }
                "step" => {
                    // `step gain bfi prev_bf pdfi prev_pdf bgn hang -> out`.
                    let want: Vec<i32> = row
                        .parts()
                        .filter(|token| *token != "->")
                        .map(|token| token.parse().expect("integer token"))
                        .collect();
                    assert_eq!(want.len(), 8, "cbgainav step row shape");
                    assert_eq!(gain_code, want[0], "gain stream diverged");

                    let quality = FrameQuality {
                        bad: WithPrevious {
                            current: want[1] != 0,
                            previous: want[2] != 0,
                        },
                        degraded: WithPrevious {
                            current: want[3] != 0,
                            previous: want[4] != 0,
                        },
                        background_noise: want[5] != 0,
                        voiced_hangover: i16::try_from(want[6]).expect("hangover"),
                    };

                    // The oracle drives this at 6.70 kbit/s.
                    let got = smoother.smooth(
                        &mut c,
                        R6_70,
                        Word16(i16::try_from(gain_code).expect("gain fits")),
                        &lsf,
                        &lsf_avg,
                        quality,
                    );
                    assert_eq!(i32::from(got.0), want[7], "step {compared}: mixed gain");
                    compared += 1;
                }
                other => panic!("unexpected row {other:?} in the cbgainav section"),
            }
        }

        assert_eq!(
            compared, 12,
            "compared {compared} smoothing steps, expected 12"
        );
    }

    // ---------------------------------------------------------------------
    // Properties the oracle cannot reach.
    //
    // Every fixture above shares its assumptions with the code under test in
    // one respect: it was produced by calling the reference, so it can only
    // disagree where the two implementations disagree. What it cannot catch
    // is a stage the oracle never drove into an interesting state — and the
    // `cbgainav` vectors are exactly that case, since `hangCount` never
    // reaches 40 in twelve calls and every output is therefore the input
    // unchanged.
    // ---------------------------------------------------------------------

    #[test]
    fn the_smoothing_fixture_never_reaches_the_mixing_path() {
        // Pinning the blind spot rather than assuming it: if a regenerated
        // fixture ever does exercise the mix, this fails and the property
        // tests below can be retired in favour of the real vectors.
        let identical = rows("cbgainav")
            .iter()
            .filter(|row| row.label == "step")
            .all(|row| {
                let tokens: Vec<&str> = row.parts().collect();
                tokens[0] == tokens[tokens.len() - 1]
            });
        assert!(
            identical,
            "the cbgainav vectors now exercise mixing; test against them directly"
        );
    }

    #[test]
    fn a_still_spectrum_mixes_all_the_way_to_the_running_mean() {
        // With the LSFs sitting exactly on their average, the spectral motion
        // is zero, so the mix constant is zero and the output must be the
        // five-tap mean of the history alone. That pins the mixing arithmetic
        // the fixture never reaches, against an independently computed mean.
        let mut c = ctx();
        let mut smoother = CodeGainSmoother::new();
        let lsf = [
            Word16(2000),
            Word16(4500),
            Word16(7000),
            Word16(9500),
            Word16(12000),
            Word16(14500),
            Word16(17000),
            Word16(19500),
            Word16(22000),
            Word16(24500),
        ];
        let quality = FrameQuality::default();

        // Forty subframes get `hang_count` to the threshold; until then the
        // mix is disabled and the output is the input.
        for step in 0..40i16 {
            let gain = Word16(100 + step);
            let out = smoother.smooth(&mut c, R6_70, gain, &lsf, &lsf, quality);
            assert_eq!(out, gain, "step {step} should still pass through");
        }

        let gain = Word16(4000);
        let out = smoother.smooth(&mut c, R6_70, gain, &lsf, &lsf, quality);

        // The history now holds 134..139 followed by 4000; the five-tap mean
        // takes the newest five of the seven slots.
        let history: [i32; 5] = [136, 137, 138, 139, 4000];
        let mut acc = 0i32;
        for value in history {
            acc += 6554 * value * 2;
        }
        let want = i16::try_from((acc + 0x8000) >> 16).expect("mean fits");
        assert_eq!(out.0, want, "a zero mix constant must give the plain mean");
        assert_ne!(out, gain, "the mix did not engage");
    }

    #[test]
    fn smoothing_state_advances_for_the_rates_that_discard_the_result() {
        // 12.2 kbit/s throws the mixed gain away, but its history and
        // counters must still move; otherwise a stream that changes rate
        // diverges a few frames later.
        let mut c = ctx();
        let lsf = [Word16(2000); LSF_ORDER];
        let quality = FrameQuality::default();

        let mut smoothing = CodeGainSmoother::new();
        let mut discarding = CodeGainSmoother::new();
        for step in 0..8i16 {
            let gain = Word16(500 + step * 37);
            smoothing.smooth(&mut c, R6_70, gain, &lsf, &lsf, quality);
            let passed = discarding.smooth(&mut c, R12_2, gain, &lsf, &lsf, quality);
            assert_eq!(passed, gain, "12.2 must return its input unchanged");
        }

        assert_eq!(
            smoothing.history, discarding.history,
            "the gain history must advance for every rate"
        );
        assert_eq!(smoothing.hang_count, discarding.hang_count);
        assert_eq!(smoothing.hang_var, discarding.hang_var);
    }

    #[test]
    fn the_predictor_averages_before_scaling_not_after() {
        // The two orders differ exactly when the 16-bit sum saturates, which
        // is reachable and is the reference's defined behaviour. Averaging
        // term by term would give 30000 here instead of 8191.
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        for _ in 0..NPRED {
            predictor.push(Word16(30000), Word16(30000));
        }
        let (log2_avg, db_avg) = predictor.limited_average(&mut c);
        assert_eq!(log2_avg.0, 8191, "the sum must saturate before scaling");
        assert_eq!(db_avg.0, 8191);
    }

    #[test]
    fn the_reset_average_saturates_in_one_scale_and_not_the_other() {
        // Both histories start at the same floor expressed in two scales, and
        // the two behave differently under the saturating sum: four copies of
        // −14336 pin at −32768 and average to −8192, while four copies of
        // −2381 fit and average back to exactly −2381.
        //
        // So the reset average is *above* the dB floor, not at it — the
        // saturation is not an edge case to be avoided but the defined
        // behaviour, and a version that avoided it would start every
        // post-erasure frame at a different level.
        let mut c = ctx();
        let predictor = CodeGainPredictor::new();
        let (log2_avg, db_avg) = predictor.limited_average(&mut c);

        assert_eq!(
            log2_avg.0, MIN_ENERGY_LOG2,
            "the log2 sum must not saturate"
        );
        assert_eq!(db_avg.0, -8192, "the dB sum must saturate before scaling");
        assert!(
            db_avg.0 > MIN_ENERGY_DB,
            "the floor is never actually reached here"
        );
    }

    #[test]
    fn only_the_log2_floor_is_reachable_through_the_saturating_sum() {
        // Following the saturation through: the sum is capped at ±32768 and
        // the quarter-scaling then caps the average at [−8192, 8191]. So the
        // log2 floor of −2381 bites, and the dB floor of −14336 lies below
        // anything the average can produce and never does.
        //
        // That asymmetry is worth stating because it looks like a bug in both
        // directions: the dB clamp reads as dead code, and "fixing" the sum to
        // avoid saturating would make it live and change the output.
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        for _ in 0..NPRED {
            predictor.push(Word16(i16::MIN), Word16(i16::MIN));
        }
        let (log2_avg, db_avg) = predictor.limited_average(&mut c);
        assert_eq!(log2_avg.0, MIN_ENERGY_LOG2, "the log2 floor is reachable");
        assert_eq!(db_avg.0, -8192, "and is what the dB scale bottoms out at");

        for extreme in [i16::MIN, -20000, -14336, 0, 20000, i16::MAX] {
            let mut predictor = CodeGainPredictor::new();
            for _ in 0..NPRED {
                predictor.push(Word16(extreme), Word16(extreme));
            }
            let (_, db_avg) = predictor.limited_average(&mut c);
            // −8192 is the bound; it sits above MIN_ENERGY_DB, which is why
            // the dB clamp never fires.
            assert!(
                db_avg.0 >= -8192,
                "history {extreme} averaged to {}, below the -8192 the sum can reach \
                 and so below the {MIN_ENERGY_DB} floor",
                db_avg.0
            );
        }
    }

    #[test]
    fn the_predictor_history_is_newest_first_and_four_deep() {
        let mut predictor = CodeGainPredictor::new();
        for step in 1..=4i16 {
            predictor.push(Word16(step), Word16(step * 10));
        }
        assert_eq!(predictor.past_log2.map(|w| w.0), [4, 3, 2, 1]);
        assert_eq!(predictor.past_db.map(|w| w.0), [40, 30, 20, 10]);

        // A fifth push must drop the oldest, not grow the window.
        predictor.push(Word16(5), Word16(50));
        assert_eq!(predictor.past_log2.map(|w| w.0), [5, 4, 3, 2]);
    }

    #[test]
    fn the_pitch_concealer_clamps_its_state_but_not_its_output() {
        // The excitation wants the unclamped gain; the concealment history
        // wants it clamped to 1.0. Conflating the two is easy and quiet.
        let mut c = ctx();
        let mut state = PitchGainConcealer::new();
        let loud = Word16(20000);
        let returned = state.update(&mut c, WithPrevious::default(), loud);
        assert_eq!(returned, loud, "the returned gain must not be clamped");
        assert_eq!(state.last.0, 16384, "the state must be clamped to 1.0");
        assert_eq!(
            state.recent[GAIN_BUFFER - 1].0,
            16384,
            "the clamped value is what enters the history"
        );
    }

    #[test]
    fn the_code_concealer_clamps_nothing() {
        let mut c = ctx();
        let mut state = CodeGainConcealer::new();
        let loud = Word16(20000);
        let returned = state.update(&mut c, WithPrevious::default(), loud);
        assert_eq!(returned, loud);
        assert_eq!(state.last, loud, "the code gain state has no clamp");
        assert_eq!(state.recent[GAIN_BUFFER - 1], loud);
    }

    #[test]
    fn a_good_frame_after_a_bad_one_is_limited_but_a_run_of_good_ones_is_not() {
        let mut c = ctx();
        let mut state = PitchGainConcealer::new();

        // Establish a quiet known-good gain.
        state.update(
            &mut c,
            WithPrevious {
                current: false,
                previous: false,
            },
            Word16(4000),
        );
        // A bad frame leaves `last_good` alone.
        state.update(
            &mut c,
            WithPrevious {
                current: true,
                previous: false,
            },
            Word16(9000),
        );
        // The first good frame afterwards is capped at the known-good value.
        let limited = state.update(
            &mut c,
            WithPrevious {
                current: false,
                previous: true,
            },
            Word16(9000),
        );
        assert_eq!(limited.0, 4000, "the first good frame must be limited");
        // The next one is not.
        let free = state.update(
            &mut c,
            WithPrevious {
                current: false,
                previous: false,
            },
            Word16(9000),
        );
        assert_eq!(free.0, 9000, "only the frame after an erasure is limited");
    }

    #[test]
    fn only_the_code_concealer_advances_the_predictor() {
        // A lost frame still has to rotate the MA history, or the first good
        // frame afterwards predicts from stale entries. The pitch concealer
        // has nothing to rotate and must leave it alone.
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        for step in 1..=4i16 {
            predictor.push(Word16(step), Word16(step * 10));
        }
        let before = predictor;

        let pitch = PitchGainConcealer::new();
        let _substituted = pitch.conceal(&mut c, 3);
        assert_eq!(
            predictor, before,
            "ec_gain_pitch must not touch the predictor"
        );

        let code = CodeGainConcealer::new();
        code.conceal(&mut c, &mut predictor, 3);
        assert_ne!(predictor, before, "ec_gain_code must advance the predictor");
        // And with the *average*, taken before the shift.
        let (log2_avg, db_avg) = before.limited_average(&mut c);
        assert_eq!(predictor.past_log2[0], log2_avg);
        assert_eq!(predictor.past_db[0], db_avg);
    }

    #[test]
    fn the_median_is_the_middle_value_however_the_history_is_ordered() {
        // A selection sort written out by hand is exactly where an index base
        // goes wrong, and a wrong one still returns a plausible gain.
        let mut c = ctx();
        let values = [Word16(7), Word16(1), Word16(9), Word16(3), Word16(5)];
        assert_eq!(median(&mut c, &values).0, 5);

        // Every permutation of the same five must agree.
        let mut permuted = values;
        for rotate in 1..5 {
            permuted.rotate_left(rotate);
            assert_eq!(median(&mut c, &permuted).0, 5, "rotation {rotate}");
        }

        // Negative gains are legal in Q1 and must not be mistaken for the
        // consumed marker.
        let signed = [Word16(-9), Word16(-1), Word16(-5), Word16(-3), Word16(-7)];
        assert_eq!(median(&mut c, &signed).0, -5);
    }

    #[test]
    fn the_median_reproduces_the_references_degenerate_case() {
        // With every entry at i16::MIN the inner scan selects nothing --
        // sub(-32768, -32767) is negative -- and the index carried over from
        // the previous iteration is reused. A "fixed" version that re-seeds
        // the index each time returns a different slot.
        let mut c = ctx();
        let floor = [Word16(i16::MIN); GAIN_BUFFER];
        assert_eq!(median(&mut c, &floor).0, i16::MIN);

        // Ties go to the LAST index, which is observable when the tied values
        // sit at different positions.
        let tied = [Word16(4), Word16(4), Word16(4), Word16(1), Word16(9)];
        assert_eq!(median(&mut c, &tied).0, 4);
    }

    #[test]
    fn twelve_two_clears_two_bits_of_the_pitch_quantiser_and_seven_ninety_five_does_not() {
        // Both rates send four bits into the same sixteen-entry table; only
        // 12.2 discards the low two bits, for bit-exactness with the EFR.
        let mut c = ctx();
        for index in 0..16u16 {
            let wide = decode_pitch_gain(&mut c, R7_95, index);
            let narrow = decode_pitch_gain(&mut c, R12_2, index);
            assert_eq!(wide.0, QUA_GAIN_PITCH[usize::from(index)]);
            assert_eq!(narrow.0, wide.0 & !3, "index {index}");
        }
    }

    #[test]
    fn the_pitch_quantiser_stays_inside_its_q14_range() {
        // 19661 is 1.2 in Q14 and is the table's documented maximum; a value
        // above it would mean the table had been read at the wrong scale.
        for (index, &gain) in QUA_GAIN_PITCH.iter().enumerate() {
            assert!(
                (0..=19661).contains(&gain),
                "entry {index} is {gain}, outside the Q14 pitch-gain range"
            );
        }
    }

    #[test]
    fn the_two_gain_paths_denormalise_by_different_shifts() {
        // The joint tables hold g_fac in Q12 and the split table in Q11, so
        // the denormalising shift is `10 - exp` in one path and `9 - exp` in
        // the other. This pins both constants against the two candidates, so
        // a later "simplification" that unifies them fails here rather than
        // halving or doubling the innovation in production.
        let mut c = ctx();
        let code = [Word16(1200); L_SUBFR];

        // Split path, 7.95 kbit/s.
        let index = 31u16;
        let g_fac = Word16(QUA_GAIN_CODE[usize::from(index) * 3]);
        let mut split_state = CodeGainPredictor::new();
        let prediction = split_state.predict(&mut c, R7_95, &code);
        let gcode0 = extract_l(pow2(&mut c, Word16(14), prediction.fraction));
        let product = l_mult(&mut c, g_fac, gcode0);

        let nine = sub(&mut c, Word16(9), prediction.exponent).0;
        let with_nine = extract_h(l_shr(&mut c, product, nine));
        let ten = sub(&mut c, Word16(10), prediction.exponent).0;
        let with_ten = extract_h(l_shr(&mut c, product, ten));
        assert_ne!(
            with_nine, with_ten,
            "pick a louder case; the two shifts agree"
        );

        let split = decode_code_gain(&mut c, &mut split_state, R7_95, index, &code);
        assert_eq!(split, with_nine, "the split path must shift by 9 - exp");

        // Joint path, 6.70 kbit/s, same construction against its own table.
        let joint_index = 100u16;
        let joint_g_fac = Word16(GAIN_HIGHRATES[usize::from(joint_index) * 4 + 1]);
        let mut joint_state = CodeGainPredictor::new();
        let prediction = joint_state.predict(&mut c, R6_70, &code);
        let gcode0 = extract_l(pow2(&mut c, Word16(14), prediction.fraction));
        let product = l_mult(&mut c, joint_g_fac, gcode0);

        let nine = sub(&mut c, Word16(9), prediction.exponent).0;
        let with_nine = extract_h(l_shr(&mut c, product, nine));
        let ten = sub(&mut c, Word16(10), prediction.exponent).0;
        let with_ten = extract_h(l_shr(&mut c, product, ten));
        assert_ne!(
            with_nine, with_ten,
            "pick a louder case; the two shifts agree"
        );

        let joint = decode_joint(&mut c, &mut joint_state, R6_70, joint_index, &code, true);
        assert_eq!(
            joint.code, with_ten,
            "the joint path must shift by 10 - exp"
        );
    }

    #[test]
    fn seven_forty_uses_its_own_slightly_wrong_constant() {
        // 5439 against 5443 -- one digit, and the whole rate diverges. The
        // two prediction paths must disagree for the same history and
        // innovation.
        let mut c = ctx();
        let code = [Word16(2000); L_SUBFR];
        let predictor = CodeGainPredictor::new();

        let is641 = predictor.predict(&mut c, R7_40, &code);
        let correct = predictor.predict(&mut c, R10_2, &code);
        assert_ne!(
            (is641.exponent, is641.fraction),
            (correct.exponent, correct.fraction),
            "7.40 must keep the IS-641 constant"
        );
    }

    #[test]
    fn only_seven_ninety_five_reports_the_innovation_energy() {
        let mut c = ctx();
        let code = [Word16(3000); L_SUBFR];
        let predictor = CodeGainPredictor::new();

        for mode_index in 0..=R12_2 {
            let prediction = predictor.predict(&mut c, mode_index, &code);
            assert_eq!(
                prediction.innovation_energy.is_some(),
                mode_index == R7_95,
                "mode {mode_index} innovation energy"
            );
        }
    }

    #[test]
    fn the_predicted_fraction_is_never_negative() {
        // `Pow2` indexes a table with the fraction and a negative one would
        // read off the front. The double-precision split guarantees it, and
        // this pins the guarantee across every rate and a wide energy range.
        let mut c = ctx();
        let predictor = CodeGainPredictor::new();
        let mut seed = 4242i16;

        for mode_index in 0..=R12_2 {
            for _ in 0..8 {
                let mut code = [Word16(0); L_SUBFR];
                for sample in &mut code {
                    *sample = shr(&mut c, Word16(next_noise(&mut seed)), 2);
                }
                let prediction = predictor.predict(&mut c, mode_index, &code);
                assert!(
                    prediction.fraction.0 >= 0,
                    "mode {mode_index}: fraction {} is negative",
                    prediction.fraction.0
                );
            }
        }
    }

    #[test]
    fn a_silent_innovation_predicts_without_dividing_by_zero() {
        // A frame of pure silence has no innovation energy at all. The
        // logarithm has to survive it, at every rate.
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        let silence = [Word16(0); L_SUBFR];

        for mode_index in 0..=R12_2 {
            let prediction = predictor.predict(&mut c, mode_index, &silence);
            assert!(prediction.fraction.0 >= 0);
        }

        // And the whole decode path, both halves of it.
        decode_joint(&mut c, &mut predictor, R4_75, 0, &silence, true);
        decode_code_gain(&mut c, &mut predictor, R12_2, 0, &silence);
    }

    #[test]
    fn four_seventy_five_reads_a_different_half_of_its_entry_per_subframe() {
        // One 8-bit index covers two subframes. If the offset were ignored,
        // both subframes would decode the same pitch gain -- which sounds
        // fine and is wrong.
        let mut c = ctx();
        let code = [Word16(1500); L_SUBFR];

        let mut even_state = CodeGainPredictor::new();
        let even = decode_joint(&mut c, &mut even_state, R4_75, 37, &code, true);

        let mut odd_state = CodeGainPredictor::new();
        let odd = decode_joint(&mut c, &mut odd_state, R4_75, 37, &code, false);

        assert_eq!(even.pitch.0, GAIN_MR475[37 * 4]);
        assert_eq!(odd.pitch.0, GAIN_MR475[37 * 4 + 2]);
        assert_ne!(
            even.pitch, odd.pitch,
            "the two halves of entry 37 are identical; pick another index"
        );
    }

    #[test]
    fn comfort_noise_reseeding_leaves_the_two_scales_related_not_equal() {
        // A SID frame re-seeds both histories from one number, scaling the
        // log2 one by 1/(20 log10 2). Resetting to the floor instead -- the
        // intuitive reading -- makes the first speech frame after a silence
        // far too quiet.
        let mut c = ctx();
        let mut predictor = CodeGainPredictor::new();
        predictor.reseed_from_sid(&mut c, Word16(12000));

        let db = predictor.past_db[0];
        let log2_scaled = predictor.past_log2[0];
        assert!(db.0 <= 0, "the seed is clamped at zero from above");
        assert!(db.0 >= -14436, "and floored below");
        assert_eq!(log2_scaled, mult(&mut c, Word16(5443), db));
        assert!(predictor.past_db.iter().all(|&v| v == db));
        assert!(predictor.past_log2.iter().all(|&v| v == log2_scaled));

        // A very loud SID pins at the ceiling rather than going positive.
        predictor.reseed_from_sid(&mut c, Word16(32000));
        assert_eq!(predictor.past_db[0].0, 0);
    }

    #[test]
    fn comfort_noise_wipes_the_smoother_and_re_arms_its_hangover() {
        let mut c = ctx();
        let mut smoother = CodeGainSmoother::new();
        let lsf = [Word16(2000); LSF_ORDER];
        for step in 0..6i16 {
            smoother.smooth(
                &mut c,
                R6_70,
                Word16(700 + step),
                &lsf,
                &lsf,
                FrameQuality::default(),
            );
        }
        assert_ne!(smoother.history, [Word16(0); GAIN_HISTORY]);

        smoother.reset_for_comfort_noise();
        assert_eq!(smoother.history, [Word16(0); GAIN_HISTORY]);
        assert_eq!(smoother.hang_count.0, 0);
        assert_eq!(
            smoother.hang_var.0, 20,
            "the hangover is re-armed, not cleared"
        );
    }
}
