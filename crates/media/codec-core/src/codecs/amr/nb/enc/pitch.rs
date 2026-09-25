//! AMR-NB pitch analysis, encoder side — 3GPP TS 26.090 §5.5 and §5.6.
//!
//! Implements TS 26.073's `ol_ltp` (`ol_ltp.c`), `Pitch_ol` with its `Lag_max`
//! and `comp_corr` (`pitch_ol.c`, `calc_cor.c`), `Pitch_ol_wgh` with its own
//! weighted `Lag_max` and `gmed_n` (`p_ol_wgh.c`, `gmed_n.c`), `Pitch_fr` with
//! `Norm_Corr`, `searchFrac` and `getRange` (`pitch_fr.c`), `Interpol_3or6`
//! (`inter_36.c`), `Convolve` (`convolve.c`), `Enc_lag3`/`Enc_lag6`,
//! `G_pitch` (`g_pitch.c`), `q_gain_pitch`'s index search (`q_gain_p.c`), the
//! `cl_ltp` driver (`cl_ltp.c`), and the tone-stability tracker `check_lsp` /
//! `check_gp_clipping` / `update_gp_clipping` (`ton_stab.c`).
//!
//! # Three searches, three different tie-breaks
//!
//! This is the trap the module exists to avoid. All three searches produce
//! perfectly reasonable speech when broken, and only the transmitted index
//! reveals it:
//!
//! | search | comparison | walk | tie goes to |
//! |---|---|---|---|
//! | open-loop lag, per section ([`peak_lag`]) | `>=` | descending | the **smallest** lag |
//! | closed-loop integer lag ([`ClosedLoopPitch::search`]) | `>=` | ascending | the **largest** lag |
//! | closed-loop fraction ([`search_fraction`]) | `>` | ascending from the start point | the **lowest** fraction |
//!
//! The open loop's preference for short lags is *not* in its tie-break — it is
//! the 0.85 handicap [`arbitrate_sections`] applies to the incumbent, whose own
//! comparisons are strict and therefore keep the longer-lag candidate on a tie.
//!
//! # Scaling decisions that choose the answer
//!
//! Two frame-wide or window-wide scaling choices are taken once from a single
//! measurement and then applied throughout, and both change which candidate
//! wins:
//!
//! - [`open_loop_lag`] picks ×8, ×1 or ÷8 for the whole frame from the total
//!   energy, detecting overflow as `t0 == MAX_32` rather than through a flag.
//! - [`normalised_correlation`] picks ×1 or ÷4 for the whole delay window from
//!   the energy at the *shortest* delay only, and narrows each result with a
//!   saturating 16-bit extraction, after which every saturated delay compares
//!   equal and the `>=` tie-break hands the lag to the largest of them.
//!
//! For every rate but 12.2 the open loop's normalised correlation is taken with
//! `extract_l` — the **low** half-word, unsaturated — so it can come out
//! negative for a positive product. That is what the section arbitration
//! compares, and it is normative.
//!
//! # Validated by
//!
//! `testdata/nb_enc_trace.txt` (three frames at 7.40 kbit/s from TS 26.073's own
//! encoder) and the bitstream `testdata/amrnb_enc_mode4.amr` it produced:
//!
//! - [`convolve`] against `y1`, [`pitch_gain`] against `gain_pit_ol`, and
//!   [`update_target`] against `xn2`, for all twelve committed subframes;
//! - the whole closed-loop search — [`normalised_correlation`], the integer
//!   scan, [`search_fraction`] and its post-normalisation — against `T0` and
//!   `T0_frac` on the six delta-coded subframes, whose window follows from the
//!   previous subframe's lag and so needs no open-loop input;
//! - [`encode_lag_1_3`] against all twelve transmitted lag indices;
//! - [`encode_lag_1_3`]/[`encode_lag_1_6`] against the decoder's own
//!   [`super::super::lag`] over their entire code spaces.
//!
//! The open-loop search itself is **not** compared against the trace, and
//! cannot be: its input is the weighted speech `wsp`, which the instrumented
//! reference does not record, and the frame-level `T_op0` row it does emit is
//! written only on the 4.75/5.15 code path. It is covered here by hand-built
//! correlation vectors that pin each comparison and tie direction, by a planted
//! period, and — for the full-search subframes — by showing that some open-loop
//! lag reproduces the traced closed-loop result, uniquely so in frame 0.

use super::super::decoder_tables::{CORR_WEIGHT, INTER_6_SEARCH, QUA_GAIN_PITCH};
use super::super::lag::{Excitation, LagResolution, LagWindow, PitchLag};
use super::super::lsp::M;
use super::super::math::inv_sqrt;
use super::super::{L_FRAME, L_INTERPOL, L_SUBFR, PIT_MAX, PIT_MIN, PIT_MIN_MR122};
use super::vad::VoiceActivityDetector;
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_abs, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_extract, mpy_32, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_16, MAX_32, MIN_32};

/// Half a frame, the open loop's analysis window at every rate but 4.75 and
/// 5.15 kbit/s.
pub const L_FRAME_BY2: usize = L_FRAME / 2;

/// Padding on each side of the closed-loop window, for the interpolation
/// filter's reach — `L_INTER_SRCH`.
pub const L_INTER_SRCH: i16 = 4;

/// Pitch-gain clipping threshold, 0.95 in Q14.
pub const GP_CLIP: Word16 = Word16(15565);

/// Length of the pitch-gain history the clipping test averages over.
pub const N_FRAME: usize = 7;

/// Index of `exc[0]` inside the excitation buffer [`Excitation::all`] returns.
///
/// The closed-loop search reaches `PIT_MAX + L_INTER_SRCH = 147` samples back
/// and 39 samples forward from here, so the buffer is exactly large enough and
/// not one sample more.
pub const EXC_ORIGIN: usize = PIT_MAX as usize + L_INTERPOL;

/// Handicap applied to the incumbent when comparing open-loop sections, 0.85
/// in Q15.
const THRESHOLD: Word16 = Word16(27853);

/// Energy below which the open loop scales its input up by eight, 2²⁰.
const OPEN_LOOP_QUIET: Word32 = Word32(1_048_576);

/// Energy above which [`normalised_correlation`] works on a quartered
/// excitation, 2²⁶.
const NORM_CORR_LOUD: Word32 = Word32(67_108_864);

/// Fractional phases per sample in [`INTER_6_SEARCH`].
const UP_SAMP_MAX: i16 = 6;

/// Taps per side of the correlation interpolator.
const INTERP_TAPS: usize = L_INTER_SRCH as usize;

/// Mode index of 4.75 kbit/s.
const MR475: u8 = 0;
/// Mode index of 5.15 kbit/s.
const MR515: u8 = 1;
/// Mode index of 5.90 kbit/s.
const MR59: u8 = 2;
/// Mode index of 6.70 kbit/s.
const MR67: u8 = 3;
/// Mode index of 7.95 kbit/s.
const MR795: u8 = 5;
/// Mode index of 10.2 kbit/s.
const MR102: u8 = 6;
/// Mode index of 12.2 kbit/s.
const MR122: u8 = 7;

/// Longest signal window the open loop scales in one go: a whole frame plus its
/// history.
const SCALED_LEN: usize = PIT_MAX as usize + L_FRAME;

// ---------------------------------------------------------------------------
// Open-loop pitch
// ---------------------------------------------------------------------------

/// A signal scaled into the open loop's working range.
struct Scaled {
    samples: [Word16; SCALED_LEN],
    /// Index of sample 0; the history occupies `0..PIT_MAX`.
    origin: usize,
    /// Right shift applied, one of 3, 0 or −3. Only 12.2 kbit/s undoes it.
    factor: i16,
}

impl Scaled {
    fn at(&self, offset: i16) -> Word16 {
        let index = isize::try_from(self.origin).expect("origin fits") + isize::from(offset);
        self.samples[usize::try_from(index).expect("open-loop read stays inside the window")]
    }
}

/// Scale a signal so the correlation accumulator neither overflows nor loses
/// resolution.
///
/// Three-way, decided once for the whole analysis window from the total energy.
/// The overflow branch is selected by the *saturated* accumulator reading
/// exactly `MAX_32`, which is how this function detects overflow — there is no
/// flag involved, and an `i64` accumulator would never take the branch.
fn scale_for_correlation(
    ctx: &mut DspContext,
    signal: &[Word16],
    origin: usize,
    l_frame: usize,
) -> Scaled {
    let mut energy = Word32(0);
    for i in 0..PIT_MAX as usize + l_frame {
        let s = signal[origin - PIT_MAX as usize + i];
        energy = l_mac(ctx, energy, s, s);
    }

    let factor = if l_sub(ctx, energy, Word32(MAX_32)).0 == 0 {
        3
    } else if l_sub(ctx, energy, OPEN_LOOP_QUIET).0 < 0 {
        -3
    } else {
        0
    };

    let mut samples = [Word16(0); SCALED_LEN];
    for i in 0..PIT_MAX as usize + l_frame {
        samples[i] = shr(ctx, signal[origin - PIT_MAX as usize + i], factor);
    }

    Scaled {
        samples,
        origin: PIT_MAX as usize,
        factor,
    }
}

/// Cross-correlations of a scaled signal at every lag in `pit_min..=PIT_MAX` —
/// `comp_corr`.
///
/// Entry `t` of the result is `Σ_{j<l_frame} s[j] · s[j−t]`, accumulated with
/// the saturating `L_mac`. Lags below `pit_min` are left at zero.
fn correlations(
    ctx: &mut DspContext,
    scaled: &Scaled,
    l_frame: usize,
    pit_min: i16,
) -> [Word32; PIT_MAX as usize + 1] {
    let mut corr = [Word32(0); PIT_MAX as usize + 1];
    for lag in (pit_min..=PIT_MAX).rev() {
        let mut acc = Word32(0);
        for j in 0..l_frame {
            let j = i16::try_from(j).expect("frame length fits in i16");
            acc = l_mac(ctx, acc, scaled.at(j), scaled.at(j - lag));
        }
        corr[usize::try_from(lag).expect("lag is positive")] = acc;
    }
    corr
}

/// The lag with the largest raw correlation in `lag_min..=lag_max`.
///
/// Walks **descending** and keeps a candidate on a non-strict `>=`, so among
/// equals the **smallest** lag wins. The running maximum starts at `MIN_32`,
/// which means even an all-`MIN_32` section returns `lag_min` rather than the
/// initial `lag_max`.
///
/// # Panics
///
/// If the range runs past the end of `corr`.
#[must_use]
pub fn peak_lag(
    ctx: &mut DspContext,
    corr: &[Word32; PIT_MAX as usize + 1],
    lag_max: i16,
    lag_min: i16,
) -> i16 {
    let mut best = Word32(MIN_32);
    let mut chosen = lag_max;
    for lag in (lag_min..=lag_max).rev() {
        let value = corr[usize::try_from(lag).expect("lag is positive")];
        if l_sub(ctx, value, best).0 >= 0 {
            best = value;
            chosen = lag;
        }
    }
    chosen
}

/// One open-loop section: its best lag and that lag's normalised correlation —
/// `Lag_max` in `pitch_ol.c`.
///
/// `efr_scaling` is the reference's `scal_flag`, set only at 12.2 kbit/s. It
/// changes both the pre-multiply shift and, decisively, how the 32-bit product
/// is narrowed to the Word16 the section arbitration compares:
///
/// - set: undo the input scaling, then `extract_h(L_shl(t0, 15))`;
/// - clear: `extract_l(t0)` — the **low** half-word, taken with no saturation
///   and no shift, which can be negative for a positive product.
///
/// Reproducing the second is the single easiest line in the open-loop path to
/// get wrong, and it silently reorders the sections.
#[allow(clippy::too_many_arguments)]
fn section_peak(
    ctx: &mut DspContext,
    corr: &[Word32; PIT_MAX as usize + 1],
    scaled: &Scaled,
    efr_scaling: bool,
    l_frame: usize,
    lag_max: i16,
    lag_min: i16,
    vad: Option<&mut VoiceActivityDetector>,
) -> (i16, Word16) {
    let chosen = peak_lag(ctx, corr, lag_max, lag_min);
    let best = corr[usize::try_from(chosen).expect("lag is positive")];

    let mut energy = Word32(0);
    for i in 0..l_frame {
        let s = scaled.at(i16::try_from(i).expect("frame length fits in i16") - chosen);
        energy = l_mac(ctx, energy, s, s);
    }

    // `vad_tone_detection`, from inside `Lag_max` -- so it runs three times per
    // `Pitch_ol`, once per section, and on the *raw* peak and energy. Placing
    // it after the normalisation below would compare two Q15 values against a
    // threshold meant for the unnormalised pair.
    if let Some(vad) = vad {
        vad.observe_tone(ctx, best, energy);
    }

    let mut inverse = inv_sqrt(ctx, energy);
    if efr_scaling {
        inverse = l_shl(ctx, inverse, 1);
    }

    let (best_hi, best_lo) = l_extract(best);
    let (energy_hi, energy_lo) = l_extract(inverse);
    let mut product = mpy_32(best_hi, best_lo, energy_hi, energy_lo);

    let normalised = if efr_scaling {
        product = l_shr(ctx, product, scaled.factor);
        extract_h(l_shl(ctx, product, 15))
    } else {
        extract_l(product)
    };
    (chosen, normalised)
}

