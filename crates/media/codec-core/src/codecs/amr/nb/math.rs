//! AMR-NB's table-driven transcendentals, TS 26.073 `log2.c`, `pow2.c`,
//! `sqrt_l.c` and `inv_sqrt.c`.
//!
//! # Why these are not shared
//!
//! Every fixed-point speech codec in this crate needs `log2`, `2^x` and a
//! square root, and every one of them tabulates those functions over its own
//! points. G.729's live in [`crate::codecs::g729::impls::math`]; AMR-WB's in
//! `wb::math`; these are AMR-NB's. Sharing any pair would give a codec that
//! sounds nearly right and fails conformance, so the tables — and therefore the
//! code that interpolates them — stay with their codec. Only the genuinely
//! table-free arithmetic is shared, in [`crate::fixed_point`].
//!
//! # The common shape
//!
//! All four are the same idea: normalise the input, take some top bits as a
//! table index and the next fifteen as an interpolation fraction, then walk one
//! linear segment between adjacent entries. The tables *are* the function, not
//! an approximation of it — a more accurate logarithm produces different bits
//! and fails conformance.
//!
//! Where they differ is which bits become the index, and that difference is not
//! cosmetic: `log2`, `sqrt` and `inv_sqrt` shift right by nine first because
//! their input is a normalised `Word32`, while `pow2`'s input is a Q15 fraction
//! that is already in position. Sharing one extraction helper across all four
//! would have to smuggle that shift back in.

