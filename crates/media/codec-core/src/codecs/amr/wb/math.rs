//! Transcendental primitives for AMR-WB, from TS 26.173 `math_op.c`/`log2.c`.
//!
//! Table lookup plus one linear interpolation, in every case. That is not a
//! shortcut around a "real" implementation — it *is* the definition, so a more
//! accurate `log2` here produces a different bitstream and fails conformance.
//!
//! These are separate from the G.729 versions in
//! [`crate::codecs::g729`] on purpose. Both codecs have tables for the same
//! functions with different values; sharing them would give a codec that
//! sounds nearly right and is not conformant.

use super::gain_tables::{ISQRT_TABLE, LOG2_TABLE, POW2_TABLE};
use crate::fixed_point::arith::{extract_h, extract_l, negate, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu, l_mult};
use crate::fixed_point::shift::{l_shl, l_shr, l_shr_r, norm_l};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// A value as `frac × 2^exp`, which is how the reference passes these around.
pub type Normalised = (Word32, i16);

/// `1/sqrt(frac × 2^exp)`.
///
/// # Panics
///
/// If `frac` is not normalised, which would put the table index out of range.
///
/// Non-positive input yields `(1.0, 0)` rather than an error: `1/sqrt(0)` has
/// no useful value, and returning the largest representable result is what
/// keeps the callers' arithmetic in range.
#[must_use]
pub fn isqrt_n(ctx: &mut DspContext, value: Normalised) -> Normalised {
    let (mut frac, mut exp) = value;
    if frac.0 <= 0 {
        return (Word32(0x7fff_ffff), 0);
    }

    // An odd exponent cannot be halved, so borrow one from the mantissa.
    if exp & 1 == 1 {
        frac = l_shr(ctx, frac, 1);
    }
    exp = negate(ctx, Word16((exp - 1) >> 1)).0;

    // Bits 25..31 select the table entry, 10..24 interpolate within it.
    frac = l_shr(ctx, frac, 9);
    let i = usize::try_from(extract_h(frac).0 - 16).expect("normalised input keeps the index in range");
    frac = l_shr(ctx, frac, 1);
    let a = Word16(extract_l(frac).0 & 0x7fff);

    let mut result = l_deposit_h(Word16(ISQRT_TABLE[i]));
    let slope = sub(ctx, Word16(ISQRT_TABLE[i]), Word16(ISQRT_TABLE[i + 1]));
    result = l_msu(ctx, result, slope, a);

    (result, exp)
}

/// `2^(exponent + fraction)`, with `fraction` in Q15.
///
/// # Panics
///
/// If `fraction` is negative, which is not a valid Q15 fraction here.
#[must_use]
pub fn pow2(ctx: &mut DspContext, exponent: i16, fraction: Word16) -> Word32 {
    let scaled = l_mult(ctx, fraction, Word16(32));
    let i = usize::try_from(extract_h(scaled).0).expect("a Q15 fraction is non-negative");
    let a = Word16(extract_l(l_shr(ctx, scaled, 1)).0 & 0x7fff);

    let mut result = l_deposit_h(Word16(POW2_TABLE[i]));
    let slope = sub(ctx, Word16(POW2_TABLE[i]), Word16(POW2_TABLE[i + 1]));
    result = l_msu(ctx, result, slope, a);

    l_shr_r(ctx, result, 30 - exponent)
}

/// `log2(x)`, split into integer and Q15 fractional parts.
///
/// Returns `(0, 0)` for non-positive input.
///
/// # Panics
///
/// Never for positive input: normalisation pins the table index in range.
#[must_use]
pub fn log2(ctx: &mut DspContext, x: Word32) -> (i16, Word16) {
    if x.0 <= 0 {
        return (0, Word16(0));
    }
    let shift = norm_l(x);
    let normalised = l_shl(ctx, x, shift);

    let exponent = 30 - shift;
    let shifted = l_shr(ctx, normalised, 9);
    let i = usize::try_from(extract_h(shifted).0 - 32)
        .expect("normalised input keeps the index in range");
    let a = Word16(extract_l(l_shr(ctx, shifted, 1)).0 & 0x7fff);

    let mut y = l_deposit_h(Word16(LOG2_TABLE[i]));
    let slope = sub(ctx, Word16(LOG2_TABLE[i]), Word16(LOG2_TABLE[i + 1]));
    y = l_msu(ctx, y, slope, a);

    (exponent, extract_h(y))
}

/// Normalised dot product of two vectors, as `(Q31 value, exponent 0..30)`.
///
/// The accumulator starts at 1 rather than 0 so an all-zero input still
/// normalises, instead of leaving `norm_l` to describe nothing.
#[must_use]
pub fn dot_product12(ctx: &mut DspContext, x: &[Word16], y: &[Word16]) -> Normalised {
    let mut sum = Word32(1);
    for (&a, &b) in x.iter().zip(y.iter()) {
        sum = l_mac(ctx, sum, a, b);
    }
    let shift = norm_l(sum);
    (l_shl(ctx, sum, shift), 30 - shift)
}