/// Pick between the three open-loop sections, favouring short lags —
/// `Pitch_ol`'s tail.
///
/// Each candidate is `(lag, normalised correlation)`, given long-lag section
/// first. A shorter section takes over only when it **strictly** exceeds
/// `0.85 ×` the incumbent's correlation, so a tie keeps the incumbent, which is
/// the longer-lag candidate. The preference for short lags lives entirely in
/// that 0.85.
///
/// The handicap is applied with `mult`, which floors toward −∞; the
/// correlations are Word16 and may be negative, and for a negative incumbent
/// the flooring makes the threshold *stricter*. A rounding multiply or a
/// truncating divide changes the chosen lag.
///
/// The third comparison deliberately reuses the possibly-updated correlation
/// without writing it back, exactly as the reference does.
#[must_use]
pub fn arbitrate_sections(
    ctx: &mut DspContext,
    long: (i16, Word16),
    middle: (i16, Word16),
    short: (i16, Word16),
) -> i16 {
    let (mut lag, mut best) = long;
    let handicapped = mult(ctx, best, THRESHOLD);
    if sub(ctx, handicapped, middle.1).0 < 0 {
        best = middle.1;
        lag = middle.0;
    }
    let handicapped = mult(ctx, best, THRESHOLD);
    if sub(ctx, handicapped, short.1).0 < 0 {
        lag = short.0;
    }
    lag
}

/// Open-loop pitch lag over one analysis window — `Pitch_ol`.
///
/// `signal` is the weighted speech; `signal[origin - PIT_MAX .. origin +
/// l_frame]` must be present, and `l_frame` is 160 at 4.75 and 5.15 kbit/s and
/// 80 elsewhere. Returns an integer lag, Q0.
///
/// The range is split into three sections that cannot contain one another's
/// pitch multiples — `PIT_MAX..4·pit_min`, `4·pit_min−1..2·pit_min` and
/// `2·pit_min−1..pit_min` — each searched in full for its raw correlation peak,
/// then arbitrated by [`arbitrate_sections`] on the energy-normalised value.
/// `pit_min` is 18 at 12.2 kbit/s and 20 everywhere else.
///
/// `vad` is `Some` only when DTX is enabled, and none of what it does here
/// influences the returned lag — the detector reads this stage, it does not
/// steer it. Three hooks fire, in this order and nowhere else: the tone
/// register shifts once on entry, each of the three sections reports its raw
/// peak, and the *second* half-frame of the frame additionally computes
/// the private `high_pass_correlation` helper for the complex-background detector.
///
/// `second_half` is the reference's `idx`, and at 4.75 and 5.15 kbit/s — where
/// one search covers the whole frame — it is true for that single call.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn open_loop_lag(
    ctx: &mut DspContext,
    mode_index: u8,
    signal: &[Word16],
    origin: usize,
    l_frame: usize,
    mut vad: Option<&mut VoiceActivityDetector>,
    second_half: bool,
) -> i16 {
    let pit_min = if mode_index == MR122 {
        PIT_MIN_MR122
    } else {
        PIT_MIN
    };
    // `vad_tone_detection_update`, before anything else in `Pitch_ol`. The
    // flag says "only one lag this frame", which is true exactly of the two
    // rates that search all 160 samples at once.
    if let Some(vad) = vad.as_deref_mut() {
        vad.shift_tone_register(ctx, matches!(mode_index, MR475 | MR515));
    }

    let scaled = scale_for_correlation(ctx, signal, origin, l_frame);
    let corr = correlations(ctx, &scaled, l_frame, pit_min);
    let efr_scaling = mode_index == MR122;

    let quarter = shl(ctx, Word16(pit_min), 2).0;
    let half = shl(ctx, Word16(pit_min), 1).0;

    let long = section_peak(
        ctx,
        &corr,
        &scaled,
        efr_scaling,
        l_frame,
        PIT_MAX,
        quarter,
        vad.as_deref_mut(),
    );
    let middle = section_peak(
        ctx,
        &corr,
        &scaled,
        efr_scaling,
        l_frame,
        quarter - 1,
        half,
        vad.as_deref_mut(),
    );
    let short = section_peak(
        ctx,
        &corr,
        &scaled,
        efr_scaling,
        l_frame,
        half - 1,
        pit_min,
        vad.as_deref_mut(),
    );

    if let Some(vad) = vad {
        if second_half {
            let correlation = high_pass_correlation(ctx, &corr, &scaled, l_frame, PIT_MAX, pit_min);
            vad.observe_correlation(correlation);
        }
    }

    arbitrate_sections(ctx, long, middle, short)
}

/// `hp_max`: the largest high-pass filtered correlation, normalised — Q15.
///
/// A second-difference across the correlation vector, which suppresses the
/// smooth part and leaves whatever varies lag to lag. A background that is
/// *complex* rather than merely noisy — music, babble — produces a large value
/// here while looking unvoiced to every other measure, and this is the only
/// thing that distinguishes it.
///
/// The denominator is the same second difference applied to the two
/// zero-lag-adjacent energies, so it is a ratio of like quantities. Both are
/// normalised before the division, and the shift difference is applied
/// afterwards — `div_s` needs its arguments comparable, and the exponent
/// bookkeeping is what makes the Q15 result meaningful.
fn high_pass_correlation(
    ctx: &mut DspContext,
    corr: &[Word32; PIT_MAX as usize + 1],
    scaled: &Scaled,
    l_frame: usize,
    lag_max: i16,
    lag_min: i16,
) -> Word16 {
    let mut max = Word32(MIN_32);
    // Strictly inside the range on both ends: the filter reads its neighbours.
    for lag in (lag_min + 1..lag_max).rev() {
        let at = |l: i16| corr[usize::try_from(l).expect("lag is positive")];
        let doubled = l_shl(ctx, at(lag), 1);
        let above = l_sub(ctx, doubled, at(lag + 1));
        let t = l_sub(ctx, above, at(lag - 1));
        let t = l_abs(ctx, t);
        if l_sub(ctx, t, max).0 >= 0 {
            max = t;
        }
    }

    let mut energy = Word32(0);
    let mut lagged = Word32(0);
    for i in 0..l_frame {
        let i = i16::try_from(i).expect("frame length fits in i16");
        let here = scaled.at(i);
        energy = l_mac(ctx, energy, here, here);
        lagged = l_mac(ctx, lagged, here, scaled.at(i - 1));
    }
    let doubled_energy = l_shl(ctx, energy, 1);
    let doubled_lagged = l_shl(ctx, lagged, 1);
    let difference = l_sub(ctx, doubled_energy, doubled_lagged);
    let denominator = l_abs(ctx, difference);

    // One less than the full normalisation on the numerator, so the quotient
    // cannot reach 1.0 and overflow `div_s`.
    let shift_num = sub(ctx, Word16(norm_l(max)), Word16(1));
    let numerator = extract_h(l_shl(ctx, max, shift_num.0));
    let shift_den = norm_l(denominator);
    let scaled_den = extract_h(l_shl(ctx, denominator, shift_den));

    let quotient = if scaled_den.0 == 0 {
        Word16(0)
    } else {
        div_s(numerator, scaled_den)
    };

    let shift = sub(ctx, shift_num, Word16(shift_den));
    if shift.0 >= 0 {
        shr(ctx, quotient, shift.0)
    } else {
        shl(ctx, quotient, -shift.0)
    }
}

/// State of the weighted open-loop search, 10.2 kbit/s only — TS 26.073
/// `pitchOLWghtState`.
///
/// Persists across frames and is updated twice per frame, once per half-frame
/// search. `wght_flg` is evaluated *after* `ada_w` moves, so it governs the
/// **next** call rather than the one that set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeightedOpenLoop {
    /// Median of the recent lag history, the centre of the proximity window.
    old_t0_med: Word16,
    /// Confidence in that median, Q15. Decays by 0.9 per unvoiced half-frame.
    ada_w: Word16,
    /// Whether the proximity weighting is armed for the next search.
    weighting_armed: bool,
}

impl Default for WeightedOpenLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl WeightedOpenLoop {
    /// The state `p_ol_wgh_reset` leaves behind.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            old_t0_med: Word16(40),
            ada_w: Word16(0),
            weighting_armed: false,
        }
    }

    /// Weighted open-loop pitch lag — `Pitch_ol_wgh`.
    ///
    /// One search over the whole `PIT_MIN..=PIT_MAX` range, with no section
    /// split and no arbitration: the lag prior is carried by the weighting
    /// table instead. `old_lags` is the caller's five-entry lag history, which
    /// the closed loop also writes into, and `voiced` reports whether the
    /// chosen lag had enough gain to be trusted.
    ///
    /// Note the input scaling shifts by **three**, not two — the reference's
    /// own comment says otherwise and the code is what counts — and its quiet
    /// threshold is 2²⁰, identical to [`open_loop_lag`]'s despite a comment
    /// claiming 2²².
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &mut self,
        ctx: &mut DspContext,
        signal: &[Word16],
        origin: usize,
        l_frame: usize,
        old_lags: &mut [Word16; 5],
        voiced: &mut bool,
        mut vad: Option<&mut VoiceActivityDetector>,
        second_half: bool,
    ) -> i16 {
        let scaled = scale_for_correlation(ctx, signal, origin, l_frame);
        let corr = correlations(ctx, &scaled, l_frame, PIT_MIN);
        let chosen = self.weighted_peak(ctx, &corr, &scaled, l_frame, voiced, vad.as_deref_mut());

        if let Some(vad) = vad {
            if second_half {
                let correlation =
                    high_pass_correlation(ctx, &corr, &scaled, l_frame, PIT_MAX, PIT_MIN);
                vad.observe_correlation(correlation);
            }
        }

        if *voiced {
            old_lags.copy_within(0..4, 1);
            old_lags[0] = Word16(chosen);
            self.old_t0_med = median_of_five(ctx, old_lags);
            self.ada_w = Word16(MAX_16);
        } else {
            self.old_t0_med = Word16(chosen);
            // 0.9 in Q15, with the flooring `mult`.
            self.ada_w = mult(ctx, self.ada_w, Word16(29491));
        }
        // 0.3 in Q15, tested after the update.
        self.weighting_armed = sub(ctx, self.ada_w, Word16(9830)).0 >= 0;

        chosen
    }

    /// The weighted `Lag_max` of `p_ol_wgh.c`.
    ///
    /// Two cursors walk [`CORR_WEIGHT`] at once. The first starts at entry 250
    /// and steps back on **every** candidate; the second starts at
    /// `123 + PIT_MAX − old_T0_med` and steps back **only while the proximity
    /// weighting is armed**. Advancing the second unconditionally shifts the
    /// whole window by one lag for the rest of the search.
    ///
    /// Ties go to the smallest lag, as in [`peak_lag`].
    fn weighted_peak(
        self,
        ctx: &mut DspContext,
        corr: &[Word32; PIT_MAX as usize + 1],
        scaled: &Scaled,
        l_frame: usize,
        voiced: &mut bool,
        vad: Option<&mut VoiceActivityDetector>,
    ) -> i16 {
        let mut fixed = CORR_WEIGHT.len() - 1;
        // The state invariant `old_t0_med ∈ [PIT_MIN, PIT_MAX]` keeps this
        // inside the table: the start is 123..246 and it steps back at most
        // PIT_MAX − PIT_MIN = 123 times.
        let mut near = usize::try_from(123 + i32::from(PIT_MAX) - i32::from(self.old_t0_med.0))
            .ok()
            .filter(|start| *start < CORR_WEIGHT.len())
            .expect("the lag history stayed inside the correlation weighting table");

        let mut best = Word32(MIN_32);
        let mut chosen = PIT_MAX;

        for lag in (PIT_MIN..=PIT_MAX).rev() {
            let raw = corr[usize::try_from(lag).expect("lag is positive")];
            let (hi, lo) = l_extract(raw);
            let mut weighted = mpy_32_16(hi, lo, Word16(CORR_WEIGHT[fixed]));
            // Both cursors step back *after* being read, and the last step of
            // the near one can leave it at -1: its start is `123 + PIT_MAX -
            // old_T0_med`, which is 123 when the median sits at PIT_MAX, and
            // the loop runs 124 times. The reference walks its pointer one
            // past the front and never dereferences it. Saturating here is
            // therefore exact, not a papering-over -- and an ordinary
            // subtraction panics, which is how this was found: only a stream
            // whose median reaches PIT_MAX gets there, and no speech fixture
            // does.
            fixed = fixed.saturating_sub(1);

            if self.weighting_armed {
                let (hi, lo) = l_extract(weighted);
                weighted = mpy_32_16(hi, lo, Word16(CORR_WEIGHT[near]));
                near = near.saturating_sub(1);
            }

            if l_sub(ctx, weighted, best).0 >= 0 {
                best = weighted;
                chosen = lag;
            }
        }

        // Not part of the decision: the gain flag the caller uses to decide
        // whether this lag joins the median history. It carries the sign of
        // `<s, s_-T> − 0.4·<s_-T, s_-T>`, with the delayed energy rounded to a
        // Word16 *before* the multiply-subtract.
        let mut cross = Word32(0);
        let mut delayed = Word32(0);
        for j in 0..l_frame {
            let j = i16::try_from(j).expect("frame length fits in i16");
            let here = scaled.at(j);
            let there = scaled.at(j - chosen);
            cross = l_mac(ctx, cross, here, there);
            delayed = l_mac(ctx, delayed, there, there);
        }
        // 10.2's own tone hooks, and note the ordering: the register shifts
        // *here*, after the search, where every other rate shifts on entry.
        // The flag is always 0 -- this rate computes two lags a frame.
        if let Some(vad) = vad {
            vad.shift_tone_register(ctx, false);
            vad.observe_tone(ctx, cross, delayed);
        }

        let rounded = round(ctx, delayed);
        // 0.4 in Q15.
        let excess = l_msu(ctx, cross, rounded, Word16(13107));
        *voiced = round(ctx, excess).0 > 0;

        chosen
    }
}

