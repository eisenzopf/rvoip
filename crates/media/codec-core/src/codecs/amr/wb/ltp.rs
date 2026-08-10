//! Long-term prediction, 3GPP TS 26.190 §6.1 and §6.4, in fixed point.
//!
//! The adaptive codebook, and the three filters that shape a subframe's
//! excitation before the two contributions are summed.
//!
//! # The adaptive codebook is the excitation's own past
//!
//! There is no stored codebook here at all: the "codebook vector" is a copy of
//! the excitation from one pitch period ago. That is what makes CELP work on
//! voiced speech, where each period looks much like the last — the encoder
//! spends bits on the *difference* rather than the waveform.
//!
//! Because the decoder reads back what it previously wrote, an error in this
//! module does not stay local. It feeds the history, which feeds the next
//! subframe, and the divergence compounds.
//!
//! # Fractional lags
//!
//! Pitch periods are not whole numbers of samples, and rounding one to the
//! nearest sample is audible as roughness on sustained vowels. The lag is
//! coded to a quarter sample and realised by interpolating with a 4×
//! upsampling FIR — [`INTER4_2`], read with a stride of four at an offset the
//! fraction selects.

use super::codebook::L_SUBFR;
use crate::fixed_point::arith::round;
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu, l_mult};
use crate::fixed_point::shift::l_shl;
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Upsampling factor of the interpolation filter.
const UP_SAMP: usize = 4;

/// Half-length of the interpolation filter, in whole samples.
const L_INTERPOL2: usize = 16;

/// Longest pitch lag.
pub const PIT_MAX: usize = 231;

/// Interpolation filter span, in samples.
pub const L_INTERPOL: usize = 17;

/// How much excitation history the adaptive codebook needs behind it.
pub const HISTORY: usize = PIT_MAX + L_INTERPOL;

/// Pitch sharpening factor, 0.85 in Q15.
pub const PIT_SHARP: Word16 = Word16(27853);

/// Quarter-sample interpolation filter, Q14, −3 dB at `0.856·fs/2`.
///
/// Stored interleaved by phase: entry `i·4 + p` is tap `i` of phase `p`, which
/// is why the read below strides by [`UP_SAMP`].
const INTER4_2: [i16; UP_SAMP * 2 * L_INTERPOL2] = [
    0, 1, 2, 1, -2, -7, -10, -7, 4, 19, 28, 22, -2, -33, -55, -49, -10, 47, 91, 92, 38, -52,
    -133, -153, -88, 43, 175, 231, 165, -9, -209, -325, -275, -60, 226, 431, 424, 175, -213,
    -544, -619, -355, 153, 656, 871, 626, -16, -762, -1207, -1044, -249, 853, 1699, 1749, 780,
    -923, -2598, -3267, -2147, 968, 5531, 10359, 14031, 15401, 14031, 10359, 5531, 968, -2147,
    -3267, -2598, -923, 780, 1749, 1699, 853, -249, -1044, -1207, -762, -16, 626, 871, 656,
    153, -355, -619, -544, -213, 175, 424, 431, 226, -60, -275, -325, -209, -9, 165, 231, 175,
    43, -88, -153, -133, -52, 38, 92, 91, 47, -10, -49, -55, -33, -2, 22, 28, 19, 4, -7, -10,
    -7, -2, 1, 2, 1, 0, 0,
];

/// Low-pass filter coefficients for the optional LTP smoothing, Q15.
const LTP_LP: (Word16, Word16) = (Word16(5898), Word16(20972));

