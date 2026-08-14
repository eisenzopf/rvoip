//! Fixed-point windowing, autocorrelation and lag windowing for AMR-WB.
//!
//! Implements TS 26.190 §5.2.1 in the arithmetic that defines the codec. AMR is
//! specified bit-exactly (§8.1: "described in a bit-exact arithmetic to allow
//! easy type approval"), so this is not "a fixed-point version of the maths" —
//! the exact sequence of saturating operations, shifts and roundings *is* the
//! specification. Every intermediate here is chosen to match TS 26.173.
//!
//! # Double-precision format
//!
//! Autocorrelations do not fit in 16 bits with enough precision for
//! Levinson-Durbin, so the reference carries each as a `(high, low)` pair —
//! "DPF" — where the value is `high · 2¹⁵ + low`, both `Word16`.
//! [`crate::fixed_point::oper32`] provides the split and multiply.
//!
//! # Scaling
//!
//! Two normalisations happen, for different reasons:
//!
//! 1. **Before accumulating**, the windowed signal is right-shifted so that
//!    summing 384 squared terms cannot overflow a `Word32`. The shift is
//!    derived from a cheap energy estimate rather than from the true energy,
//!    because the true energy is what we are about to compute.
//! 2. **After computing `r[0]`**, every lag is left-shifted by `norm_l(r[0])`.
//!    The *same* shift is applied to all lags, so their relative magnitudes —
//!    which is all Levinson-Durbin cares about — are preserved while the
//!    leading bit is pushed up for precision.

use super::tables::{ANALYSIS_WINDOW, LAG_WINDOW_DPF};
use crate::fixed_point::arith::mult_r;
use crate::fixed_point::arith32::{l_add, l_mac, l_mult};
use crate::fixed_point::oper32::{l_extract, mpy_32};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// LP predictor order.
pub const LP_ORDER: usize = 16;

/// Analysis window length in samples: 30 ms at 12.8 kHz.
pub const WINDOW_LEN: usize = 384;

/// An autocorrelation sequence in double-precision format.
///
/// `high[i] · 2¹⁵ + low[i]` is `r(i)`, for lags `0..=LP_ORDER`. Only the ratios
/// between lags carry meaning — the sequence has been scaled by a common
/// power of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Autocorrelation {
    /// Most significant halves.
    pub high: [Word16; LP_ORDER + 1],
    /// Least significant halves.
    pub low: [Word16; LP_ORDER + 1],
}

impl Default for Autocorrelation {
    fn default() -> Self {
        Self {
            high: [Word16(0); LP_ORDER + 1],
            low: [Word16(0); LP_ORDER + 1],
        }
    }
}

/// Window a frame and compute its autocorrelations, TS 26.173 `Autocorr`.
///
/// `speech` is 384 samples of pre-emphasised, down-scaled input at 12.8 kHz.
/// The result is **not** lag-windowed: the caller applies [`lag_window`], as
/// the reference does in two separate calls.
///
/// The two were fused here for a while, which was invisible because the only
/// consumer wanted both — until an encoder test compared the pre-window values
/// and found them equal to the post-window ones. They were equal because the
/// *trace* had the same defect, dumping both under one name. Two independent
/// copies of one mistake agreeing is the oldest failure in this project, and
/// splitting the function is what makes the two values distinguishable at all.
#[must_use]
pub fn autocorrelation(speech: &[Word16; WINDOW_LEN]) -> Autocorrelation {
    let mut ctx = DspContext::default();

    // Apply the analysis window.
    let mut y = [Word16(0); WINDOW_LEN];
    for ((slot, &sample), &w) in y.iter_mut().zip(speech.iter()).zip(ANALYSIS_WINDOW.iter()) {
        *slot = mult_r(&mut ctx, sample, Word16(w));
    }

    // Estimate the energy to pick a safe pre-accumulation shift. The bias of
    // 16 in the high half (sqrt(256)) keeps the estimate away from zero so
    // norm_l has something to work with on near-silent frames.
    let mut estimate = Word32(i32::from(16i16) << 16);
    for &sample in &y {
        let square = l_mult(&mut ctx, sample, sample);
        let term = l_shr(&mut ctx, square, 8);
        estimate = l_add(&mut ctx, estimate, term);
    }

    // shift = 4 - norm/2: halving because energy is a squared quantity, and the
    // constant 4 absorbs the 2^8 the estimate was scaled by.
    let norm = norm_l(estimate);
    let mut shift = shr(&mut ctx, Word16(norm), 1);
    shift = Word16(4i16.saturating_sub(shift.0)).max(Word16(0));

    for slot in &mut y {
        *slot = shr_r(&mut ctx, *slot, shift.0);
    }

    // r[0], then normalise. Starting the accumulator at 1 rather than 0 is what
    // keeps a silent frame from producing r(0) = 0, which would make
    // Levinson-Durbin divide by zero. It is not the -40 dB noise floor; that
    // lives in the lag window.
    let mut sum = Word32(1);
    for &sample in &y {
        sum = l_mac(&mut ctx, sum, sample, sample);
    }
    let norm = norm_l(sum);
    sum = l_shl(&mut ctx, sum, norm);

    let mut result = Autocorrelation::default();
    let (hi, lo) = l_extract(sum);
    result.high[0] = hi;
    result.low[0] = lo;

    // Remaining lags, scaled by the same norm so relative magnitudes hold.
    for lag in 1..=LP_ORDER {
        let mut sum = Word32(0);
        for j in 0..WINDOW_LEN - lag {
            sum = l_mac(&mut ctx, sum, y[j], y[j + lag]);
        }
        let sum = l_shl(&mut ctx, sum, norm);
        let (hi, lo) = l_extract(sum);
        result.high[lag] = hi;
        result.low[lag] = lo;
    }

    result
}