/// Five-point median — `gmed_n` with `n = 5`.
///
/// Repeatedly takes the maximum, records where it came from and poisons it,
/// then returns the value at the middle-ranked position. Two oddities are
/// reproduced: the running maximum starts at −32767 rather than −32768, so an
/// input of −32768 could never be selected; and the comparison is `>=`, so
/// among equal values the **highest** index is taken.
///
/// Not shared with [`super::super::synthesis::median_of_nine`]: that one is
/// fixed at nine points and its middle rank is a different index.
#[must_use]
pub fn median_of_five(ctx: &mut DspContext, values: &[Word16; 5]) -> Word16 {
    let mut remaining = *values;
    let mut rank = [0usize; 5];
    let mut chosen = 0usize;

    for slot in &mut rank {
        let mut largest = Word16(-32767);
        for (j, &candidate) in remaining.iter().enumerate() {
            if sub(ctx, candidate, largest).0 >= 0 {
                largest = candidate;
                chosen = j;
            }
        }
        remaining[chosen] = Word16(MIN_16_VALUE);
        *slot = chosen;
    }
    values[rank[5 / 2]]
}

/// The poison value `gmed_n` writes over an already-ranked entry.
const MIN_16_VALUE: i16 = -32768;

/// Both half-frames' open-loop lags, with the per-rate cadence — `ol_ltp`
/// together with the loop in `cod_amr`.
///
/// `wsp` is the weighted speech with `wsp[origin - PIT_MAX .. origin + L_FRAME]`
/// present. Returns the two lags the closed loop's full searches are centred
/// on, for the first and second halves of the frame.
///
/// The cadence is not uniform, and getting it wrong costs both rates that use
/// the exception:
///
/// - **4.75 and 5.15 kbit/s** search **once** over the whole 160-sample frame
///   and copy the result into both halves. The reference reaches this through
///   an outer test that runs *before* the `mode <= MR795` chain, so a flattened
///   dispatch sends them down the 80-sample path instead.
/// - **10.2 kbit/s** uses the weighted search, which owns `old_lags` and
///   `voiced` and does **not** clear the latter on entry; every other rate
///   clears both entries first.
/// - **12.2 kbit/s** searches down to lag 18 rather than 20.
#[allow(clippy::too_many_arguments)]
pub fn open_loop_lags(
    ctx: &mut DspContext,
    mode_index: u8,
    weighted: &mut WeightedOpenLoop,
    wsp: &[Word16],
    origin: usize,
    old_lags: &mut [Word16; 5],
    voiced: &mut [bool; 2],
    mut vad: Option<&mut VoiceActivityDetector>,
) -> [i16; 2] {
    if mode_index != MR102 {
        voiced[0] = false;
        voiced[1] = false;
    }

    if mode_index == MR475 || mode_index == MR515 {
        // One search over the whole frame, and the reference passes idx = 1 --
        // so the complex-background hook fires on it.
        let lag = open_loop_lag(ctx, mode_index, wsp, origin, L_FRAME, vad, true);
        return [lag, lag];
    }

    let mut lags = [0i16; 2];
    for (half, slot) in lags.iter_mut().enumerate() {
        let at = origin + half * L_FRAME_BY2;
        *slot = if mode_index == MR102 {
            let mut flag = voiced[half];
            let lag = weighted.search(
                ctx,
                wsp,
                at,
                L_FRAME_BY2,
                old_lags,
                &mut flag,
                vad.as_deref_mut(),
                half == 1,
            );
            voiced[half] = flag;
            lag
        } else {
            open_loop_lag(
                ctx,
                mode_index,
                wsp,
                at,
                L_FRAME_BY2,
                vad.as_deref_mut(),
                half == 1,
            )
        };
    }
    lags
}

// ---------------------------------------------------------------------------
// Closed-loop pitch
// ---------------------------------------------------------------------------

/// Per-rate closed-loop search parameters — the `mode_dep_parm` table of
/// `pitch_fr.c`.
///
/// File-local to the reference rather than a `.tab`, so it is transcribed here
/// with its row order matching the mode indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeParams {
    /// Above this integer lag a full search transmits no fraction at all.
    pub max_frac_lag: i16,
    /// One-third resolution; only 12.2 kbit/s clears it.
    pub one_third: bool,
    /// Fraction the search starts from, and the incumbent for its ties.
    pub first_frac: i16,
    /// Last fraction tried.
    pub last_frac: i16,
    /// How far below the open-loop lag a full search starts.
    pub delta_int_low: i16,
    /// Width of a full search's window.
    pub delta_int_range: i16,
    /// How far below the previous lag a delta search starts.
    pub delta_frc_low: i16,
    /// Width of a delta search's window.
    pub delta_frc_range: i16,
    /// Shortest lag this rate can code.
    pub pit_min: i16,
}

/// The closed-loop parameters of one rate.
///
/// # Panics
///
/// If `mode_index` names no speech rate. The reference declares a ninth,
/// all-zero row for comfort noise that is never indexed, and indexing it would
/// give a search window of width zero at lag zero.
#[must_use]
pub const fn mode_params(mode_index: u8) -> ModeParams {
    assert!(
        mode_index <= MR122,
        "no closed-loop parameters for this mode"
    );
    let (max_frac_lag, one_third, first_frac, last_frac, pit_min) = match mode_index {
        MR122 => (94, false, -3, 3, PIT_MIN_MR122),
        _ => (84, true, -2, 2, PIT_MIN),
    };
    let (delta_int_low, delta_int_range) = match mode_index {
        MR475 | MR515 => (5, 10),
        _ => (3, 6),
    };
    let (delta_frc_low, delta_frc_range) = match mode_index {
        MR795 => (10, 19),
        _ => (5, 9),
    };
    ModeParams {
        max_frac_lag,
        one_third,
        first_frac,
        last_frac,
        delta_int_low,
        delta_int_range,
        delta_frc_low,
        delta_frc_range,
        pit_min,
    }
}

/// A search window around a lag — `getRange`.
///
/// Clamping the top **re-derives** the bottom, so the window keeps its full
/// `range + 1` width at the top of the lag range; clamping the bottom does not,
/// so a window pinned at `pit_min` is the one place the width shrinks.
#[must_use]
pub fn lag_window(
    ctx: &mut DspContext,
    lag: i16,
    delta_low: i16,
    delta_range: i16,
    pit_min: i16,
) -> LagWindow {
    let mut min = sub(ctx, Word16(lag), Word16(delta_low));
    if sub(ctx, min, Word16(pit_min)).0 < 0 {
        min = Word16(pit_min);
    }
    let mut max = add(ctx, min, Word16(delta_range));
    if sub(ctx, max, Word16(PIT_MAX)).0 > 0 {
        max = Word16(PIT_MAX);
        min = sub(ctx, max, Word16(delta_range));
    }
    LagWindow { min, max }
}

/// Truncated convolution of two subframe-length vectors — `Convolve`.
///
/// `y[n] = Σ_{i≤n} x[i]·h[n−i]` for the first `L_SUBFR` samples only. The
/// accumulator saturates, the `L_shl(s, 3)` that removes `h`'s Q12 scaling
/// saturates too, and the result is taken with `extract_h` rather than `round`.
///
/// # Panics
///
/// If either input is shorter than a subframe.
#[must_use]
pub fn convolve(ctx: &mut DspContext, x: &[Word16], h: &[Word16]) -> [Word16; L_SUBFR] {
    assert!(
        x.len() >= L_SUBFR && h.len() >= L_SUBFR,
        "convolve needs a full subframe"
    );
    let mut y = [Word16(0); L_SUBFR];
    for n in 0..L_SUBFR {
        let mut s = Word32(0);
        for i in 0..=n {
            s = l_mac(ctx, s, x[i], h[n - i]);
        }
        y[n] = extract_h(l_shl(ctx, s, 3));
    }
    y
}

/// Normalised correlations across a closed-loop delay window.
///
/// Indexed by integer lag; [`Correlations::at`] does the offsetting the
/// reference does with a shifted pointer.
#[derive(Debug, Clone, Copy)]
pub struct Correlations {
    first: i16,
    last: i16,
    values: [Word16; 40],
}

impl Correlations {
    /// The normalised correlation at one lag.
    ///
    /// # Panics
    ///
    /// If the lag was never computed, which means the caller's window and this
    /// window disagree.
    #[must_use]
    pub fn at(&self, lag: i16) -> Word16 {
        assert!(
            lag >= self.first && lag <= self.last,
            "lag {lag} is outside the correlated window {}..={}",
            self.first,
            self.last
        );
        self.values[usize::try_from(lag - self.first).expect("in-window offset")]
    }
}

/// Normalised correlation between the target and the filtered past excitation,
/// for every delay in a padded window — `Norm_Corr`.
///
/// `exc[origin]` is the current subframe's first sample; the search reaches
/// `t_max` samples back and, through the convolution at the shortest delay,
/// `L_SUBFR − t_min` samples *forward* into the current subframe. That forward
/// region is not stale: `subframePreProc` seeds it with the subframe's LP
/// residual before this runs, and the reference depends on that.
///
/// `h` is the weighted synthesis filter's impulse response, Q12. The result at
/// lag `t` is `<xn, y_t> / sqrt(<y_t, y_t>)`.
///
/// Two things fix which lag later wins:
///
/// - the ×1 / ÷4 decision is taken **once**, from the energy at `t_min` only,
///   and applies to the whole window; `scaled_excf` is filled before the test
///   regardless of the outcome;
/// - each result is narrowed with `extract_h(L_shl(s, 16))`, which **saturates**
///   rather than truncating, so once several delays saturate they compare equal.
///
/// The in-place update between delays must run **descending**, because each
/// output reads the previous delay's neighbouring sample.
///
/// # Panics
///
/// If the window is wider than 40 lags, or if `exc` does not cover it.
#[must_use]
pub fn normalised_correlation(
    ctx: &mut DspContext,
    exc: &[Word16],
    origin: usize,
    xn: &[Word16],
    h: &[Word16],
    t_min: i16,
    t_max: i16,
) -> Correlations {
    let width = usize::try_from(t_max - t_min + 1).expect("a non-empty window");
    assert!(
        width <= 40,
        "the closed-loop window is at most 40 lags wide"
    );
    assert!(
        xn.len() >= L_SUBFR && h.len() >= L_SUBFR,
        "the target and impulse response are one subframe each"
    );

    let mut back = usize::try_from(i32::from(t_min)).expect("delays are positive");
    let start = origin
        .checked_sub(back)
        .expect("excitation history is too short");
    let mut filtered = convolve(ctx, &exc[start..start + L_SUBFR], h);

    // Computed unconditionally, before the test that decides whether it is
    // used — the reference does the same and the flags it sets are shared.
    let mut quartered = [Word16(0); L_SUBFR];
    for (slot, &value) in quartered.iter_mut().zip(filtered.iter()) {
        *slot = shr(ctx, value, 2);
    }

    let mut energy = Word32(0);
    for &value in &filtered {
        energy = l_mac(ctx, energy, value, value);
    }
    let loud = l_sub(ctx, energy, NORM_CORR_LOUD).0 > 0;
    if loud {
        filtered = quartered;
    }
    // `15 - 12` normally, `15 - 12 - 2` when the excitation was quartered.
    let h_fac = if loud { 1 } else { 3 };
    let scaling = if loud { 2 } else { 0 };

    let mut values = [Word16(0); 40];
    for lag in t_min..=t_max {
        let mut energy = Word32(0);
        for &value in &filtered {
            energy = l_mac(ctx, energy, value, value);
        }
        let (norm_hi, norm_lo) = l_extract(inv_sqrt(ctx, energy));

        let mut cross = Word32(0);
        for (j, &value) in filtered.iter().enumerate() {
            cross = l_mac(ctx, cross, xn[j], value);
        }
        let (cross_hi, cross_lo) = l_extract(cross);

        let s = mpy_32(cross_hi, cross_lo, norm_hi, norm_lo);
        values[usize::try_from(lag - t_min).expect("in-window offset")] =
            extract_h(l_shl(ctx, s, 16));

        if lag != t_max {
            back += 1;
            let tap = origin - back;
            for j in (1..L_SUBFR).rev() {
                let product = l_mult(ctx, exc[tap], h[j]);
                let term = extract_h(l_shl(ctx, product, h_fac));
                filtered[j] = add(ctx, term, filtered[j - 1]);
            }
            filtered[0] = shr(ctx, exc[tap], scaling);
        }
    }

    Correlations {
        first: t_min,
        last: t_max,
        values,
    }
}

