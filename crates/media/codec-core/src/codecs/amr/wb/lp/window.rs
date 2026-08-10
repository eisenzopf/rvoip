//! AMR-WB LP analysis window, autocorrelation and lag windowing.
//!
//! Implements 3GPP TS 26.190 §5.2.1. Quoting the normative text, because every
//! constant here comes from it:
//!
//! > LP analysis is performed once per frame using an asymmetric window. The
//! > window has its weight concentrated at the fourth subframe and it consists
//! > of two parts: the first part is a half of a Hamming window and the second
//! > part is a quarter of a Hamming-cosine function cycle. […] where the values
//! > L1=256 and L2=128 are used.
//!
//! > a 60 Hz bandwidth expansion is used by lag windowing the autocorrelations
//! > […] where f0=60 Hz is the bandwidth expansion and fs=12800 Hz is the
//! > sampling frequency. Further, r(0) is multiplied by the white noise
//! > correction factor 1.0001 which is equivalent to adding a noise floor at
//! > -40 dB.
//!
//! **The last sentence is not what the normative reference does.** TS 26.173
//! `lag_wind.c` applies the lag window to `r[1..=16]` only and leaves `r[0]`
//! alone, folding the reciprocal `0.9999` into the table instead. See
//! [`WHITE_NOISE_CORRECTION`]. The prose describes the intent; the fixed-point
//! C defines the codec.
//!
//! # Numeric style
//!
//! Indices are small and exactly representable, so the `usize`-to-`f64`
//! conversions here are lossless; and the window definitions are written to
//! mirror the spec's equations rather than to be fused, so they stay readable
//! against the text above.
//!
//! # Why the window is asymmetric
//!
//! Concentrating the weight on the fourth (most recent) subframe means the LP
//! filter describes the end of the frame, which is where the coefficients are
//! actually used before the next analysis arrives. A symmetric window would
//! describe the middle of a frame that has already been coded.

#![allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]

/// LP predictor order for AMR-WB. Sixteen, against G.729's ten — the wideband
/// spectrum needs more poles to describe.
pub const LP_ORDER: usize = 16;


/// First window part: half a Hamming window.
pub const L1: usize = 256;

/// Second window part: a quarter cycle of a Hamming-cosine.
pub const L2: usize = 128;

/// Analysis window length, 30 ms at 12.8 kHz. The 384 samples span the 256-sample
/// frame plus 128 of look-back, which is where the 5 ms overhead comes from.
pub const WINDOW_LEN: usize = L1 + L2;

/// Internal sampling rate, in hertz.
pub const INTERNAL_RATE: f64 = 12_800.0;

/// Lag-window bandwidth expansion, in hertz.
pub const LAG_WINDOW_F0: f64 = 60.0;

/// White noise correction: a -40 dB noise floor keeping Levinson-Durbin stable
/// on near-silent input.
///
/// **Applied as `0.9999` on `r[1..=16]`, not as `1.0001` on `r(0)`.**
///
/// TS 26.190's prose says "r(0) is multiplied by the white noise correction
/// factor 1.0001". The normative fixed-point reference, TS 26.173
/// `lag_wind.c`, does something else: its loop runs `i = 1..M` and never
/// touches `r[0]`, with the reciprocal folded into the lag-window table —
/// whose own header states "noise floor = 1.0001 = (0.9999 on r[1]..r[16])".
///
/// The two are equivalent up to a uniform scale on the autocorrelation
/// sequence, and Levinson-Durbin is scale-invariant, so in exact arithmetic
/// the predictor is identical. **In fixed point they are not interchangeable**:
/// the sequence is normalised and rounded at every step, so a different scale
/// changes intermediate values. Bit-exactness is defined against the reference
/// C, so the reference's placement is the correct one and this module follows
/// it.
pub const WHITE_NOISE_CORRECTION: f64 = 0.9999;

/// The analysis window, computed from the TS 26.190 §5.2.1 definition.
///
/// Part one, for `n` in `0..L1`, is half a Hamming window:
/// `0.54 - 0.46 cos(2πn / (2·L1 - 1))`
///
/// Part two, for `n` in `L1..L1+L2`, is a quarter Hamming-cosine cycle:
/// `cos(2π(n - L1) / (4·L2 - 1))`
///
/// Computed rather than tabulated so the definition stays visible and cannot
/// drift from the spec through a transcription error.
#[must_use]
pub fn analysis_window() -> [f64; WINDOW_LEN] {
    let mut w = [0.0f64; WINDOW_LEN];
    for (n, slot) in w.iter_mut().enumerate().take(L1) {
        let num = 2.0 * std::f64::consts::PI * n as f64;
        *slot = 0.54 - 0.46 * (num / (2.0 * L1 as f64 - 1.0)).cos();
    }
    for (offset, slot) in w[L1..].iter_mut().enumerate() {
        let num = 2.0 * std::f64::consts::PI * offset as f64;
        *slot = (num / (4.0 * L2 as f64 - 1.0)).cos();
    }
    w
}