/// Build the adaptive codebook vector for one subframe, in place.
///
/// `buffer[..offset]` is the excitation history, most recent last, and must be
/// at least [`HISTORY`] long. `buffer[offset..offset + len]` is overwritten
/// with the predicted vector.
///
/// # Why this writes in place
///
/// It has to. When the pitch lag is shorter than a subframe — which is common,
/// since lags start at 34 against a 64-sample subframe — the filter reads
/// samples it wrote **earlier in this same subframe**. That self-reference is
/// not incidental: it is how a short lag produces a waveform that repeats
/// within the subframe. Reading from a separate immutable history and writing
/// elsewhere would silently produce a different, wrong answer for every lag
/// below 64.
///
/// # Panics
///
/// If `buffer` is too short for the lag's reach or the requested length.
pub fn predict(buffer: &mut [Word16], offset: usize, lag: usize, frac: u8, len: usize) {
    let mut ctx = DspContext::default();

    // Negating the fraction turns "delay by T0 + frac/4" into a filter phase.
    // A non-zero fraction borrows a whole sample from the integer lag, which
    // is what `UP_SAMP - frac` expresses without going through a negative.
    let mut start = offset - lag;
    let phase = if frac == 0 {
        0
    } else {
        start -= 1;
        UP_SAMP - usize::from(frac)
    };
    // Step back to the first tap the filter needs.
    start -= L_INTERPOL2 - 1;

    for j in 0..len {
        let mut sum = Word32(0);
        let mut k = UP_SAMP - 1 - phase;
        for i in 0..2 * L_INTERPOL2 {
            sum = l_mac(&mut ctx, sum, buffer[start + j + i], Word16(INTER4_2[k]));
            k += UP_SAMP;
        }
        // The filter is Q14 and the accumulator Q30, so one shift gives Q31.
        let scaled = l_shl(&mut ctx, sum, 1);
        buffer[offset + j] = round(&mut ctx, scaled);
    }
}

/// Smooth the adaptive codebook vector with a three-tap low-pass.
///
/// Selectable per subframe above 8.85 kbit/s. It trades pitch-pulse sharpness
/// for less high-frequency noise in the predicted component, and the encoder
/// picks whichever fits the frame — which is why the choice costs a bit.
///
/// `input` must hold one sample before and one after the subframe: index 0 is
/// `exc[-1]`, so the subframe proper starts at index 1.
///
/// # Panics
///
/// If `input` is shorter than `L_SUBFR + 2`.
#[must_use]
pub fn low_pass(input: &[Word16]) -> [Word16; L_SUBFR] {
    assert!(
        input.len() >= L_SUBFR + 2,
        "the low-pass filter needs one sample either side of the subframe"
    );
    let mut ctx = DspContext::default();
    let mut out = [Word16(0); L_SUBFR];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut acc = l_mult(&mut ctx, LTP_LP.0, input[i]);
        acc = l_mac(&mut ctx, acc, LTP_LP.1, input[i + 1]);
        acc = l_mac(&mut ctx, acc, LTP_LP.0, input[i + 2]);
        *slot = round(&mut ctx, acc);
    }
    out
}

/// Apply the spectral tilt of the previous subframe's excitation, in place.
///
/// A first-order preemphasis whose coefficient tracks how voiced the signal
/// is. Voiced speech has a falling spectrum, so tilting the flat pulse vector
/// to match stops the innovation sounding brighter than what it is added to.
pub fn preemphasis(x: &mut [Word16], mu: Word16, memory: &mut Word16) {
    let mut ctx = DspContext::default();
    let last = x[x.len() - 1];

    // Backwards, so each step still sees the unfiltered previous sample.
    for i in (1..x.len()).rev() {
        let acc = l_msu(&mut ctx, l_deposit_h(x[i]), x[i - 1], mu);
        x[i] = round(&mut ctx, acc);
    }
    let acc = l_msu(&mut ctx, l_deposit_h(x[0]), *memory, mu);
    x[0] = round(&mut ctx, acc);

    *memory = last;
}

/// Sharpen the innovation at the pitch period, in place.
///
/// Adds a scaled copy of the vector delayed by one pitch period, which is a
/// comb filter tuned to the talker's pitch. It makes the innovation
/// periodic-looking, so voiced speech is not left sounding buzzy where the
/// adaptive contribution alone cannot carry it.
///
/// Only samples at or beyond `lag` are touched; the start of the subframe has
/// no predecessor a period back.
pub fn sharpen(x: &mut [Word16], lag: usize, sharp: Word16) {
    let mut ctx = DspContext::default();
    for i in lag..x.len() {
        let acc = l_mac(&mut ctx, l_deposit_h(x[i]), x[i - lag], sharp);
        x[i] = round(&mut ctx, acc);
    }
}