/// Interpolate the normalised correlation at a fractional lag —
/// `Interpol_3or6`.
///
/// `frac` counts thirds when `one_third` is set and sixths otherwise; a
/// one-third phase is the doubled one-sixth phase, so a single 25-tap table
/// serves both. Reads `corr[lag−4 ..= lag+4]`, which is exactly why the
/// correlated window is padded by [`L_INTER_SRCH`] on each side.
///
/// **Not the 61-tap `inter_6` of `pred_lt.c`.** The reference gives two
/// different tables the same name; this is the short one, from `inter_36.tab`.
///
/// # Panics
///
/// If `lag ± 4` falls outside the correlated window.
#[must_use]
pub fn interpolate(
    ctx: &mut DspContext,
    corr: &Correlations,
    lag: i16,
    frac: i16,
    one_third: bool,
) -> Word16 {
    let mut phase = if one_third {
        shl(ctx, Word16(frac), 1).0
    } else {
        frac
    };
    let mut centre = lag;
    if phase < 0 {
        phase += UP_SAMP_MAX;
        centre -= 1;
    }
    let phase = usize::try_from(phase).expect("the phase is folded non-negative");
    let mirror = usize::try_from(UP_SAMP_MAX).expect("six is positive") - phase;

    let mut s = Word32(0);
    for i in 0..INTERP_TAPS {
        let k = i * usize::try_from(UP_SAMP_MAX).expect("six is positive");
        let step = i16::try_from(i).expect("four taps fit in i16");
        s = l_mac(
            ctx,
            s,
            corr.at(centre - step),
            Word16(INTER_6_SEARCH[phase + k]),
        );
        s = l_mac(
            ctx,
            s,
            corr.at(centre + 1 + step),
            Word16(INTER_6_SEARCH[mirror + k]),
        );
    }
    round(ctx, s)
}

/// Refine an integer lag to a fraction — `searchFrac`.
///
/// The incoming fraction is evaluated **first** and becomes the incumbent, then
/// `frac+1 ..= last_frac` ascending. The comparison is **strict**, so ties go
/// to the lowest fraction and in particular the starting fraction beats
/// everything after it — the opposite strictness from the integer search that
/// precedes it.
///
/// The post-normalisation afterwards can move the integer lag by ±1, and so out
/// of the window that was searched. Its two one-third tests are **sequential,
/// not exclusive**: with the mode table's `first_frac = −2` and
/// `last_frac = 2`, both are reachable.
///
/// Returns the possibly-adjusted `(lag, frac)`.
#[must_use]
pub fn search_fraction(
    ctx: &mut DspContext,
    lag: i16,
    frac: i16,
    last_frac: i16,
    corr: &Correlations,
    one_third: bool,
) -> (i16, i16) {
    let mut lag = lag;
    let mut frac = frac;

    let mut best = interpolate(ctx, corr, lag, frac, one_third);
    let mut candidate = frac + 1;
    while candidate <= last_frac {
        let value = interpolate(ctx, corr, lag, candidate, one_third);
        if sub(ctx, value, best).0 > 0 {
            best = value;
            frac = candidate;
        }
        candidate += 1;
    }

    if one_third {
        if frac == -2 {
            frac = 1;
            lag = sub(ctx, Word16(lag), Word16(1)).0;
        }
        if frac == 2 {
            frac = -1;
            lag = add(ctx, Word16(lag), Word16(1)).0;
        }
    } else if frac == -3 {
        frac = 3;
        lag = sub(ctx, Word16(lag), Word16(1)).0;
    }

    (lag, frac)
}

/// What one closed-loop search produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosedLoopResult {
    /// The chosen lag and fraction.
    pub lag: PitchLag,
    /// The transmitted pitch index, Q0.
    pub index: u16,
    /// The integer window that was searched, before the fractional refinement
    /// possibly stepped outside it.
    pub window: LagWindow,
    /// Whether the index is coded relative to the previous subframe.
    pub delta: bool,
}

/// The closed-loop pitch search and its one piece of carried state — TS 26.073
/// `Pitch_frState`.
///
/// `previous_lag` is read twice per delta subframe — once for the search
/// window, once by [`encode_lag_1_3`]'s four-bit path — and written only after
/// both, so it is the *old* value both times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClosedLoopPitch {
    previous_lag: Word16,
}

impl ClosedLoopPitch {
    /// The state `Pitch_fr_reset` leaves behind: a previous lag of zero.
    ///
    /// Zero is not a decodable lag; it only ever reaches [`lag_window`], which
    /// clamps it up to `pit_min`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous_lag: Word16(0),
        }
    }

    /// The lag carried out of the previous subframe.
    #[must_use]
    pub const fn previous_lag(&self) -> Word16 {
        self.previous_lag
    }

    /// Search one subframe for its pitch lag — `Pitch_fr`.
    ///
    /// `open_loop` holds the two half-frame open-loop lags; `exc[origin]` is
    /// the current subframe's first sample, with its 40 samples already seeded
    /// with the LP residual (see [`normalised_correlation`]); `xn` is the pitch
    /// target and `h1` the weighted synthesis impulse response, Q12.
    ///
    /// Subframes 0 and 2 search the full window around the matching open-loop
    /// lag, subframes 1 and 3 a narrow window around the previous subframe's —
    /// **except** at 4.75 and 5.15 kbit/s, whose subframe 2 also searches
    /// relative to the previous subframe, and is the only odd subframe that
    /// does.
    ///
    /// The integer scan is ascending with a non-strict `>=`, so ties go to the
    /// **largest** lag. The fractional refinement that follows is gated:
    ///
    /// - a full search whose integer lag exceeds `max_frac_lag` transmits no
    ///   fraction at all;
    /// - the four-bit rates (4.75, 5.15, 5.90 and **6.70** — the reference's
    ///   comment omits the last, and the code is what counts) restrict the
    ///   fractional range according to where the lag sits relative to an anchor
    ///   clamped into the window, and skip the refinement entirely when it sits
    ///   further away than that;
    /// - every other case searches the full fractional range.
    ///
    /// # Panics
    ///
    /// If `exc` does not cover the window the search reaches.
    // Nine parameters, because `Pitch_fr` genuinely depends on nine things and
    // bundling them into a struct would only move the list somewhere else.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn search(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        open_loop: [i16; 2],
        exc: &[Word16],
        origin: usize,
        xn: &[Word16],
        h1: &[Word16],
        subframe: usize,
    ) -> ClosedLoopResult {
        let parm = mode_params(mode_index);
        // The reference's comment names only 4.75, 5.15 and 5.90; its code
        // includes 6.70, and the code is what the bitstream follows.
        let four_bit = matches!(mode_index, MR475 | MR515 | MR59 | MR67);

        let even = subframe.is_multiple_of(2);
        // 4.75 and 5.15 kbit/s delta-code subframe 2 as well.
        let delta = !even || (subframe == 2 && matches!(mode_index, MR475 | MR515));
        let window = if delta {
            lag_window(
                ctx,
                self.previous_lag.0,
                parm.delta_frc_low,
                parm.delta_frc_range,
                parm.pit_min,
            )
        } else {
            lag_window(
                ctx,
                open_loop[subframe / 2],
                parm.delta_int_low,
                parm.delta_int_range,
                parm.pit_min,
            )
        };

        let t_min = sub(ctx, window.min, Word16(L_INTER_SRCH)).0;
        let t_max = add(ctx, window.max, Word16(L_INTER_SRCH)).0;
        let corr = normalised_correlation(ctx, exc, origin, xn, h1, t_min, t_max);

        let mut best = corr.at(window.min.0);
        let mut lag = window.min.0;
        for candidate in window.min.0 + 1..=window.max.0 {
            if sub(ctx, corr.at(candidate), best).0 >= 0 {
                best = corr.at(candidate);
                lag = candidate;
            }
        }

        let mut frac = parm.first_frac;
        let mut last_frac = parm.last_frac;
        if !delta && sub(ctx, Word16(lag), Word16(parm.max_frac_lag)).0 > 0 {
            frac = 0;
        } else if delta && four_bit {
            let anchor = four_bit_anchor(ctx, self.previous_lag.0, window);
            if lag == anchor || lag == anchor - 1 {
                let (l, f) = search_fraction(ctx, lag, frac, last_frac, &corr, parm.one_third);
                lag = l;
                frac = f;
            } else if lag == anchor - 2 {
                // Only the right-hand fractions.
                frac = 0;
                let (l, f) = search_fraction(ctx, lag, frac, last_frac, &corr, parm.one_third);
                lag = l;
                frac = f;
            } else if lag == anchor + 1 {
                // Only the left-hand fractions.
                last_frac = 0;
                let (l, f) = search_fraction(ctx, lag, frac, last_frac, &corr, parm.one_third);
                lag = l;
                frac = f;
            } else {
                frac = 0;
            }
        } else {
            let (l, f) = search_fraction(ctx, lag, frac, last_frac, &corr, parm.one_third);
            lag = l;
            frac = f;
        }

        let index = if parm.one_third {
            encode_lag_1_3(ctx, lag, frac, self.previous_lag.0, window, delta, four_bit)
        } else {
            encode_lag_1_6(ctx, lag, frac, window.min.0, delta)
        };

        // Only now, after `Enc_lag3` has consumed the old value.
        self.previous_lag = Word16(lag);

        ClosedLoopResult {
            lag: PitchLag {
                integer: Word16(lag),
                frac: Word16(frac),
                resolution: if parm.one_third {
                    LagResolution::OneThird
                } else {
                    LagResolution::OneSixth
                },
            },
            index,
            window,
            delta,
        }
    }
}

/// Pull the previous subframe's lag into the window the four-bit code is
/// anchored on.
///
/// Both clamps are applied **in order** and the second can undo the first.
/// `Pitch_fr` and `Enc_lag3` each recompute this from the same inputs and must
/// agree; the decoder's [`super::super::lag::delta_lag_1_3`] recomputes it a
/// third time.
fn four_bit_anchor(ctx: &mut DspContext, previous_lag: i16, window: LagWindow) -> i16 {
    let mut anchor = Word16(previous_lag);
    let above = sub(ctx, anchor, window.min);
    if sub(ctx, above, Word16(5)).0 > 0 {
        anchor = add(ctx, window.min, Word16(5));
    }
    let below = sub(ctx, window.max, anchor);
    if sub(ctx, below, Word16(4)).0 > 0 {
        anchor = sub(ctx, window.max, Word16(4));
    }
    anchor.0
}

/// `3v`, accumulated as the reference does.
fn triple(ctx: &mut DspContext, v: Word16) -> Word16 {
    let doubled = add(ctx, v, v);
    add(ctx, doubled, v)
}

/// Encode a one-third-resolution pitch lag — `Enc_lag3`.
///
/// `delta` selects between the absolute eight-bit code of subframes 0 and 2 and
/// the relative code of the others; `four_bit` selects the four-bit anchored
/// code that 4.75, 5.15, 5.90 and 6.70 kbit/s use in place of the five-bit one.
///
/// The absolute code splits its space at lag 85: below it the lag carries a
/// third-of-a-sample fraction, above it whole samples only. The four-bit code
/// spends its middle eight codes on the fractional region around the anchor and
/// the outer eight on whole samples, and its two branch tests are `>=` then
/// `>` — asymmetric on purpose.
///
/// # Panics
///
/// If the arguments are not a lag this coding can represent, which would make
/// the index negative.
#[must_use]
pub fn encode_lag_1_3(
    ctx: &mut DspContext,
    lag: i16,
    frac: i16,
    previous_lag: i16,
    window: LagWindow,
    delta: bool,
    four_bit: bool,
) -> u16 {
    let lag = Word16(lag);
    let frac = Word16(frac);

    let index = if !delta {
        if sub(ctx, lag, Word16(85)).0 <= 0 {
            let tripled = triple(ctx, lag);
            let biased = sub(ctx, tripled, Word16(58));
            add(ctx, biased, frac)
        } else {
            add(ctx, lag, Word16(112))
        }
    } else if four_bit {
        let anchor = Word16(four_bit_anchor(ctx, previous_lag, window));
        let tripled = triple(ctx, lag);
        let uplag = add(ctx, tripled, frac);
        let two_below = sub(ctx, anchor, Word16(2));
        let below = triple(ctx, two_below);

        if sub(ctx, below, uplag).0 >= 0 {
            let offset = sub(ctx, lag, anchor);
            add(ctx, offset, Word16(5))
        } else {
            let one_above = add(ctx, anchor, Word16(1));
            let above = triple(ctx, one_above);
            if sub(ctx, above, uplag).0 > 0 {
                let offset = sub(ctx, uplag, below);
                add(ctx, offset, Word16(3))
            } else {
                let offset = sub(ctx, lag, anchor);
                add(ctx, offset, Word16(11))
            }
        }
    } else {
        let offset = sub(ctx, lag, window.min);
        let steps = triple(ctx, offset);
        let biased = add(ctx, steps, Word16(2));
        add(ctx, biased, frac)
    };

    u16::try_from(index.0).expect("a pitch index is non-negative")
}

/// Encode a one-sixth-resolution pitch lag — `Enc_lag6`, 12.2 kbit/s only.
///
/// The absolute code splits at lag 94. The relative code uses only 61 of its 64
/// points; 61 to 63 are reserved to signal a transmission error to the decoder,
/// and this never produces them.
///
/// # Panics
///
/// If the arguments are not a lag this coding can represent, which would make
/// the index negative.
#[must_use]
pub fn encode_lag_1_6(ctx: &mut DspContext, lag: i16, frac: i16, t0_min: i16, delta: bool) -> u16 {
    let lag = Word16(lag);
    let frac = Word16(frac);

    let index = if delta {
        let offset = sub(ctx, lag, Word16(t0_min));
        let tripled = triple(ctx, offset);
        let sixfold = add(ctx, tripled, tripled);
        let biased = add(ctx, sixfold, Word16(3));
        add(ctx, biased, frac)
    } else if sub(ctx, lag, Word16(94)).0 <= 0 {
        let tripled = triple(ctx, lag);
        let sixfold = add(ctx, tripled, tripled);
        let biased = sub(ctx, sixfold, Word16(105));
        add(ctx, biased, frac)
    } else {
        add(ctx, lag, Word16(368))
    };

    u16::try_from(index.0).expect("a pitch index is non-negative")
}