/// The lag window applied to the autocorrelations, `w(i)` for `i` in
/// `0..=LP_ORDER`.
///
/// `exp(-0.5 (2π f0 i / fs)²)` — a Gaussian in the lag domain, equivalent to
/// convolving the spectrum with a 60 Hz-wide kernel. It broadens sharp spectral
/// peaks slightly, which keeps the resulting synthesis filter from ringing.
#[must_use]
pub fn lag_window() -> [f64; LP_ORDER + 1] {
    let mut w = [0.0f64; LP_ORDER + 1];
    for (i, slot) in w.iter_mut().enumerate() {
        let x = 2.0 * std::f64::consts::PI * LAG_WINDOW_F0 * i as f64 / INTERNAL_RATE;
        *slot = (-0.5 * x * x).exp();
    }
    w
}

/// Window a frame of pre-emphasised speech and compute its autocorrelations,
/// with lag windowing and the white-noise correction applied.
///
/// Returns `r[0..=LP_ORDER]`, ready for Levinson-Durbin.
///
/// This is the floating-point form. The shipped codec needs the fixed-point
/// formulation, which normalises and tracks Q-formats; this exists first so the
/// fixed-point version has something unambiguous to be checked against, and so
/// the spec's definition is expressed once in a readable form.
#[must_use]
pub fn autocorrelation(speech: &[f64; WINDOW_LEN]) -> [f64; LP_ORDER + 1] {
    let window = analysis_window();
    let mut windowed = [0.0f64; WINDOW_LEN];
    for (slot, (s, w)) in windowed.iter_mut().zip(speech.iter().zip(window.iter())) {
        *slot = s * w;
    }

    let mut r = [0.0f64; LP_ORDER + 1];
    for (k, slot) in r.iter_mut().enumerate() {
        let mut acc = 0.0;
        for n in k..WINDOW_LEN {
            acc += windowed[n] * windowed[n - k];
        }
        *slot = acc;
    }

    // Matches TS 26.173 `lag_wind.c`: r[0] is left alone, and the noise-floor
    // reciprocal rides along with the lag window on r[1..=16].
    let lag = lag_window();
    for (slot, w) in r.iter_mut().zip(lag.iter()).skip(1) {
        *slot *= w * WHITE_NOISE_CORRECTION;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_has_the_lengths_the_spec_gives() {
        assert_eq!(L1, 256);
        assert_eq!(L2, 128);
        assert_eq!(WINDOW_LEN, 384, "30 ms at 12.8 kHz");
        // 384 samples spans the 256-sample frame plus 128 of look-back; the
        // frame itself is 20 ms, so the extra 10 ms is the stated overhead.
        assert!((WINDOW_LEN as f64 / INTERNAL_RATE - 0.030).abs() < 1e-12);
    }

    #[test]
    fn window_is_asymmetric_with_its_weight_late() {
        let w = analysis_window();
        // The whole point of the asymmetric window: the second half carries
        // more energy than the first, so the filter describes the end of the
        // frame where it will actually be applied.
        let first: f64 = w[..WINDOW_LEN / 2].iter().map(|x| x * x).sum();
        let second: f64 = w[WINDOW_LEN / 2..].iter().map(|x| x * x).sum();
        assert!(second > first, "weight should sit late: {first} vs {second}");
    }

    #[test]
    fn window_parts_join_at_their_peak() {
        let w = analysis_window();
        // Part one is half a Hamming, rising to 1.0 at its end; part two is a
        // cosine quarter-cycle starting at 1.0. They must meet there, or the
        // window has a step in it.
        assert!((w[L1 - 1] - 1.0).abs() < 2e-4, "end of part one: {}", w[L1 - 1]);
        assert!((w[L1] - 1.0).abs() < 1e-12, "start of part two: {}", w[L1]);
        assert!(w[0] > 0.07 && w[0] < 0.09, "Hamming starts near 0.08: {}", w[0]);
        // And it decays towards the end rather than cutting off abruptly.
        assert!(w[WINDOW_LEN - 1] < 0.02, "tail: {}", w[WINDOW_LEN - 1]);
    }

    #[test]
    fn window_is_positive_and_bounded() {
        for (i, &v) in analysis_window().iter().enumerate() {
            assert!(v > 0.0 && v <= 1.0, "w[{i}] = {v}");
        }
    }

    #[test]
    fn lag_window_matches_a_60hz_expansion() {
        let w = lag_window();
        assert_eq!(w.len(), LP_ORDER + 1);
        assert!((w[0] - 1.0).abs() < 1e-15, "w(0) must be exactly 1");
        // Monotonically decreasing, and still close to 1 at the last lag: a
        // 60 Hz expansion is a gentle correction, not a heavy taper.
        for i in 1..w.len() {
            assert!(w[i] < w[i - 1], "lag window must decrease at {i}");
        }
        // exp(-0.5 (2π·60·16/12800)²) ≈ 0.8949 at the highest lag.
        assert!(
            (w[LP_ORDER] - 0.894_911).abs() < 1e-5,
            "w(16) = {}",
            w[LP_ORDER]
        );
    }

    #[test]
    fn lag_window_matches_the_values_the_reference_documents() {
        // vo-amrwbenc's lag_wind.tab states its first values in a comment:
        //   lag_wind[0] = 1.00000000   lag_wind[1] = 0.99946642
        //   lag_wind[2] = 0.99816680   lag_wind[3] = 0.99600452
        //   lag_wind[4] = 0.99298513
        //
        // Those are NOT what this function returns, and the difference is
        // informative rather than a defect. The same file notes
        // "noise floor = 1.0001 = (0.9999 on r[1]..r[16])": the reference folds
        // the white-noise correction into its lag window instead of applying it
        // to r(0). TS 26.190 specifies the correction on r(0), which is what
        // this module does, so our lag window is the reference's divided by
        // 0.9999.
        //
        // The two differ only by an overall scale on the autocorrelation
        // sequence, which Levinson-Durbin is invariant to — the resulting
        // predictor is identical either way. Checking against the documented
        // values with the factor restored confirms the formula itself.
        let w = lag_window();

        // Lag 0 is unscaled in both: the reference stores 1.0 there and notes
        // the 0.9999 applies to r[1]..r[16] only.
        assert!((w[0] - 1.0).abs() < 1e-15);

        let documented = [0.999_466_42, 0.998_166_80, 0.996_004_52, 0.992_985_13];
        for (offset, &want) in documented.iter().enumerate() {
            let lag = offset + 1;
            let folded = w[lag] * 0.9999;
            assert!(
                (folded - want).abs() < 1e-6,
                "lag {lag}: ours×0.9999 = {folded}, reference documents {want}"
            );
        }
    }

    #[test]
    fn autocorrelation_of_a_sinusoid_peaks_at_zero_lag() {
        let mut s = [0.0f64; WINDOW_LEN];
        for (n, slot) in s.iter_mut().enumerate() {
            *slot = (2.0 * std::f64::consts::PI * 300.0 * n as f64 / INTERNAL_RATE).sin() * 8000.0;
        }
        let r = autocorrelation(&s);
        assert!(r[0] > 0.0);
        for k in 1..=LP_ORDER {
            assert!(r[k].abs() <= r[0], "|r({k})| exceeded r(0)");
        }
    }

    #[test]
    fn the_noise_floor_rides_on_r1_upwards_not_on_r0() {
        // Pins the divergence between the spec's prose and the normative
        // reference. TS 26.190 says r(0) is multiplied by 1.0001; TS 26.173
        // `lag_wind.c` instead leaves r[0] alone and folds 0.9999 into the lag
        // window for r[1..=16]. Equivalent up to scale in exact arithmetic,
        // *not* interchangeable once the sequence is normalised and rounded in
        // fixed point — so the reference's placement is the one that counts.
        let mut s = [0.0f64; WINDOW_LEN];
        for (n, slot) in s.iter_mut().enumerate() {
            *slot = ((n * 41 % 197) as f64) - 98.0;
        }
        let w = analysis_window();
        let lag = lag_window();
        let r = autocorrelation(&s);

        // r(0) is the plain windowed energy, with no correction applied.
        let raw_r0: f64 = (0..WINDOW_LEN).map(|n| (s[n] * w[n]).powi(2)).sum();
        assert!(
            (r[0] / raw_r0 - 1.0).abs() < 1e-12,
            "r(0) must be unscaled, got ratio {}",
            r[0] / raw_r0
        );

        // r(1) carries both the lag window and the noise-floor reciprocal.
        let raw_r1: f64 = (1..WINDOW_LEN).map(|n| s[n] * w[n] * s[n - 1] * w[n - 1]).sum();
        let expected = raw_r1 * lag[1] * WHITE_NOISE_CORRECTION;
        assert!(
            (r[1] - expected).abs() <= expected.abs() * 1e-9,
            "r(1): {} vs {expected}",
            r[1]
        );
    }

    #[test]
    fn exact_silence_has_no_autocorrelation() {
        // An all-zero frame gives r(0) = 0 and no predictor; the noise floor is
        // a conditioning aid for near-silence, not a fix for true silence.
        let r = autocorrelation(&[0.0f64; WINDOW_LEN]);
        assert!(r[0].abs() < f64::EPSILON);
    }

    #[test]
    fn autocorrelation_is_the_textbook_definition() {
        // Guards the loop bounds, which are the easiest thing to get wrong.
        let mut s = [0.0f64; WINDOW_LEN];
        for (n, slot) in s.iter_mut().enumerate() {
            *slot = ((n * 37 % 211) as f64) - 105.0;
        }
        let w = analysis_window();
        let lag = lag_window();
        let got = autocorrelation(&s);

        for k in 0..=LP_ORDER {
            let mut want = 0.0;
            for n in k..WINDOW_LEN {
                want += s[n] * w[n] * s[n - k] * w[n - k];
            }
            // r[0] is unscaled; r[1..] carry both the lag window and the
            // noise-floor reciprocal, as TS 26.173 `lag_wind.c` does.
            if k > 0 {
                want *= lag[k] * WHITE_NOISE_CORRECTION;
            }
            let tol = want.abs() * 1e-9 + 1e-6;
            assert!((got[k] - want).abs() <= tol, "r({k}): {} vs {want}", got[k]);
        }
    }
}