/// Apply the lag window in place, per TS 26.173 `lag_wind.c`.
///
/// Lags 1 through 16 only — **`r[0]` is deliberately untouched**. TS 26.190's
/// prose says `r(0)` is multiplied by the white-noise correction 1.0001; the
/// reference instead folds the reciprocal 0.9999 into this table and leaves
/// `r[0]` alone. The two are equivalent up to a uniform scale in exact
/// arithmetic, but not once values are normalised and rounded, so the
/// reference's placement is the one that produces conformant output.
pub fn lag_window(r: &mut Autocorrelation) {
    for lag in 1..=LP_ORDER {
        let (w_hi, w_lo) = (
            Word16(LAG_WINDOW_DPF[(lag - 1) * 2]),
            Word16(LAG_WINDOW_DPF[(lag - 1) * 2 + 1]),
        );
        let scaled = mpy_32(r.high[lag], r.low[lag], w_hi, w_lo);
        let (hi, lo) = l_extract(scaled);
        r.high[lag] = hi;
        r.low[lag] = lo;
    }
}

#[cfg(test)]
// The test signal generator works in f64 purely to synthesise input; the code
// under test is entirely fixed point. Float lints on that scaffolding are noise.
#[allow(
    clippy::suboptimal_flops,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;
    use crate::fixed_point::oper32::l_comp;

    /// The DPF pair as a plain i64, for comparisons that only care about value.
    fn value(r: &Autocorrelation, lag: usize) -> i64 {
        i64::from(l_comp(r.high[lag], r.low[lag]).0)
    }

    fn speechlike(seed: u64) -> [Word16; WINDOW_LEN] {
        let mut s = [Word16(0); WINDOW_LEN];
        let f1 = 300.0 + (seed % 7) as f64 * 40.0;
        let f2 = 1100.0 + (seed % 5) as f64 * 90.0;
        for (n, slot) in s.iter_mut().enumerate() {
            let t = n as f64 / 12800.0;
            let env = 0.5 + 0.5 * (2.0 * std::f64::consts::PI * 3.0 * t).sin();
            let v = env
                * (0.6 * (2.0 * std::f64::consts::PI * f1 * t).sin()
                    + 0.3 * (2.0 * std::f64::consts::PI * f2 * t).sin())
                * 12000.0;
            *slot = Word16(v.clamp(-32768.0, 32767.0) as i16);
        }
        s
    }

    // ---- bit-exactness against TS 26.173 ----

    /// Per-stage dumps from the normative reference: input samples and the
    /// exact autocorrelations its `Autocorr` + `Lag_window` produce.
    ///
    /// Generated by `tools/build-amr-reference.sh`, which fetches TS 26.173 and
    /// drives its functions directly. This is the only kind of evidence that
    /// counts for a bit-exact codec — property tests can show the output is
    /// *plausible*, but conformance means matching these integers exactly.
    const LP_STAGES: &str = include_str!("../../testdata/lp_stages_wb.txt");

    /// Pull one labelled row out of a case block.
    fn row(case: usize, label: &str) -> Vec<i16> {
        let marker = format!("case {case}\n");
        let block = LP_STAGES
            .split(&marker)
            .nth(1)
            .unwrap_or_else(|| panic!("case {case} missing from fixture"));
        let block = block.split("\ncase ").next().unwrap_or(block);
        for line in block.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("case {case} has no row {label:?}");
    }

    fn case_count() -> usize {
        LP_STAGES.matches("case ").count()
    }

    #[test]
    fn autocorrelation_is_bit_exact_against_ts26173() {
        assert!(case_count() >= 4, "fixture should carry several cases");

        for case in 0..case_count() {
            let samples = row(case, "x");
            assert_eq!(samples.len(), WINDOW_LEN, "case {case}: input length");
            let mut speech = [Word16(0); WINDOW_LEN];
            for (slot, &v) in speech.iter_mut().zip(samples.iter()) {
                *slot = Word16(v);
            }

            let mut got = autocorrelation(&speech);
            lag_window(&mut got);
            let want_h = row(case, "r_h");
            let want_l = row(case, "r_l");

            for lag in 0..=LP_ORDER {
                assert_eq!(
                    got.high[lag].0, want_h[lag],
                    "case {case}: r_h[{lag}] differs from the reference"
                );
                assert_eq!(
                    got.low[lag].0, want_l[lag],
                    "case {case}: r_l[{lag}] differs from the reference"
                );
            }
        }
    }

    #[test]
    fn lag_windowing_is_bit_exact_against_ts26173() {
        // Isolates the lag window from the autocorrelation: feed the
        // reference's own pre-lag values in and require its post-lag values
        // out, so a failure here cannot be blamed on the accumulation.
        for case in 0..case_count() {
            let pre_h = row(case, "r_h_prelag");
            let pre_l = row(case, "r_l_prelag");
            let want_h = row(case, "r_h");
            let want_l = row(case, "r_l");

            let mut r = Autocorrelation::default();
            for lag in 0..=LP_ORDER {
                r.high[lag] = Word16(pre_h[lag]);
                r.low[lag] = Word16(pre_l[lag]);
            }
            lag_window(&mut r);

            for lag in 0..=LP_ORDER {
                assert_eq!(r.high[lag].0, want_h[lag], "case {case}: lag {lag} high");
                assert_eq!(r.low[lag].0, want_l[lag], "case {case}: lag {lag} low");
            }
        }
    }

    #[test]
    fn window_table_matches_the_spec_formula_to_within_one_lsb() {
        // Documents the §5.2.1 definition and catches transcription errors,
        // without claiming the formula is the source of truth — seven entries
        // genuinely differ by 1 LSB, which is why the table is carried.
        let (l1, l2) = (256usize, 128usize);
        let mut worst = 0i32;
        for (n, &entry) in ANALYSIS_WINDOW.iter().enumerate() {
            let w = if n < l1 {
                0.54 - 0.46
                    * (2.0 * std::f64::consts::PI * n as f64 / (2.0 * l1 as f64 - 1.0)).cos()
            } else {
                (2.0 * std::f64::consts::PI * (n - l1) as f64 / (4.0 * l2 as f64 - 1.0)).cos()
            };
            let expected = (w * 32767.0).round() as i32;
            worst = worst.max((i32::from(entry) - expected).abs());
        }
        assert!(
            worst <= 1,
            "window table diverges from the formula by {worst}"
        );
    }

    #[test]
    fn window_has_the_shape_the_spec_describes() {
        assert_eq!(ANALYSIS_WINDOW.len(), WINDOW_LEN);
        // Weight concentrated late, which is the point of the asymmetry.
        let first: i64 = ANALYSIS_WINDOW[..WINDOW_LEN / 2]
            .iter()
            .map(|&v| i64::from(v) * i64::from(v))
            .sum();
        let second: i64 = ANALYSIS_WINDOW[WINDOW_LEN / 2..]
            .iter()
            .map(|&v| i64::from(v) * i64::from(v))
            .sum();
        assert!(second > first, "{first} vs {second}");
        // Peaks at the join between the two parts.
        assert_eq!(ANALYSIS_WINDOW[255], 32767);
        assert!(ANALYSIS_WINDOW.iter().all(|&v| v > 0));
    }

    #[test]
    fn lag_table_covers_lags_one_through_sixteen() {
        assert_eq!(LAG_WINDOW_DPF.len(), LP_ORDER * 2, "16 (high, low) pairs");
        // Reconstructed values must descend from just under 1.0, and the first
        // must match the value the reference documents in its own header.
        let as_f64 = |lag: usize| {
            let hi = f64::from(LAG_WINDOW_DPF[(lag - 1) * 2]);
            let lo = f64::from(LAG_WINDOW_DPF[(lag - 1) * 2 + 1]);
            (hi * 32768.0 + lo) / 1_073_741_824.0
        };
        assert!(
            (as_f64(1) - 0.999_466_42).abs() < 1e-6,
            "lag 1 = {}",
            as_f64(1)
        );
        for lag in 2..=LP_ORDER {
            assert!(as_f64(lag) < as_f64(lag - 1), "lag {lag} did not decrease");
        }
    }

    #[test]
    fn r0_dominates_and_every_lag_is_bounded_by_it() {
        // The defining property of an autocorrelation sequence. If the shared
        // normalisation shift were applied inconsistently across lags this
        // would fail.
        for seed in 0..6u64 {
            let mut r = autocorrelation(&speechlike(seed));
            lag_window(&mut r);
            let r0 = value(&r, 0);
            assert!(r0 > 0, "seed {seed}: r(0) = {r0}");
            for lag in 1..=LP_ORDER {
                assert!(
                    value(&r, lag).abs() <= r0,
                    "seed {seed}: |r({lag})| = {} exceeds r(0) = {r0}",
                    value(&r, lag).abs()
                );
            }
        }
    }

    #[test]
    fn r0_is_normalised_into_the_top_bits() {
        // After the shared shift, r(0) should sit high in the Word32 range;
        // that is the whole purpose of the norm_l step, and it is what gives
        // Levinson-Durbin its precision.
        for seed in 0..6u64 {
            let mut r = autocorrelation(&speechlike(seed));
            lag_window(&mut r);
            let r0 = value(&r, 0);
            assert!(r0 >= 1 << 29, "seed {seed}: r(0) = {r0} not normalised");
        }
    }

    #[test]
    fn silence_still_produces_a_usable_r0() {
        // The accumulator starts at 1 rather than 0 precisely so a silent frame
        // cannot yield r(0) = 0 and make Levinson-Durbin divide by zero.
        let mut r = autocorrelation(&[Word16(0); WINDOW_LEN]);
        lag_window(&mut r);
        assert!(
            value(&r, 0) > 0,
            "silent frame gave r(0) = {}",
            value(&r, 0)
        );
    }

    #[test]
    fn lag_window_leaves_r0_alone_and_shrinks_the_rest() {
        // Pins the divergence from TS 26.190's prose. r[0] must come through
        // untouched; every other lag must be scaled down slightly.
        let mut r = Autocorrelation::default();
        for lag in 0..=LP_ORDER {
            let (hi, lo) = l_extract(Word32(1 << 30));
            r.high[lag] = hi;
            r.low[lag] = lo;
        }
        let before = value(&r, 0);
        lag_window(&mut r);

        assert_eq!(value(&r, 0), before, "r(0) must not be scaled");
        for lag in 1..=LP_ORDER {
            let after = value(&r, lag);
            assert!(
                after < before,
                "lag {lag} should shrink: {after} vs {before}"
            );
            // A 60 Hz expansion is gentle; nothing should collapse.
            assert!(after > before / 2, "lag {lag} shrank too far: {after}");
        }
    }

    #[test]
    fn louder_input_does_not_change_the_normalised_result_much() {
        // The pre-accumulation shift adapts to signal level, so the normalised
        // autocorrelation should be nearly scale-invariant. This is what lets
        // one Levinson-Durbin implementation serve every input level.
        let quiet = speechlike(3);
        let mut loud = quiet;
        for slot in &mut loud {
            slot.0 = slot.0.saturating_mul(2);
        }
        let mut rq = autocorrelation(&quiet);
        let mut rl = autocorrelation(&loud);
        lag_window(&mut rq);
        lag_window(&mut rl);

        for lag in 1..=LP_ORDER {
            let a = value(&rq, lag) as f64 / value(&rq, 0) as f64;
            let b = value(&rl, lag) as f64 / value(&rl, 0) as f64;
            assert!(
                (a - b).abs() < 0.02,
                "lag {lag}: normalised {a:.4} vs {b:.4} at double amplitude"
            );
        }
    }
}