// ---------------------------------------------------------------------------
// Pitch gain
// ---------------------------------------------------------------------------

/// The four correlation terms the gain quantisers need — the reference's
/// `g_coeff[0..3]`.
///
/// Written before `G_pitch`'s early return, so they are meaningful even when
/// the gain came out zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GainCoefficients {
    /// `<y1, y1>` normalised, Q15.
    pub yy: Word16,
    /// Exponent of [`yy`](Self::yy).
    pub exp_yy: Word16,
    /// `<xn, y1>` normalised, Q15.
    pub xy: Word16,
    /// Exponent of [`xy`](Self::xy).
    pub exp_xy: Word16,
}

/// Pitch (adaptive codebook) gain — `G_pitch`. Q14, clamped to 1.2.
///
/// `xn` is the pitch target and `y1` the filtered adaptive codevector, both Q0.
///
/// The **overflow flag is the branch selector**, and this is where a port with
/// a wider accumulator silently diverges: each of the two dot products is
/// computed on the unscaled vectors first, and only if `L_mac` saturated is it
/// recomputed on a quartered copy. The exponent corrections then differ —
/// **−4** for the energy, where both factors were scaled, and **−2** for the
/// cross term, where only one was. The flag must be cleared immediately before
/// each accumulation and read immediately after.
///
/// `1L` rather than `0` seeds both sums, so an all-zero input cannot divide by
/// zero. The `xy < 4` escape tests the *normalised* cross term, not the raw dot
/// product. 12.2 kbit/s clears the two low bits, because its gain carries only
/// Q12 resolution in a Q14 word.
///
/// # Panics
///
/// If either input is shorter than a subframe.
#[must_use]
pub fn pitch_gain(
    ctx: &mut DspContext,
    mode_index: u8,
    xn: &[Word16],
    y1: &[Word16],
) -> (Word16, GainCoefficients) {
    assert!(
        xn.len() >= L_SUBFR && y1.len() >= L_SUBFR,
        "a full subframe"
    );

    let mut quartered = [Word16(0); L_SUBFR];
    for (slot, &value) in quartered.iter_mut().zip(y1.iter()) {
        *slot = shr(ctx, value, 2);
    }

    ctx.overflow = false;
    let mut s = Word32(1);
    for &value in &y1[..L_SUBFR] {
        s = l_mac(ctx, s, value, value);
    }
    let (energy, energy_shift) = if ctx.overflow {
        let mut s = Word32(1);
        for &value in &quartered {
            s = l_mac(ctx, s, value, value);
        }
        let exp = norm_l(s);
        let normalised = l_shl(ctx, s, exp);
        // Both factors were quartered, so the exponent loses four.
        (round(ctx, normalised), sub(ctx, Word16(exp), Word16(4)))
    } else {
        let exp = norm_l(s);
        let normalised = l_shl(ctx, s, exp);
        (round(ctx, normalised), Word16(exp))
    };

    ctx.overflow = false;
    let mut s = Word32(1);
    for (i, &value) in y1[..L_SUBFR].iter().enumerate() {
        s = l_mac(ctx, s, xn[i], value);
    }
    let (cross, cross_shift) = if ctx.overflow {
        let mut s = Word32(1);
        for (i, &value) in quartered.iter().enumerate() {
            s = l_mac(ctx, s, xn[i], value);
        }
        let exp = norm_l(s);
        let normalised = l_shl(ctx, s, exp);
        // Only one factor was quartered here, so the correction is two.
        (round(ctx, normalised), sub(ctx, Word16(exp), Word16(2)))
    } else {
        let exp = norm_l(s);
        let normalised = l_shl(ctx, s, exp);
        (round(ctx, normalised), Word16(exp))
    };

    let coefficients = GainCoefficients {
        yy: energy,
        exp_yy: sub(ctx, Word16(15), energy_shift),
        xy: cross,
        exp_xy: sub(ctx, Word16(15), cross_shift),
    };

    if sub(ctx, cross, Word16(4)).0 < 0 {
        return (Word16(0), coefficients);
    }

    // Halving guarantees the ETSI division's `0 <= num <= den` precondition:
    // both operands were normalised into [16384, 32767].
    let numerator = shr(ctx, cross, 1);
    let mut gain = crate::fixed_point::div::div_s(numerator, energy);
    // Denormalise. The shift can be negative, in which case `shr` shifts left
    // and saturates.
    let denormalise = sub(ctx, cross_shift, energy_shift).0;
    gain = shr(ctx, gain, denormalise);

    // 1.2 in Q14.
    if sub(ctx, gain, Word16(19661)).0 > 0 {
        gain = Word16(19661);
    }
    if mode_index == MR122 {
        gain = Word16(gain.0 & !0x0003);
    }
    (gain, coefficients)
}

/// Nearest entry of the scalar pitch-gain codebook — the search inside
/// `q_gain_pitch`.
///
/// Entry 0 is the **unconditional** initial incumbent and is *not* tested
/// against `gp_limit`; later entries above the limit are skipped. The
/// comparison is strict, so ties go to the lowest index.
///
/// # Panics
///
/// Never: the codebook has sixteen entries and the index names one of them.
#[must_use]
pub fn nearest_pitch_gain(ctx: &mut DspContext, gain_limit: Word16, gain: Word16) -> u16 {
    let first = sub(ctx, gain, Word16(QUA_GAIN_PITCH[0]));
    let mut smallest = abs_s(ctx, first);
    let mut index = 0usize;
    for (i, &candidate) in QUA_GAIN_PITCH.iter().enumerate().skip(1) {
        if sub(ctx, Word16(candidate), gain_limit).0 > 0 {
            continue;
        }
        let difference = sub(ctx, gain, Word16(candidate));
        let error = abs_s(ctx, difference);
        if sub(ctx, error, smallest).0 < 0 {
            smallest = error;
            index = i;
        }
    }
    u16::try_from(index).expect("a gain index fits in 16 bits")
}

/// Quantise the pitch gain at 12.2 kbit/s — `q_gain_pitch(MR122, …)`.
///
/// 12.2 is the only rate whose pitch gain is quantised inside `cl_ltp` rather
/// than with the code gain. Returns the transmitted index and the quantised
/// gain with its **two low bits cleared** — and it is that masked value, not
/// the codebook entry, that the target update afterwards uses.
#[must_use]
pub fn quantise_pitch_gain_12k2(
    ctx: &mut DspContext,
    gain_limit: Word16,
    gain: Word16,
) -> (u16, Word16) {
    let index = nearest_pitch_gain(ctx, gain_limit, gain);
    let entry = QUA_GAIN_PITCH[usize::from(index)];
    (index, Word16(entry & !0x0003))
}

// ---------------------------------------------------------------------------
// Tone stability and gain clipping
// ---------------------------------------------------------------------------

/// The tone-stability tracker — TS 26.073 `tonStabState`.
///
/// Two independent pieces of hysteresis that both guard against the pitch
/// predictor locking onto a steady tone: a frame counter over the LSP spacing,
/// and a running mean of the recent pitch gains. Despite its reference name
/// (`gp[N_FRAME]`) the gain history holds one entry per **subframe**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToneStability {
    count: Word16,
    gains: [Word16; N_FRAME],
}

impl ToneStability {
    /// The state `ton_stab_reset` leaves behind.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            count: Word16(0),
            gains: [Word16(0); N_FRAME],
        }
    }

    /// Detect a resonance in the unquantised LSPs — `check_lsp`.
    ///
    /// Called once per frame with the **current** frame's unquantised LSPs, Q15
    /// — the reference reads `lsp_old` after `lsp()` has already copied this
    /// frame's vector into it. Returns whether the pitch gain should be watched
    /// for clipping.
    ///
    /// Twelve *consecutive* resonant frames are needed to raise the flag, and a
    /// single non-resonant frame resets the count to zero. The count saturates
    /// at twelve so the flag does not become harder to clear the longer it has
    /// been set.
    pub fn check_lsp(&mut self, ctx: &mut DspContext, lsp: &[Word16; M]) -> bool {
        // Upper band: pairs (3,4) through (7,8).
        let mut upper = Word16(MAX_16);
        for i in 3..M - 2 {
            let gap = sub(ctx, lsp[i], lsp[i + 1]);
            if sub(ctx, gap, upper).0 < 0 {
                upper = gap;
            }
        }
        // Lower band: pairs (1,2) and (2,3).
        let mut lower = Word16(MAX_16);
        for i in 1..3 {
            let gap = sub(ctx, lsp[i], lsp[i + 1]);
            if sub(ctx, gap, lower).0 < 0 {
                lower = gap;
            }
        }

        let threshold = if sub(ctx, lsp[1], Word16(32000)).0 > 0 {
            Word16(600)
        } else if sub(ctx, lsp[1], Word16(30500)).0 > 0 {
            Word16(800)
        } else {
            Word16(1100)
        };

        if sub(ctx, upper, Word16(1500)).0 < 0 || sub(ctx, lower, threshold).0 < 0 {
            self.count = add(ctx, self.count, Word16(1));
        } else {
            self.count = Word16(0);
        }

        if sub(ctx, self.count, Word16(12)).0 >= 0 {
            self.count = Word16(12);
            true
        } else {
            false
        }
    }

    /// Whether the mean of the recent pitch gains has run too high —
    /// `check_gp_clipping`.
    ///
    /// The candidate gain contributes at one eighth, matching the scale the
    /// history is stored at, so the test is effectively "mean of the last eight
    /// pitch gains above 0.95". The running sum is a Word16 with saturating
    /// adds, which can only make the test fire.
    #[must_use]
    pub fn clipping(&self, ctx: &mut DspContext, gain_pitch: Word16) -> bool {
        let mut sum = shr(ctx, gain_pitch, 3);
        for &past in &self.gains {
            sum = add(ctx, sum, past);
        }
        sub(ctx, sum, GP_CLIP).0 > 0
    }

    /// Push one subframe's gain into the history — `update_gp_clipping`.
    ///
    /// Fed the **quantised** gain, where [`clipping`](Self::clipping) tests the
    /// unquantised candidate. So the test in subframe *n* compares quantised
    /// gains from subframes *n−7*…*n−1* against an unquantised one for *n*.
    pub fn update(&mut self, ctx: &mut DspContext, gain_pitch: Word16) {
        self.gains.copy_within(1.., 0);
        self.gains[N_FRAME - 1] = shr(ctx, gain_pitch, 3);
    }

    /// The stored gain history, Q14 divided by eight.
    #[must_use]
    pub const fn history(&self) -> &[Word16; N_FRAME] {
        &self.gains
    }
}

// ---------------------------------------------------------------------------
// The closed-loop LTP driver
// ---------------------------------------------------------------------------

/// What `cl_ltp` produced for one subframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LtpResult {
    /// The chosen lag and its transmitted index.
    pub pitch: ClosedLoopResult,
    /// Pitch gain, Q14, after clipping and any per-rate cap.
    pub gain_pitch: Word16,
    /// Upper bound the gain quantiser must respect: `MAX_16` normally,
    /// [`GP_CLIP`] when clipping is active.
    pub gain_limit: Word16,
    /// Correlation terms for the gain quantiser.
    pub gain_coefficients: GainCoefficients,
    /// The quantised pitch-gain index — 12.2 kbit/s only, where the gain is
    /// quantised here rather than with the code gain.
    pub gain_index: Option<u16>,
}

/// Subtract the scaled adaptive contribution from a target — the loop that
/// closes `cl_ltp`.
///
/// `target[i] -= extract_h(L_shl(L_mult(source[i], gain), 1))`, in place. The
/// `L_shl(…, 1)` is the Q14→Q15 promotion and it saturates.
///
/// # Panics
///
/// If either slice is shorter than a subframe.
pub fn update_target(ctx: &mut DspContext, target: &mut [Word16], source: &[Word16], gain: Word16) {
    assert!(
        target.len() >= L_SUBFR && source.len() >= L_SUBFR,
        "a full subframe"
    );
    for i in 0..L_SUBFR {
        let product = l_mult(ctx, source[i], gain);
        let scaled = extract_h(l_shl(ctx, product, 1));
        target[i] = sub(ctx, target[i], scaled);
    }
}