use super::decoder_tables::{INV_SQRT_TABLE, LOG2_TABLE, POW2_TABLE, SQRT_L_TABLE};
use crate::fixed_point::arith::{add, extract_h, extract_l, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_msu, l_mult};
use crate::fixed_point::shift::{l_shl, l_shr, l_shr_r, norm_l, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Walk one linear segment of a lookup table.
///
/// Returns the interpolated value in the high half of a `Word32`, which is what
/// every caller wants: `log2` and `pow2` take its high half as a Q15 fraction,
/// the two roots keep the full word.
fn interpolate(ctx: &mut DspContext, table: &[i16], index: usize, a: Word16) -> Word32 {
    let l_y = l_deposit_h(Word16(table[index]));
    let tmp = sub(ctx, Word16(table[index]), Word16(table[index + 1]));
    l_msu(ctx, l_y, tmp, a)
}

/// Split a positioned value into a table index and a Q15 interpolation
/// fraction.
///
/// The mask on the fraction is load-bearing. Without it the sign bit of the
/// extracted half-word leaks in, which on the wideband side showed up as a 2%
/// error in a `log2`/`pow2` round trip and took a while to find because the
/// result stayed plausible.
fn index_and_fraction(ctx: &mut DspContext, l_x: Word32) -> (i16, Word16) {
    let i = extract_h(l_x);
    let l_x = l_shr(ctx, l_x, 1);
    let a = extract_l(l_x);
    (i.0, Word16(a.0 & 0x7fff))
}

/// `log2` of an already-normalised value, TS 26.073 `Log2_norm`.
///
/// `exp` is the normalisation shift that produced `l_x`. Returns the integer
/// part and the Q15 fractional part.
///
/// A non-positive input returns `(0, 0)` rather than an error: the callers feed
/// this energies, and a silent frame legitimately has none.
///
/// # Panics
///
/// Panics if `l_x` is not normalised to `exp`, since the table index would then
/// be out of range. That is a caller error rather than bad data.
pub fn log2_norm(ctx: &mut DspContext, l_x: Word32, exp: i16) -> (Word16, Word16) {
    if l_x.0 <= 0 {
        return (Word16(0), Word16(0));
    }

    let exponent = sub(ctx, Word16(30), Word16(exp));

    let positioned = l_shr(ctx, l_x, 9);
    let (i, a) = index_and_fraction(ctx, positioned);
    // Normalisation puts the leading one in bit 30, so the extracted index is
    // always 32..=63; this brings it into the table's 0..=31. The conversion
    // asserts that invariant rather than wrapping if it is ever broken.
    let i = usize::try_from(i - 32).expect("log2 index is normalised into range");

    let l_y = interpolate(ctx, &LOG2_TABLE, i, a);
    (exponent, extract_h(l_y))
}

/// `log2` of an arbitrary `Word32`, TS 26.073 `Log2`.
pub fn log2(ctx: &mut DspContext, l_x: Word32) -> (Word16, Word16) {
    let exp = norm_l(l_x);
    let normalised = l_shl(ctx, l_x, exp);
    log2_norm(ctx, normalised, exp)
}

/// `2^x`, TS 26.073 `Pow2`.
///
/// `exponent` is the integer part (0..=30) and `fraction` the Q15 fractional
/// part. The result is a `Word32` scaled so that the integer part lands at bit
/// `exponent`.
///
/// # Panics
///
/// Panics on a negative `fraction`, which is outside the function's domain.
pub fn pow2(ctx: &mut DspContext, exponent: Word16, fraction: Word16) -> Word32 {
    // `L_mult` doubles as it multiplies, so this is `fraction << 6`: it puts the
    // fraction's top five bits where `extract_h` reads them as the index. No
    // shift by nine here — unlike the three functions above, the input is
    // already a Q15 fraction rather than a normalised word.
    let l_x = l_mult(ctx, fraction, Word16(32));
    let (i, a) = index_and_fraction(ctx, l_x);
    let i = usize::try_from(i).expect("pow2 index is non-negative");
    let l_x = interpolate(ctx, &POW2_TABLE, i, a);

    let exp = sub(ctx, Word16(30), exponent);
    l_shr_r(ctx, l_x, exp.0)
}

/// Square root with the denormalisation left to the caller, TS 26.073
/// `sqrt_l_exp`.
///
/// Returns `(y, e)`, where the caller recovers the root as `y >> (e / 2)`.
/// Splitting it this way is not an optimisation: `Dec_gain`'s energy measure
/// needs the exponent separately, and rounding the shift here would lose bits
/// it depends on.
///
/// # Panics
///
/// Panics only if the even-normalisation invariant is broken, which cannot
/// happen for a positive input.
pub fn sqrt_l_exp(ctx: &mut DspContext, l_x: Word32) -> (Word32, i16) {
    if l_x.0 <= 0 {
        return (Word32(0), 0);
    }

    // The *next lower even* normalisation shift. A square root halves the
    // exponent, so an odd shift would leave half a bit to carry; forcing it
    // even makes the halving exact, at the cost of normalising into [0.25, 1)
    // instead of [0.5, 1) — which is why the table has 49 entries rather than
    // 33 and the index base is 16 rather than 32.
    let e = norm_l(l_x) & !1;
    let l_x = l_shl(ctx, l_x, e);

    let positioned = l_shr(ctx, l_x, 9);
    let (i, a) = index_and_fraction(ctx, positioned);
    // The even normalisation leaves the index in 16..=63.
    let i = usize::try_from(i - 16).expect("sqrt index is normalised into range");

    (interpolate(ctx, &SQRT_L_TABLE, i, a), e)
}

/// `1/sqrt(x)`, TS 26.073 `Inv_sqrt`, denormalised in place.
///
/// A non-positive input returns `0x3fff_ffff` — the reference's saturated
/// stand-in for an infinite result, which keeps the gain predictor bounded on a
/// silent frame instead of dividing by zero.
///
/// # Panics
///
/// Panics only if the normalisation invariant is broken, which cannot happen
/// for a positive input.
pub fn inv_sqrt(ctx: &mut DspContext, l_x: Word32) -> Word32 {
    if l_x.0 <= 0 {
        return Word32(0x3fff_ffff);
    }

    let norm = norm_l(l_x);
    let mut l_x = l_shl(ctx, l_x, norm);

    let mut exp = sub(ctx, Word16(30), Word16(norm));
    // An *even* exponent is the one that needs fixing here, because the halving
    // below rounds toward zero: absorbing the odd bit into the mantissa is what
    // makes the remaining shift exactly `exp / 2`.
    if (exp.0 & 1) == 0 {
        l_x = l_shr(ctx, l_x, 1);
    }
    exp = shr(ctx, exp, 1);
    exp = add(ctx, exp, Word16(1));

    let positioned = l_shr(ctx, l_x, 9);
    let (i, a) = index_and_fraction(ctx, positioned);
    let i = usize::try_from(i - 16).expect("inv_sqrt index is normalised into range");

    let l_y = interpolate(ctx, &INV_SQRT_TABLE, i, a);
    l_shr(ctx, l_y, exp.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These check *properties*, not conformance. Bit-exactness is asserted by
    /// the modules that consume these against the reference's own vectors; what
    /// these catch is a table read backwards or an index base off by sixteen,
    /// which would still produce smooth, plausible curves.
    fn ctx() -> DspContext {
        DspContext::default()
    }

    #[test]
    fn log2_recovers_the_exponent_of_a_power_of_two() {
        let mut c = ctx();
        for shift in 1..=30 {
            let (exponent, fraction) = log2(&mut c, Word32(1 << shift));
            assert_eq!(exponent.0, shift as i16, "log2(2^{shift})");
            // A power of two lands exactly on a table entry, so the fraction is
            // zero rather than merely small. An index base off by one would
            // show up here and nowhere else in this test.
            assert_eq!(fraction.0, 0, "2^{shift} has no fractional log2");
        }
    }

    #[test]
    fn log2_is_monotonic_and_tracks_the_real_logarithm() {
        let mut c = ctx();
        let mut last = f64::MIN;
        for x in (1..=0x3fff_ffffi32).step_by(0x0010_0000) {
            let (e, f) = log2(&mut c, Word32(x));
            let got = f64::from(e.0) + f64::from(f.0) / 32768.0;
            let want = f64::from(x).log2();
            assert!((got - want).abs() < 0.001, "log2({x}) = {got}, want {want}");
            assert!(got > last, "log2 must increase; {got} followed {last}");
            last = got;
        }
    }

    #[test]
    fn pow2_inverts_log2() {
        let mut c = ctx();
        for &x in &[3i32, 100, 12_345, 1_000_000, 0x3fff_ffff] {
            let (exponent, fraction) = log2(&mut c, Word32(x));
            let back = pow2(&mut c, exponent, fraction);
            let rel = (f64::from(back.0) - f64::from(x)).abs() / f64::from(x);
            assert!(rel < 0.001, "2^log2({x}) = {} (relative {rel})", back.0);
        }
    }

    #[test]
    fn sqrt_squares_back_to_its_input() {
        let mut c = ctx();
        for &x in &[1i32 << 20, 1 << 25, 12_345_678, 0x3fff_ffff] {
            let (y, e) = sqrt_l_exp(&mut c, Word32(x));
            // The reference's convention: y is Q31 and e is twice the shift
            // still owed, so the root of the Q31 input is y * 2^(-e/2).
            let root = f64::from(y.0) / 2f64.powi(31) / 2f64.powi(i32::from(e) / 2);
            let want = f64::from(x) / 2f64.powi(31);
            assert!(
                (root * root - want).abs() / want < 0.001,
                "sqrt({x})^2 = {} , want {want}",
                root * root
            );
        }
    }

    #[test]
    fn inv_sqrt_is_the_reciprocal_of_the_integer_root() {
        // The two roots use *different conventions*, which is worth pinning
        // rather than discovering downstream: `sqrt_l_exp` treats its input as
        // Q31 and hands the denormalisation back to the caller, while
        // `Inv_sqrt` treats its input as a plain integer and returns Q30. They
        // are not each other's inverse as written, and the gain predictor
        // depends on exactly this.
        let mut c = ctx();
        for &x in &[1i32 << 20, 1 << 25, 12_345_678, 0x3fff_ffff] {
            let got = f64::from(inv_sqrt(&mut c, Word32(x)).0) / 2f64.powi(30);
            let want = 1.0 / f64::from(x).sqrt();
            assert!(
                (got - want).abs() / want < 0.001,
                "inv_sqrt({x}) = {got}, want {want}"
            );
        }
    }

    #[test]
    fn degenerate_inputs_return_the_references_stand_ins() {
        let mut c = ctx();
        // Silence is a legitimate input, not an error: every one of these is
        // called on an energy, and a silent frame has none.
        assert_eq!(log2(&mut c, Word32(0)), (Word16(0), Word16(0)));
        assert_eq!(log2(&mut c, Word32(-1)), (Word16(0), Word16(0)));
        assert_eq!(sqrt_l_exp(&mut c, Word32(0)), (Word32(0), 0));
        // Not zero: an infinite reciprocal saturates instead, so the gain
        // predictor stays bounded rather than exploding on a silent frame.
        assert_eq!(inv_sqrt(&mut c, Word32(0)).0, 0x3fff_ffff);
    }
}