/// Median of five values.
///
/// Used to pick a representative gain from recent history during concealment,
/// where a mean would be dragged by a single outlier — exactly the case a lost
/// frame creates.
#[must_use]
pub const fn median5(x: &[Word16; 5]) -> Word16 {
    let mut v = [x[0].0, x[1].0, x[2].0, x[3].0, x[4].0];
    // The reference's fixed comparison network, which is why this is not a
    // sort: it does the minimum work to place the middle element.
    if v[1] < v[0] {
        v.swap(0, 1);
    }
    if v[2] < v[0] {
        v.swap(0, 2);
    }
    if v[3] < v[0] {
        v.swap(0, 3);
    }
    if v[4] < v[0] {
        v[4] = v[0];
    }
    if v[2] < v[1] {
        v.swap(1, 2);
    }
    if v[3] < v[1] {
        v.swap(1, 3);
    }
    if v[4] < v[1] {
        v[4] = v[1];
    }
    if v[3] < v[2] {
        v[2] = v[3];
    }
    if v[4] < v[2] {
        v[2] = v[4];
    }
    Word16(v[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median5_picks_the_middle_value() {
        let cases: [([i16; 5], i16); 5] = [
            ([1, 2, 3, 4, 5], 3),
            ([5, 4, 3, 2, 1], 3),
            ([3, 1, 4, 1, 5], 3),
            ([-100, 0, 100, 0, 0], 0),
            ([7, 7, 7, 7, 7], 7),
        ];
        for (input, want) in cases {
            let got = median5(&input.map(Word16));
            assert_eq!(got.0, want, "median of {input:?}");
        }
    }

    #[test]
    fn median5_resists_an_outlier_far_better_than_a_mean() {
        // The reason it is a median and not a mean: one lost frame must not
        // drag the concealed gain. A median is not *unchanged* by an outlier
        // -- replacing the smallest value with the largest shifts it up one
        // rank -- so the honest claim is that it moves far less than the mean.
        let steady = [1000i16, 1010, 990, 1005, 995];
        let spiked = [1000i16, 1010, 32000, 1005, 995];

        let mean = |v: [i16; 5]| v.iter().map(|&x| i32::from(x)).sum::<i32>() / 5;
        let median_shift =
            (i32::from(median5(&spiked.map(Word16)).0) - i32::from(median5(&steady.map(Word16)).0))
                .abs();
        let mean_shift = (mean(spiked) - mean(steady)).abs();

        assert!(
            median_shift * 100 < mean_shift,
            "median moved {median_shift} against the mean's {mean_shift}"
        );
    }

    #[test]
    fn pow2_and_log2_round_trip_approximately() {
        // Not exact -- both are table-interpolated -- but a composition that
        // drifts badly means one of them is wrong.
        let mut ctx = DspContext::default();
        for x in [1000i32, 10_000, 100_000, 1_000_000, 16_777_216] {
            let (exp, frac) = log2(&mut ctx, Word32(x));
            let back = pow2(&mut ctx, exp, frac);
            let error = (f64::from(back.0) - f64::from(x)).abs() / f64::from(x);
            assert!(error < 0.01, "log2/pow2 round trip of {x} gave {} ({error:.4})", back.0);
        }
    }

    #[test]
    fn isqrt_n_approximates_the_reciprocal_square_root() {
        let mut ctx = DspContext::default();
        for x in [1i32 << 20, 1 << 24, 1 << 28, 3 << 25] {
            // Normalise first, as the callers do.
            let shift = norm_l(Word32(x));
            let normalised = l_shl(&mut ctx, Word32(x), shift);
            let (frac, exp) = isqrt_n(&mut ctx, (normalised, 31 - shift));

            // Not `1i32 << 31`: that is i32::MIN, and dividing by it flips the sign.
            let got = f64::from(frac.0) / 2f64.powi(31) * 2f64.powi(i32::from(exp));
            let want = 1.0 / f64::from(x).sqrt();
            let error = (got - want).abs() / want;
            assert!(error < 0.01, "isqrt({x}) gave {got}, want {want} ({error:.4})");
        }
    }

    #[test]
    fn a_zero_vector_still_normalises() {
        // The accumulator starts at 1 for exactly this case.
        let mut ctx = DspContext::default();
        let zeros = [Word16(0); 64];
        let (value, exp) = dot_product12(&mut ctx, &zeros, &zeros);
        assert!(value.0 > 0, "zero dot product did not normalise");
        assert!((0..=30).contains(&exp), "exponent {exp} out of range");
    }
}