/// Closed-loop long-term prediction for one subframe — `cl_ltp`.
///
/// Searches for the lag, builds the adaptive codevector in place in
/// `excitation`, filters it into `y1`, computes the pitch gain, applies the
/// clipping and per-rate limits, and updates both targets.
///
/// `excitation`'s current subframe must already hold the LP residual on entry —
/// [`normalised_correlation`] reads it. It holds the adaptive codevector on
/// return, which is what the gain quantiser and the excitation update both
/// expect.
///
/// `resonant` is [`ToneStability::check_lsp`]'s once-per-frame verdict. Note the
/// clipping test runs on the **unclipped** gain, before the 4.75/5.15 cap, and
/// that those two rates cap the gain at 0.85 but never force it down to
/// [`GP_CLIP`] the way every other rate does.
///
/// # Panics
///
/// If the excitation buffer is too short for the window the search reaches.
#[allow(clippy::too_many_arguments)]
pub fn closed_loop_ltp(
    ctx: &mut DspContext,
    mode_index: u8,
    pitch: &mut ClosedLoopPitch,
    tone: &mut ToneStability,
    open_loop: [i16; 2],
    subframe: usize,
    excitation: &mut Excitation,
    xn: &[Word16; L_SUBFR],
    h1: &[Word16; L_SUBFR],
    resonant: bool,
    res2: &mut [Word16; L_SUBFR],
    xn2: &mut [Word16; L_SUBFR],
    y1: &mut [Word16; L_SUBFR],
) -> LtpResult {
    let found = pitch.search(
        ctx,
        mode_index,
        open_loop,
        excitation.all(),
        EXC_ORIGIN,
        xn,
        h1,
        subframe,
    );

    excitation.predict(ctx, found.lag);
    let adaptive: [Word16; L_SUBFR] = excitation
        .subframe()
        .try_into()
        .expect("the excitation subframe is L_SUBFR long");
    *y1 = convolve(ctx, &adaptive, h1);

    let (mut gain_pitch, gain_coefficients) = pitch_gain(ctx, mode_index, xn, y1);

    let mut gain_limit = Word16(MAX_16);
    // Evaluated on the *unclipped* gain, before the per-rate cap below.
    let clipped = resonant && sub(ctx, gain_pitch, GP_CLIP).0 > 0 && tone.clipping(ctx, gain_pitch);

    let mut gain_index = None;
    if matches!(mode_index, MR475 | MR515) {
        // 0.85 in Q14, unconditionally — "to cope with bit errors in the
        // decoder in a better way".
        if sub(ctx, gain_pitch, Word16(13926)).0 > 0 {
            gain_pitch = Word16(13926);
        }
        if clipped {
            gain_limit = GP_CLIP;
        }
    } else {
        if clipped {
            gain_limit = GP_CLIP;
            gain_pitch = GP_CLIP;
        }
        if mode_index == MR122 {
            let (index, quantised) = quantise_pitch_gain_12k2(ctx, gain_limit, gain_pitch);
            gain_pitch = quantised;
            gain_index = Some(index);
        }
    }

    xn2.copy_from_slice(xn);
    update_target(ctx, xn2, y1, gain_pitch);
    update_target(ctx, res2, &adaptive, gain_pitch);

    LtpResult {
        pitch: found,
        gain_pitch,
        gain_limit,
        gain_coefficients,
        gain_index,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::bitstream::parse;
    use super::super::super::lag::{
        absolute_lag, delta_coding, delta_lag_1_3, delta_lag_1_6, delta_window,
    };
    use super::*;

    /// The committed 7.40 kbit/s encoder trace, three frames.
    const TRACE: &str = include_str!("../../testdata/nb_enc_trace.txt");

    /// TS 26.073's own 7.40 kbit/s output for `testdata/amrnb_enc_input.pcm`.
    const MODE4: &[u8] = include_bytes!("../../testdata/amrnb_enc_mode4.amr");

    /// 7.40 kbit/s.
    const MR74: u8 = 4;

    /// Frames the trace covers.
    const FRAMES: usize = 3;

    /// Subframes per frame.
    const SUBFRAMES: usize = 4;

    /// Which transmitted parameter carries each subframe's pitch index at
    /// 7.40 kbit/s: three LSF indices, then lag, positions, signs, gain.
    const LAG_PARAM: [usize; SUBFRAMES] = [3, 7, 11, 15];

    /// One trace row's values.
    ///
    /// Panics rather than returning `None`: a comparison that silently never
    /// happens reads exactly like agreement.
    fn trace(frame: usize, subframe: usize, name: &str) -> Vec<i16> {
        let want = format!("T {frame} {subframe} {name} ");
        for line in TRACE.lines() {
            if let Some(rest) = line.strip_prefix(&want) {
                return rest
                    .split_whitespace()
                    .map(|v| v.parse().expect("trace value fits in i16"))
                    .collect();
            }
        }
        panic!("the committed trace has no row {want:?}");
    }

    fn vector(frame: usize, subframe: usize, name: &str) -> [Word16; L_SUBFR] {
        let v = trace(frame, subframe, name);
        assert_eq!(v.len(), L_SUBFR, "{name} is one subframe long");
        let mut out = [Word16(0); L_SUBFR];
        for (slot, value) in out.iter_mut().zip(v) {
            *slot = Word16(value);
        }
        out
    }

    fn scalar(frame: usize, subframe: usize, name: &str) -> i16 {
        let v = trace(frame, subframe, name);
        assert_eq!(v.len(), 1, "{name} is a scalar");
        v[0]
    }

    /// A transmitted index as the decoder's `Word16`.
    fn code(index: u16) -> Word16 {
        Word16(i16::try_from(index).expect("a pitch index fits in i16"))
    }

    fn reference_parameters(frame: usize) -> Vec<u16> {
        const PAYLOAD: usize = 19;
        let offset = 6 + frame * (1 + PAYLOAD);
        assert_eq!((MODE4[offset] >> 3) & 0x0f, MR74, "frame {frame}: ToC mode");
        parse(MR74, &MODE4[offset + 1..offset + 1 + PAYLOAD]).expect("frame parses")
    }

    /// Replay the encoder's excitation buffer from the traced adaptive
    /// codevector, algebraic code and quantised gains.
    ///
    /// The trace does not record the excitation, but it records everything the
    /// excitation is built from, and `spstproc.c` builds it as
    /// `round(L_shl(gain_pit·exc + gain_code·code, 1))` at every rate but 12.2.
    /// [`the_replayed_excitation_reproduces_the_traced_codevector`] checks the
    /// replay against `adapt` before any other test relies on it.
    struct Replay {
        excitation: Excitation,
        ctx: DspContext,
    }

    impl Replay {
        fn new() -> Self {
            Self {
                excitation: Excitation::new(),
                ctx: DspContext::default(),
            }
        }

        /// Seed the subframe region with the LP residual, as `subframePreProc`
        /// does, and hand back the buffer as the search will see it.
        fn open_subframe(&mut self, frame: usize, subframe: usize) {
            let res = vector(frame, subframe, "res");
            self.excitation.subframe_mut().copy_from_slice(&res);
        }

        /// Apply the traced lag, code and gains, then step the history on.
        fn close_subframe(&mut self, frame: usize, subframe: usize) {
            let lag = PitchLag {
                integer: Word16(scalar(frame, subframe, "T0")),
                frac: Word16(scalar(frame, subframe, "T0_frac")),
                resolution: LagResolution::OneThird,
            };
            self.excitation.predict(&mut self.ctx, lag);
            assert_eq!(
                self.excitation.subframe(),
                vector(frame, subframe, "adapt"),
                "frame {frame} subframe {subframe}: replayed adaptive codevector"
            );

            let code = vector(frame, subframe, "code");
            let gain_pitch = Word16(scalar(frame, subframe, "gain_pit"));
            let gain_code = Word16(scalar(frame, subframe, "gain_code"));
            for (i, &pulse) in code.iter().enumerate() {
                let mut acc = l_mult(&mut self.ctx, self.excitation.subframe()[i], gain_pitch);
                acc = l_mac(&mut self.ctx, acc, pulse, gain_code);
                acc = l_shl(&mut self.ctx, acc, 1);
                let total = round(&mut self.ctx, acc);
                self.excitation.subframe_mut()[i] = total;
            }
            self.excitation.advance();
        }
    }

    #[test]
    fn the_replayed_excitation_reproduces_the_traced_codevector() {
        // Everything that follows depends on this replay being the excitation
        // the reference actually had. `Pred_lt_3or6` reads only the history, so
        // reproducing `adapt` for twelve consecutive subframes — starting from
        // silence and never resynchronising — pins the whole buffer.
        let mut replay = Replay::new();
        let mut compared = 0;
        for frame in 0..FRAMES {
            for subframe in 0..SUBFRAMES {
                replay.open_subframe(frame, subframe);
                // The assertion lives in close_subframe, where the codevector
                // exists.
                replay.close_subframe(frame, subframe);
                compared += 1;
            }
        }
        assert_eq!(compared, 12, "three frames of four subframes");
    }

    #[test]
    fn convolve_matches_the_trace() {
        let mut ctx = DspContext::default();
        let mut compared = 0;
        for frame in 0..FRAMES {
            for subframe in 0..SUBFRAMES {
                let got = convolve(
                    &mut ctx,
                    &vector(frame, subframe, "adapt"),
                    &vector(frame, subframe, "h1"),
                );
                assert_eq!(
                    got,
                    vector(frame, subframe, "y1"),
                    "frame {frame} subframe {subframe}: filtered adaptive codevector"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 12);
    }

    #[test]
    fn the_pitch_gain_matches_the_trace() {
        // `gain_pit_ol` is traced just before the codebook search, i.e. after
        // cl_ltp's clipping logic. The committed frames are the first three of
        // the stream, so `check_lsp`'s twelve-frame counter cannot have fired
        // and the traced value is G_pitch's own output.
        let mut ctx = DspContext::default();
        let mut compared = 0;
        for frame in 0..FRAMES {
            for subframe in 0..SUBFRAMES {
                let (gain, _) = pitch_gain(
                    &mut ctx,
                    MR74,
                    &vector(frame, subframe, "xn"),
                    &vector(frame, subframe, "y1"),
                );
                assert_eq!(
                    gain.0,
                    scalar(frame, subframe, "gain_pit_ol"),
                    "frame {frame} subframe {subframe}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 12);
    }

    #[test]
    fn the_target_update_matches_the_trace() {
        let mut ctx = DspContext::default();
        let mut compared = 0;
        for frame in 0..FRAMES {
            for subframe in 0..SUBFRAMES {
                let mut xn2 = vector(frame, subframe, "xn");
                update_target(
                    &mut ctx,
                    &mut xn2,
                    &vector(frame, subframe, "y1"),
                    Word16(scalar(frame, subframe, "gain_pit_ol")),
                );
                assert_eq!(
                    xn2,
                    vector(frame, subframe, "xn2"),
                    "frame {frame} subframe {subframe}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 12);
    }

    #[test]
    fn the_closed_loop_search_matches_the_trace_on_delta_subframes() {
        // Subframes 1 and 3 derive their window from the previous subframe's
        // lag, which the trace gives, so the whole search — Norm_Corr, the
        // integer scan, searchFrac and its post-normalisation — can be run
        // against the reference with no open-loop input at all.
        let mut replay = Replay::new();
        let mut pitch = ClosedLoopPitch::new();
        let mut ctx = DspContext::default();
        let mut compared = 0;

        for frame in 0..FRAMES {
            for (subframe, &slot) in LAG_PARAM.iter().enumerate() {
                replay.open_subframe(frame, subframe);
                if !subframe.is_multiple_of(2) {
                    let found = pitch.search(
                        &mut ctx,
                        MR74,
                        [0, 0], // unused on a delta subframe
                        replay.excitation.all(),
                        EXC_ORIGIN,
                        &vector(frame, subframe, "xn"),
                        &vector(frame, subframe, "h1"),
                        subframe,
                    );
                    assert!(found.delta, "subframe {subframe} is delta-coded at 7.40");
                    assert_eq!(
                        (found.lag.integer.0, found.lag.frac.0),
                        (
                            scalar(frame, subframe, "T0"),
                            scalar(frame, subframe, "T0_frac")
                        ),
                        "frame {frame} subframe {subframe}: window {}..={}",
                        found.window.min.0,
                        found.window.max.0
                    );
                    assert_eq!(
                        u32::from(found.index),
                        u32::from(reference_parameters(frame)[slot]),
                        "frame {frame} subframe {subframe}: transmitted lag index"
                    );
                    compared += 1;
                }
                // The state the next delta search needs is the *traced* lag,
                // which the search above has just been shown to reproduce.
                pitch.previous_lag = Word16(scalar(frame, subframe, "T0"));
                replay.close_subframe(frame, subframe);
            }
        }
        assert_eq!(compared, 6, "two delta subframes in each of three frames");
    }

    #[test]
    fn some_open_loop_lag_reproduces_each_full_search_subframe() {
        // The trace records no open-loop lag at 7.40 kbit/s — the reference
        // only emits that row on the 4.75/5.15 path — and `Pitch_ol`'s input,
        // the weighted speech, is not traced either. What *can* be checked is
        // that the closed-loop search reproduces the reference's result for
        // some open-loop lag, which a broken Norm_Corr or integer scan would
        // not manage for any. In frame 0 the answer is unique.
        let mut replay = Replay::new();
        let mut ctx = DspContext::default();
        let mut compared = 0;

        for frame in 0..FRAMES {
            for subframe in 0..SUBFRAMES {
                replay.open_subframe(frame, subframe);
                if subframe % 2 == 0 {
                    let want = (
                        scalar(frame, subframe, "T0"),
                        scalar(frame, subframe, "T0_frac"),
                    );
                    let consistent: Vec<i16> = (PIT_MIN..=PIT_MAX)
                        .filter(|&candidate| {
                            let mut probe = ClosedLoopPitch::new();
                            let found = probe.search(
                                &mut ctx,
                                MR74,
                                [candidate, candidate],
                                replay.excitation.all(),
                                EXC_ORIGIN,
                                &vector(frame, subframe, "xn"),
                                &vector(frame, subframe, "h1"),
                                subframe,
                            );
                            assert!(!found.delta, "subframe {subframe} is a full search");
                            (found.lag.integer.0, found.lag.frac.0) == want
                        })
                        .collect();
                    assert!(
                        !consistent.is_empty(),
                        "frame {frame} subframe {subframe}: no open-loop lag reproduces \
                         T0={} frac={}",
                        want.0,
                        want.1
                    );
                    if frame == 0 && subframe == 0 {
                        assert_eq!(
                            consistent,
                            vec![27],
                            "the first subframe pins the open-loop lag exactly"
                        );
                    }
                    compared += 1;
                }
                replay.close_subframe(frame, subframe);
            }
        }
        assert_eq!(
            compared, 6,
            "two full-search subframes in each of three frames"
        );
    }

    #[test]
    fn every_lag_index_matches_the_reference_bitstream() {
        // Both code paths of Enc_lag3 against what TS 26.073 actually
        // transmitted, using the traced lags so a closed-loop disagreement
        // cannot mask an encoding one.
        let mut ctx = DspContext::default();
        let mut previous = Word16(0);
        let mut compared = 0;

        for frame in 0..FRAMES {
            let params = reference_parameters(frame);
            for (subframe, &slot) in LAG_PARAM.iter().enumerate() {
                let lag = scalar(frame, subframe, "T0");
                let frac = scalar(frame, subframe, "T0_frac");
                let delta = !subframe.is_multiple_of(2);
                let window = lag_window(&mut ctx, previous.0, 5, 9, PIT_MIN);
                let index = encode_lag_1_3(&mut ctx, lag, frac, previous.0, window, delta, false);
                assert_eq!(
                    u32::from(index),
                    u32::from(params[slot]),
                    "frame {frame} subframe {subframe}"
                );
                previous = Word16(lag);
                compared += 1;
            }
        }
        assert_eq!(compared, 12);
    }

    #[test]
    fn lag_encoding_round_trips_through_the_decoder() {
        // The decoder's lag path is bit-exact against TS 26.073, so encoding
        // then decoding must be the identity across every code either side can
        // produce. This is what catches an off-by-one in the four-bit path's
        // asymmetric `>=` / `>` boundaries, which no single trace frame would.
        let mut ctx = DspContext::default();
        let mut compared = 0;

        // Absolute, one-third: every lag and fraction the encoder can emit.
        for lag in PIT_MIN..=PIT_MAX {
            let fracs: &[i16] = if lag <= 84 { &[-1, 0, 1] } else { &[0] };
            for &frac in fracs {
                let window = LagWindow {
                    min: Word16(PIT_MIN),
                    max: Word16(PIT_MAX),
                };
                let index = encode_lag_1_3(&mut ctx, lag, frac, 0, window, false, false);
                let back = absolute_lag(&mut ctx, code(index), LagResolution::OneThird);
                assert_eq!((back.integer.0, back.frac.0), (lag, frac), "absolute 1/3");
                compared += 1;
            }
        }

        // Delta, one-third, five-bit (7.40 kbit/s) and four-bit (6.70).
        for previous in PIT_MIN..=PIT_MAX {
            let uniform = delta_window(&mut ctx, MR74, Word16(previous));
            for lag in uniform.min.0..=uniform.max.0 {
                for frac in -1..=1 {
                    let index = encode_lag_1_3(&mut ctx, lag, frac, previous, uniform, true, false);
                    let back = delta_lag_1_3(
                        &mut ctx,
                        code(index),
                        uniform,
                        delta_coding(MR74, Word16(previous)),
                    );
                    assert_eq!(
                        (back.integer.0, back.frac.0),
                        (lag, frac),
                        "delta 1/3, 5 bit"
                    );
                    compared += 1;
                }
            }

            // The four-bit code is not onto the (lag, fraction) grid — only
            // the middle eight codes carry a fraction, and a lag two below the
            // anchor cannot carry a negative one at all. So the statement that
            // holds is the other direction: every code the decoder can receive
            // must survive a round trip through the encoder. That is a
            // bijection on the code space, which is exactly what a wrong
            // boundary in the `>=` / `>` pair would break.
            let anchored = delta_window(&mut ctx, MR67, Word16(previous));
            let anchor = four_bit_anchor(&mut ctx, previous, anchored);
            for index in 0..16u16 {
                let decoded = delta_lag_1_3(
                    &mut ctx,
                    code(index),
                    anchored,
                    delta_coding(MR67, Word16(previous)),
                );
                let again = encode_lag_1_3(
                    &mut ctx,
                    decoded.integer.0,
                    decoded.frac.0,
                    previous,
                    anchored,
                    true,
                    true,
                );
                assert_eq!(
                    again, index,
                    "delta 1/3, 4 bit: previous {previous}, anchor {anchor}, lag {} frac {}",
                    decoded.integer.0, decoded.frac.0
                );
                compared += 1;
            }
        }

        // One-sixth, both branches.
        for lag in PIT_MIN_MR122..=PIT_MAX {
            let fracs: &[i16] = if lag <= 94 {
                &[-2, -1, 0, 1, 2, 3]
            } else {
                &[0]
            };
            for &frac in fracs {
                let index = encode_lag_1_6(&mut ctx, lag, frac, 0, false);
                let back = absolute_lag(&mut ctx, code(index), LagResolution::OneSixth);
                assert_eq!((back.integer.0, back.frac.0), (lag, frac), "absolute 1/6");
                compared += 1;
            }
        }
        for previous in PIT_MIN_MR122..=PIT_MAX {
            let window = lag_window(&mut ctx, previous, 5, 9, PIT_MIN_MR122);
            for lag in window.min.0..=window.max.0 {
                for frac in -2..=3 {
                    let index = encode_lag_1_6(&mut ctx, lag, frac, window.min.0, true);
                    assert!(index < 61, "only 61 of the 64 relative codes are used");
                    let back = delta_lag_1_6(&mut ctx, code(index), Word16(previous));
                    assert_eq!((back.integer.0, back.frac.0), (lag, frac), "delta 1/6");
                    compared += 1;
                }
            }
        }

        assert!(compared > 9000, "only {compared} lag codes round-tripped");
    }

    #[test]
    fn the_open_loop_peak_takes_the_smallest_lag_on_a_tie() {
        let mut ctx = DspContext::default();
        let mut corr = [Word32(0); PIT_MAX as usize + 1];
        // Three lags share the maximum; the descending walk with `>=` must
        // land on the smallest of them.
        corr[40] = Word32(1000);
        corr[50] = Word32(1000);
        corr[60] = Word32(1000);
        assert_eq!(peak_lag(&mut ctx, &corr, 79, 40), 40);

        // Restricting the range to exclude 40 moves the answer to 50, which
        // shows the walk really visits every lag rather than stopping early.
        assert_eq!(peak_lag(&mut ctx, &corr, 79, 41), 50);
    }

    #[test]
    fn an_all_minimum_section_returns_its_lowest_lag() {
        // `max` starts at MIN_32 and the comparison is non-strict, so the last
        // lag visited satisfies it even when every correlation is MIN_32. A
        // strict comparison would return `lag_max` instead.
        let mut ctx = DspContext::default();
        let corr = [Word32(MIN_32); PIT_MAX as usize + 1];
        assert_eq!(peak_lag(&mut ctx, &corr, 79, 40), 40);
    }

    #[test]
    fn the_section_arbitration_keeps_the_incumbent_on_a_tie() {
        let mut ctx = DspContext::default();
        // A short-lag section exactly equal to the long one loses: it must
        // strictly exceed 0.85 of it, and 0.85·x < x for positive x — so this
        // is a genuine tie only at the boundary. Check both sides of it.
        let long = (100i16, Word16(1000));
        let handicapped = mult(&mut ctx, Word16(1000), THRESHOLD);
        assert_eq!(
            arbitrate_sections(&mut ctx, long, (60, handicapped), (30, Word16(0))),
            100,
            "equalling the handicapped incumbent is not enough"
        );
        assert_eq!(
            arbitrate_sections(
                &mut ctx,
                long,
                (60, Word16(handicapped.0 + 1)),
                (30, Word16(0))
            ),
            60,
            "one more than the handicap takes it"
        );
        // The third test compares against whatever survived the second.
        assert_eq!(
            arbitrate_sections(
                &mut ctx,
                long,
                (60, Word16(handicapped.0 + 1)),
                (30, Word16(handicapped.0 + 2))
            ),
            30,
            "section three is measured against the new incumbent"
        );
    }

    #[test]
    fn the_arbitration_handicap_floors_rather_than_rounds() {
        // `mult` floors toward −∞, which for a negative incumbent makes the
        // threshold stricter than a rounding multiply would. The open-loop
        // correlations really can be negative: for every rate but 12.2 they are
        // the low half-word of a 32-bit product.
        let mut ctx = DspContext::default();
        let negative = Word16(-1000);
        let handicapped = mult(&mut ctx, negative, THRESHOLD);
        assert_eq!(handicapped.0, -851, "floor(-1000 * 27853 / 32768)");
        // A C-style truncating divide gives -850, one greater, so the strict
        // comparison against it would admit a section this one rejects.
        assert_eq!(-1000i32 * 27853 / 32768, -850);
    }

    #[test]
    fn the_open_loop_finds_a_planted_period() {
        // A signal that repeats exactly every 45 samples. The correlation peak
        // is unambiguous, so the answer does not depend on any tie-break, and
        // the 0.85 handicap is what keeps the search off the sub-multiples.
        const PERIOD: usize = 45;

        let mut ctx = DspContext::default();
        let origin = PIT_MAX as usize;
        let mut signal = vec![Word16(0); origin + L_FRAME];
        for (i, slot) in signal.iter_mut().enumerate() {
            // A short burst per period: broadband, so its autocorrelation has
            // one clear peak per period rather than a sinusoid's many.
            *slot = Word16(match i % PERIOD {
                0 => 8000,
                1 => -6000,
                2 => 3000,
                _ => 0,
            });
        }
        assert_eq!(
            open_loop_lag(&mut ctx, MR74, &signal, origin, L_FRAME_BY2, None, false),
            i16::try_from(PERIOD).expect("period fits"),
        );
    }

    #[test]
    fn the_two_lowest_rates_search_once_for_the_whole_frame() {
        // 4.75 and 5.15 kbit/s take an outer branch that a flattened dispatch
        // would miss, searching 160 samples once and copying the result. Every
        // other rate searches each half separately — so a signal whose period
        // changes at the half-frame boundary must give two different lags there
        // and one repeated lag here.
        let mut ctx = DspContext::default();
        let origin = PIT_MAX as usize;
        let mut signal = vec![Word16(0); origin + L_FRAME];
        for (i, slot) in signal.iter_mut().enumerate() {
            let period = if i < origin + L_FRAME_BY2 { 45 } else { 30 };
            *slot = Word16(match i % period {
                0 => 8000,
                1 => -6000,
                2 => 3000,
                _ => 0,
            });
        }

        let mut weighted = WeightedOpenLoop::new();
        let mut old_lags = [Word16(40); 5];
        let mut voiced = [false; 2];

        let split = open_loop_lags(
            &mut ctx,
            MR74,
            &mut weighted,
            &signal,
            origin,
            &mut old_lags,
            &mut voiced,
            None,
        );
        assert_ne!(split[0], split[1], "each half is searched on its own");

        let single = open_loop_lags(
            &mut ctx,
            MR475,
            &mut weighted,
            &signal,
            origin,
            &mut old_lags,
            &mut voiced,
            None,
        );
        assert_eq!(single[0], single[1], "one search, copied into both halves");
    }

    #[test]
    fn twelve_two_reaches_a_shorter_lag_than_the_other_rates() {
        // 12.2 kbit/s alone searches down to 18, and alone rescales the
        // normalised correlation instead of taking its low half-word. A signal
        // with a period of 19 is reachable only there.
        let mut ctx = DspContext::default();
        let origin = PIT_MAX as usize;
        let mut signal = vec![Word16(0); origin + L_FRAME];
        for (i, slot) in signal.iter_mut().enumerate() {
            *slot = Word16(match i % 19 {
                0 => 8000,
                1 => -6000,
                _ => 0,
            });
        }
        assert_eq!(
            open_loop_lag(&mut ctx, MR122, &signal, origin, L_FRAME_BY2, None, false),
            19
        );
        assert_eq!(mode_params(MR122).pit_min, 18);
        assert_eq!(mode_params(MR74).pit_min, 20);
    }

    #[test]
    fn the_fractional_search_keeps_its_starting_point_on_a_tie() {
        // A zero correlation interpolates to zero at every phase, so the strict
        // `>` leaves the incoming fraction in place — and the one-third
        // post-normalisation then rewrites −2 as +1 one sample lower. Both
        // one-third tests are sequential, so a search that ended on +2 lands on
        // −1 one sample higher.
        let mut ctx = DspContext::default();
        let flat = Correlations {
            first: 20,
            last: 40,
            values: [Word16(0); 40],
        };
        assert_eq!(search_fraction(&mut ctx, 30, -2, 2, &flat, true), (29, 1));
        // Starting at +2 there is nothing left to try, so it stays and moves up.
        assert_eq!(search_fraction(&mut ctx, 30, 2, 2, &flat, true), (31, -1));
        // One-sixth normalises only −3.
        assert_eq!(search_fraction(&mut ctx, 30, -3, 3, &flat, false), (29, 3));
        assert_eq!(search_fraction(&mut ctx, 30, 2, 3, &flat, false), (30, 2));

        // A *constant non-zero* correlation is not a tie: the interpolation
        // filter's taps sum differently at each phase, and phase zero is its
        // peak. Worth pinning, because a "flat means tied" assumption reads as
        // reasonable and is not.
        let level = Correlations {
            first: 20,
            last: 40,
            values: [Word16(1234); 40],
        };
        assert_eq!(search_fraction(&mut ctx, 30, -2, 2, &level, true), (30, 0));
    }

    #[test]
    fn the_window_clamp_preserves_its_width_at_the_top() {
        let mut ctx = DspContext::default();
        // Clamping the top drags the bottom down so the width survives.
        let high = lag_window(&mut ctx, 143, 5, 9, PIT_MIN);
        assert_eq!((high.min.0, high.max.0), (134, 143));
        // Clamping the bottom does not, so this is the one narrow window.
        let low = lag_window(&mut ctx, 20, 5, 9, PIT_MIN);
        assert_eq!((low.min.0, low.max.0), (20, 29));
        // 12.2 reaches one lower still.
        let deep = lag_window(&mut ctx, 20, 5, 9, PIT_MIN_MR122);
        assert_eq!((deep.min.0, deep.max.0), (18, 27));
    }

    #[test]
    fn the_four_bit_anchor_applies_both_clamps_in_order() {
        let mut ctx = DspContext::default();
        let window = LagWindow {
            min: Word16(30),
            max: Word16(39),
        };
        // Far above the window: the first clamp pulls it to min+5 = 35, and
        // the second leaves it there because max − 35 = 4.
        assert_eq!(four_bit_anchor(&mut ctx, 100, window), 35);
        // Far below: the first clamp does nothing, the second raises it to
        // max − 4 = 35.
        assert_eq!(four_bit_anchor(&mut ctx, 20, window), 35);
        // A wide window shows the second clamp undoing the first.
        let wide = LagWindow {
            min: Word16(30),
            max: Word16(49),
        };
        assert_eq!(four_bit_anchor(&mut ctx, 100, wide), 45);
    }

    #[test]
    fn the_gain_search_skips_entries_above_the_limit_but_never_entry_zero() {
        let mut ctx = DspContext::default();
        // With the limit below every entry, only entry 0 — the unconditional
        // incumbent — survives, however far it is from the target.
        assert_eq!(nearest_pitch_gain(&mut ctx, Word16(0), Word16(16384)), 0);
        // Unrestricted, the nearest entry wins.
        let free = nearest_pitch_gain(&mut ctx, Word16(MAX_16), Word16(16384));
        assert!(free > 0, "the codebook is not degenerate");
        // Clipping caps it.
        let capped = nearest_pitch_gain(&mut ctx, GP_CLIP, Word16(MAX_16));
        assert!(
            QUA_GAIN_PITCH[usize::from(capped)] <= GP_CLIP.0,
            "the chosen gain respects the limit"
        );
    }

    #[test]
    fn twelve_two_clears_the_two_low_bits_of_its_pitch_gain() {
        let mut ctx = DspContext::default();
        let (index, gain) = quantise_pitch_gain_12k2(&mut ctx, Word16(MAX_16), Word16(12345));
        assert_eq!(gain.0, QUA_GAIN_PITCH[usize::from(index)] & !0x0003);
        assert_eq!(gain.0 & 0x0003, 0);
        // G_pitch masks its own output the same way, and only at 12.2.
        let flat = [Word16(1000); L_SUBFR];
        let (masked, _) = pitch_gain(&mut ctx, MR122, &flat, &flat);
        assert_eq!(masked.0 & 0x0003, 0);
    }

    #[test]
    fn the_resonance_flag_needs_twelve_consecutive_frames() {
        let mut ctx = DspContext::default();
        let mut tone = ToneStability::new();
        // Ten equally spaced LSPs: the upper-band gaps are far below 1500, so
        // every frame counts as resonant.
        let mut resonant = [Word16(0); M];
        for (i, slot) in resonant.iter_mut().enumerate() {
            *slot = Word16(30000 - 100 * i16::try_from(i).expect("ten fits"));
        }
        for frame in 0..11 {
            assert!(
                !tone.check_lsp(&mut ctx, &resonant),
                "frame {frame} fired early"
            );
        }
        assert!(
            tone.check_lsp(&mut ctx, &resonant),
            "the twelfth frame sets it"
        );
        assert!(tone.check_lsp(&mut ctx, &resonant), "and it stays set");

        // A single wide-spaced frame clears the counter outright.
        let mut spread = [Word16(0); M];
        for (i, slot) in spread.iter_mut().enumerate() {
            let step = 6000 * i32::try_from(i).expect("ten fits");
            *slot = Word16(i16::try_from(30000 - step).expect("stays inside a Word16"));
        }
        assert!(!tone.check_lsp(&mut ctx, &spread));
        for _ in 0..11 {
            assert!(!tone.check_lsp(&mut ctx, &resonant), "the count restarted");
        }
        assert!(tone.check_lsp(&mut ctx, &resonant));
    }

    #[test]
    fn gain_clipping_averages_the_last_eight_subframes() {
        let mut ctx = DspContext::default();
        let mut tone = ToneStability::new();
        // A fresh tracker has no history, so only a gain of more than
        // 8 × GP_CLIP could fire the test — and Q14 cannot hold that.
        assert!(!tone.clipping(&mut ctx, Word16(MAX_16)));

        // Seven subframes at 1.2 (the G_pitch ceiling) put the mean above 0.95.
        for _ in 0..N_FRAME {
            tone.update(&mut ctx, Word16(19661));
        }
        assert_eq!(tone.history()[0].0, 19661 >> 3);
        assert!(tone.clipping(&mut ctx, Word16(19661)));

        // The history really is a shift register: seven quiet subframes clear
        // it again.
        for _ in 0..N_FRAME {
            tone.update(&mut ctx, Word16(0));
        }
        assert_eq!(tone.history(), &[Word16(0); N_FRAME]);
        assert!(!tone.clipping(&mut ctx, Word16(19661)));
    }

    #[test]
    fn the_median_takes_the_middle_of_five() {
        let mut ctx = DspContext::default();
        assert_eq!(
            median_of_five(
                &mut ctx,
                &[Word16(10), Word16(50), Word16(20), Word16(40), Word16(30)]
            )
            .0,
            30
        );
        // Ties: the ranking prefers the highest index, which decides which of
        // two equal values the middle rank names — the value is the same
        // either way, but the rank it comes from is not.
        assert_eq!(median_of_five(&mut ctx, &[Word16(7); 5]).0, 7);
    }

    #[test]
    fn clipping_forces_the_gain_down_at_every_rate_but_the_two_lowest() {
        // The committed three frames never reach a resonance, so the branch is
        // driven directly here. 4.75 and 5.15 kbit/s cap the gain at 0.85 and
        // only *report* the clip through `gain_limit`; every other rate forces
        // the gain itself down to GP_CLIP. A 50-frame run of the regenerated
        // trace exercises this eight times and agrees with TS 26.073 on each.
        let run = |mode_index: u8, resonant: bool| {
            let mut ctx = DspContext::default();
            let mut pitch = ClosedLoopPitch::new();
            let mut tone = ToneStability::new();
            // Seven subframes at the G_pitch ceiling put the running mean over
            // the threshold, so the next candidate above GP_CLIP clips.
            for _ in 0..N_FRAME {
                tone.update(&mut ctx, Word16(19661));
            }

            let mut excitation = Excitation::new();
            // A strongly periodic history, so the adaptive codevector predicts
            // the target well and the raw gain lands at the 1.2 ceiling.
            for (i, slot) in excitation.all_mut().iter_mut().enumerate() {
                *slot = Word16(if i % 40 == 0 { 8000 } else { 0 });
            }
            let xn: [Word16; L_SUBFR] = excitation.all()[EXC_ORIGIN - 40..EXC_ORIGIN]
                .try_into()
                .expect("one subframe");
            let mut h1 = [Word16(0); L_SUBFR];
            h1[0] = Word16(4096);

            let mut res2 = xn;
            let mut xn2 = [Word16(0); L_SUBFR];
            let mut y1 = [Word16(0); L_SUBFR];
            closed_loop_ltp(
                &mut ctx,
                mode_index,
                &mut pitch,
                &mut tone,
                [40, 40],
                0,
                &mut excitation,
                &xn,
                &h1,
                resonant,
                &mut res2,
                &mut xn2,
                &mut y1,
            )
        };

        // Without the resonance flag the clipping test is never even consulted.
        let quiet = run(MR74, false);
        assert_eq!(quiet.gain_limit.0, MAX_16, "no limit reported");
        assert!(
            quiet.gain_pitch.0 > GP_CLIP.0,
            "the raw gain is above the clip"
        );

        let clipped = run(MR74, true);
        assert_eq!(clipped.gain_limit, GP_CLIP);
        assert_eq!(clipped.gain_pitch, GP_CLIP, "7.40 forces the gain down");

        let low = run(MR475, true);
        assert_eq!(low.gain_limit, GP_CLIP, "4.75 reports the limit");
        assert_eq!(
            low.gain_pitch.0, 13926,
            "and caps at 0.85 rather than at GP_CLIP"
        );

        // 12.2 quantises the pitch gain here rather than with the code gain.
        let fine = run(MR122, false);
        let index = fine
            .gain_index
            .expect("12.2 quantises its pitch gain in cl_ltp");
        assert_eq!(
            fine.gain_pitch.0,
            QUA_GAIN_PITCH[usize::from(index)] & !0x0003
        );
        assert!(run(MR74, false).gain_index.is_none(), "no other rate does");
    }

    #[test]
    fn the_weighted_open_loop_updates_its_lag_history() {
        // 10.2 kbit/s only. Exercises both cursors into CORR_WEIGHT at the
        // extremes of the state invariant that keeps them in bounds, and both
        // arms of the confidence update.
        let mut ctx = DspContext::default();
        let origin = PIT_MAX as usize;
        let mut signal = vec![Word16(0); origin + L_FRAME];
        for (i, slot) in signal.iter_mut().enumerate() {
            *slot = Word16(match i % 45 {
                0 => 8000,
                1 => -6000,
                2 => 3000,
                _ => 0,
            });
        }

        let mut weighted = WeightedOpenLoop::new();
        let mut old_lags = [Word16(40); 5];
        let mut voiced = [false; 2];
        let first = open_loop_lags(
            &mut ctx,
            MR102,
            &mut weighted,
            &signal,
            origin,
            &mut old_lags,
            &mut voiced,
            None,
        );
        assert_eq!(first[0], 45, "the planted period survives the weighting");
        assert!(voiced[0] && voiced[1], "a periodic signal is voiced");
        assert_eq!(old_lags[0].0, 45, "the lag joined the history");

        // Voiced twice running arms the proximity weighting for the next call,
        // which is the path that advances the second cursor.
        let second = open_loop_lags(
            &mut ctx,
            MR102,
            &mut weighted,
            &signal,
            origin,
            &mut old_lags,
            &mut voiced,
            None,
        );
        assert_eq!(second[0], 45);

        // Silence: the correlation is flat, so no lag is voiced, and the
        // confidence decays instead of resetting.
        let quiet = vec![Word16(0); origin + L_FRAME];
        let _ = open_loop_lags(
            &mut ctx,
            MR102,
            &mut weighted,
            &quiet,
            origin,
            &mut old_lags,
            &mut voiced,
            None,
        );
        assert!(!voiced[0], "silence is not voiced");

        // Every other rate zeroes both flags on entry; 10.2 must not.
        let mut other = [true; 2];
        let _ = open_loop_lags(
            &mut ctx,
            MR74,
            &mut weighted,
            &signal,
            origin,
            &mut old_lags,
            &mut other,
            None,
        );
        assert_eq!(other, [false; 2]);
    }

    #[test]
    fn the_correlation_weighting_cursors_stay_in_bounds_at_both_extremes() {
        // `we` starts at 123 + PIT_MAX − old_T0_med and steps back once per
        // candidate lag while armed. The state invariant is that old_T0_med is
        // itself a lag, so check both ends of that range rather than trusting
        // it.
        for old in [PIT_MIN, PIT_MAX] {
            let start = 123 + i32::from(PIT_MAX) - i32::from(old);
            let start = usize::try_from(start).expect("the start cursor is non-negative");
            assert!(start < CORR_WEIGHT.len(), "start {start}");
            assert!(
                start >= usize::try_from(PIT_MAX - PIT_MIN).expect("the lag span is positive"),
                "the cursor would step below zero from {start}"
            );
        }
    }

    #[test]
    fn the_closed_loop_integer_scan_takes_the_largest_lag_on_a_tie() {
        // The opposite direction from the open loop. An all-zero excitation
        // makes every normalised correlation zero, so the ascending `>=` walk
        // ends on the window's top lag — and the flat fractional search then
        // rewrites the starting −2 as +1 one sample lower.
        let mut ctx = DspContext::default();
        let mut pitch = ClosedLoopPitch::new();
        let excitation = Excitation::new();
        let xn = [Word16(1000); L_SUBFR];
        let h1 = [Word16(4096); L_SUBFR];

        let found = pitch.search(
            &mut ctx,
            MR74,
            [50, 50],
            excitation.all(),
            EXC_ORIGIN,
            &xn,
            &h1,
            0,
        );
        let window = lag_window(&mut ctx, 50, 3, 6, PIT_MIN);
        assert_eq!(
            (found.lag.integer.0, found.lag.frac.0),
            (window.max.0 - 1, 1),
            "top of the window, then the fractional normalisation"
        );
    }
}