/// The pitch lag used for sharpening, which rounds the fraction to the nearest
/// whole sample rather than truncating.
#[must_use]
pub const fn sharpening_lag(lag: usize, frac: u8) -> usize {
    if frac > 2 {
        lag + 1
    } else {
        lag
    }
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_row, has_block};
    use super::*;

    /// The same deterministic excitation history the oracle builds.
    fn history() -> Vec<Word16> {
        (0..HISTORY)
            .map(|n| {
                let phase = n % 57;
                #[allow(clippy::cast_precision_loss)]
                let phase = phase as f64;
                let base = if phase < 3.0 {
                    2000.0f64.mul_add(-phase, 8000.0)
                } else {
                    12.0f64.mul_add(phase, -300.0)
                };
                #[allow(clippy::cast_precision_loss)]
                let drift = 0.4f64.mul_add(
                    (2.0 * std::f64::consts::PI * n as f64 / 313.0).sin(),
                    0.6,
                );
                #[allow(clippy::cast_possible_truncation)]
                Word16((base * drift) as i16)
            })
            .collect()
    }

    /// The oracle's innovation stand-in, before filtering.
    fn innovation(lag_index: usize, frac_index: usize) -> [Word16; L_SUBFR] {
        let mut code = [Word16(0); L_SUBFR];
        for n in 0..8 {
            let i = (n * 7 + lag_index * 3 + frac_index) % L_SUBFR;
            code[i] = Word16(code[i].0 + if n % 2 == 1 { -512 } else { 512 });
        }
        code
    }

    const LAGS: [usize; 6] = [34, 60, 64, 100, 128, 231];

    #[test]
    fn the_adaptive_codebook_is_bit_exact_against_ts26173() {
        assert!(has_block("ltp"), "fixture block ltp missing");
        let history = history();
        let mut checked = 0;

        for (li, &lag) in LAGS.iter().enumerate() {
            for frac in 0..4u8 {
                let c = li * 4 + usize::from(frac);
                let meta = block_row("ltp", &format!("meta{c}"));
                assert_eq!(
                    (i64::from(meta[0]), i64::from(meta[1])),
                    (i64::try_from(lag).expect("lag"), i64::from(frac)),
                    "case {c}: fixture is for a different lag"
                );

                // 65, not 64: the low-pass filter reads one sample ahead.
                let mut buffer = history.clone();
                buffer.resize(HISTORY + L_SUBFR + 1, Word16(0));
                predict(&mut buffer, HISTORY, lag, frac, L_SUBFR + 1);
                let out = &buffer[HISTORY..];

                let want = block_row("ltp", &format!("pred{c}"));
                assert_eq!(want.len(), L_SUBFR + 1, "case {c}: length");
                for (i, (&g, &w)) in out.iter().zip(want.iter()).enumerate() {
                    assert_eq!(
                        g.0, w,
                        "case {c} (lag {lag}, frac {frac}): sample {i} = {} but the reference gives {w}",
                        g.0
                    );
                }
                checked += 1;
            }
        }
        assert_eq!(checked, 24, "expected every lag and fraction combination");
    }

    #[test]
    fn the_low_pass_filter_is_bit_exact_against_ts26173() {
        let history = history();

        for (li, &lag) in LAGS.iter().enumerate() {
            for frac in 0..4u8 {
                let c = li * 4 + usize::from(frac);

                // The filter reads exc[-1], which is the previous subframe's
                // last sample -- part of the history, not of the prediction.
                let mut buffer = history.clone();
                buffer.resize(HISTORY + L_SUBFR + 1, Word16(0));
                predict(&mut buffer, HISTORY, lag, frac, L_SUBFR + 1);

                let input = &buffer[HISTORY - 1..];

                let got = low_pass(input);
                let want = block_row("ltp", &format!("ltpf{c}"));
                assert_eq!(want.len(), L_SUBFR, "case {c}: length");
                for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                    assert_eq!(g.0, w, "case {c}: filtered sample {i}");
                }
            }
        }
    }

    #[test]
    fn preemphasis_and_sharpening_are_bit_exact_against_ts26173() {
        for (li, &lag) in LAGS.iter().enumerate() {
            for frac in 0..4u8 {
                let c = li * 4 + usize::from(frac);
                let mut code = innovation(li, usize::from(frac));

                let mut memory = Word16(0);
                preemphasis(&mut code, Word16(6554), &mut memory);
                sharpen(&mut code, sharpening_lag(lag, frac), PIT_SHARP);

                let want = block_row("ltp", &format!("shrp{c}"));
                assert_eq!(want.len(), L_SUBFR, "case {c}: length");
                for (i, (&g, &w)) in code.iter().zip(want.iter()).enumerate() {
                    assert_eq!(g.0, w, "case {c}: sample {i}");
                }
            }
        }
    }

    #[test]
    fn an_integer_lag_copies_the_history_almost_exactly() {
        // At frac 0 the interpolator is a near-unity filter, so the output
        // should track the history one period back. Not exact -- the filter
        // has a -3 dB point below Nyquist -- but strongly correlated.
        let history = history();
        let lag = 100usize;
        let mut buffer = history.clone();
        buffer.resize(HISTORY + L_SUBFR, Word16(0));
        predict(&mut buffer, HISTORY, lag, 0, L_SUBFR);
        let out = buffer[HISTORY..].to_vec();

        let reference = &history[HISTORY - lag..][..L_SUBFR];
        let (mut dot, mut norm_a, mut norm_b) = (0i64, 0i64, 0i64);
        for (a, b) in out.iter().zip(reference.iter()) {
            dot += i64::from(a.0) * i64::from(b.0);
            norm_a += i64::from(a.0) * i64::from(a.0);
            norm_b += i64::from(b.0) * i64::from(b.0);
        }
        #[allow(clippy::cast_precision_loss, clippy::suboptimal_flops)]
        let correlation = dot as f64 / ((norm_a as f64).sqrt() * (norm_b as f64).sqrt());
        assert!(
            correlation > 0.95,
            "integer-lag prediction correlates only {correlation:.3} with the history"
        );
    }

    #[test]
    fn each_fraction_gives_a_distinct_vector() {
        // If the phase selection were dropped, all four fractions would return
        // the same samples -- a bug that still sounds like speech, just rough.
        let history = history();
        let mut seen: Vec<[i16; 8]> = Vec::new();
        for frac in 0..4u8 {
            let mut buffer = history.clone();
            buffer.resize(HISTORY + L_SUBFR, Word16(0));
            predict(&mut buffer, HISTORY, 64, frac, L_SUBFR);
            let head: [i16; 8] = std::array::from_fn(|i| buffer[HISTORY + i].0);
            assert!(
                !seen.contains(&head),
                "fraction {frac} produced a vector already seen"
            );
            seen.push(head);
        }
    }

    #[test]
    fn sharpening_leaves_the_first_period_untouched() {
        // There is no predecessor a period back for the start of a subframe.
        for lag in [34usize, 64, 100] {
            let mut code = innovation(0, 0);
            let before = code;
            sharpen(&mut code, lag, PIT_SHARP);
            for i in 0..lag.min(L_SUBFR) {
                assert_eq!(code[i], before[i], "lag {lag}: sample {i} was modified");
            }
        }
    }

    #[test]
    fn the_sharpening_lag_rounds_rather_than_truncates() {
        assert_eq!(sharpening_lag(64, 0), 64);
        assert_eq!(sharpening_lag(64, 2), 64);
        assert_eq!(sharpening_lag(64, 3), 65);
    }
}
