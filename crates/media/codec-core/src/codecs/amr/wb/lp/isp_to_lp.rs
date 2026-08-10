//! ISP to LP conversion, 3GPP TS 26.190 §5.2.4, in fixed point.
//!
//! Turns quantised and interpolated ISPs back into predictor coefficients, so
//! the synthesis and weighting filters can be built. **This is decoder-side
//! work** — the decoder receives quantised ISFs and never analyses — which is
//! why it comes before the encoder-only analysis stages.
//!
//! From the spec:
//!
//! > The coefficients of F1(z) and F2(z) are found by expanding Equations (14)
//! > and (15) knowing the quantized and interpolated ISPs […] with initial
//! > values f1(0)=1 and f1(1)=-2q0. The coefficients f2(i) are computed
//! > similarly by replacing q2i-2 by q2i-1 and m/2 by m/2-1, and with initial
//! > conditions f2(0)=1 and f2(1)=-2q1.
//!
//! # Shape
//!
//! `f1` is built from the even-indexed ISPs and `f2` from the odd — the two
//! interlaced root sets. `f2` is then multiplied by `(1 - z⁻²)`, restoring the
//! roots at `z = ±1` that [`super`]'s forward conversion divided out. Both are
//! scaled by the last ISP, which is not a root at all but the final predictor
//! coefficient carried alongside them. `A(z)` is then `(F1 + F2)/2`, using the
//! symmetry of one and the antisymmetry of the other to fill both ends of the
//! coefficient vector from a single loop.
//!
//! # Q-formats
//!
//! ISPs arrive Q15. The polynomial expansion runs in Q23, which is where the
//! headroom for repeated accumulation lives. Output coefficients are Q12, so
//! the final shift is 12 — carrying the halving of `(F1 + F2)/2` for free.

use super::autocorr::LP_ORDER;
use crate::fixed_point::arith::extract_l;
use crate::fixed_point::arith32::{l_add, l_msu, l_mult, l_sub};
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr_r, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Half the predictor order: the number of ISP pairs.
const NC: usize = LP_ORDER / 2;

/// Expand one interlaced root set into its polynomial, in Q23.
///
/// `isp` is read with a stride of two, starting from whichever of the two
/// interlaced sets the caller wants. `f` receives `n + 1` coefficients.
///
/// Each step folds in one more root pair via the standard
/// `f[k] ← f[k] - 2q·f[k-1] + f[k-2]` recursion, worked downwards so both
/// referenced neighbours still hold their previous-order values.
fn expand_roots(isp: &[Word16], f: &mut [Word32], n: usize) {
    let mut ctx = DspContext::default();

    // 1.0 and -2·q₀, both in Q23.
    f[0] = l_mult(&mut ctx, Word16(4096), Word16(1024));
    f[1] = l_mult(&mut ctx, isp[0], Word16(-256));

    for i in 2..=n {
        // The new top coefficient starts from two orders down.
        f[i] = f[i - 2];

        // Walk downwards so f[k-1] and f[k-2] are still previous-order values.
        let q = isp[(i - 1) * 2];
        for k in (2..=i).rev() {
            let (hi, lo) = l_extract(f[k - 1]);
            let term = l_shl(&mut ctx, mpy_32_16(hi, lo, q), 1);
            f[k] = l_sub(&mut ctx, f[k], term);
            f[k] = l_add(&mut ctx, f[k], f[k - 2]);
        }
        // The order-1 term has no f[k-2] neighbour; it just accumulates -2q.
        f[1] = l_msu(&mut ctx, f[1], q, Word16(256));
    }
}

/// Convert ISPs to predictor coefficients.
///
/// `isp` is `q[0..16]` in Q15: fifteen interlaced roots plus the last predictor
/// coefficient. Returns `a[0..=16]` in Q12 with `a[0] = 1.0`.
///
/// Adaptive scaling is not applied — the reference makes it a caller option,
/// and the decoder path uses it disabled.
#[must_use]
pub fn isp_to_lp(isp: &[Word16; LP_ORDER]) -> [Word16; LP_ORDER + 1] {
    let mut ctx = DspContext::default();

    // f1 from the even-indexed roots, f2 from the odd.
    let mut f1 = [Word32(0); NC + 1];
    let mut f2 = [Word32(0); NC + 1];
    expand_roots(&isp[0..], &mut f1, NC);
    expand_roots(&isp[1..], &mut f2, NC - 1);

    // Multiply F2(z) by (1 - z⁻²), restoring the roots at z = ±1.
    for i in (2..NC).rev() {
        f2[i] = l_sub(&mut ctx, f2[i], f2[i - 2]);
    }

    // Scale F1 by (1 + q₁₅) and F2 by (1 - q₁₅). The last ISP is the final
    // predictor coefficient rather than a root, and this is where it enters.
    let last = isp[LP_ORDER - 1];
    for i in 0..NC {
        let (hi, lo) = l_extract(f1[i]);
        let scaled = mpy_32_16(hi, lo, last);
        f1[i] = l_add(&mut ctx, f1[i], scaled);

        let (hi, lo) = l_extract(f2[i]);
        let scaled = mpy_32_16(hi, lo, last);
        f2[i] = l_sub(&mut ctx, f2[i], scaled);
    }

    // A(z) = (F1(z) + F2(z))/2. F1 is symmetric and F2 antisymmetric, so one
    // pass fills both ends: the sum gives the front, the difference the back.
    let mut a = [Word16(0); LP_ORDER + 1];
    a[0] = Word16(4096);
    for i in 1..NC {
        let j = LP_ORDER - i;
        let sum = l_add(&mut ctx, f1[i], f2[i]);
        a[i] = extract_l(l_shr_r(&mut ctx, sum, 12));
        let diff = l_sub(&mut ctx, f1[i], f2[i]);
        a[j] = extract_l(l_shr_r(&mut ctx, diff, 12));
    }

    // The middle coefficient comes from F1 alone, since F2's antisymmetry makes
    // its centre term vanish.
    let (hi, lo) = l_extract(f1[NC]);
    let scaled = mpy_32_16(hi, lo, last);
    let centre = l_add(&mut ctx, f1[NC], scaled);
    a[NC] = extract_l(l_shr_r(&mut ctx, centre, 12));

    // And the last coefficient is the last ISP, Q15 to Q12.
    a[LP_ORDER] = shr_r(&mut ctx, last, 3);

    a
}

/// Readers for the TS 26.173 per-stage dump, shared across the LP modules.
///
/// Panicking on a missing case or row is the intended behaviour: a fixture
/// that no longer holds what a test asks for is a broken fixture, and a loud
/// failure beats a test that silently checks nothing.
#[cfg(test)]
#[allow(clippy::missing_panics_doc, clippy::must_use_candidate)]
pub mod tests_support {
    const LP_STAGES: &str = include_str!("../../testdata/lp_stages_wb.txt");

    /// One labelled row of one case, as integers.
    pub fn row(case: usize, label: &str) -> Vec<i16> {
        let marker = format!("case {case}\n");
        let block = LP_STAGES
            .split(&marker)
            .nth(1)
            .unwrap_or_else(|| panic!("case {case} missing"));
        let block = block.split("\ncase ").next().unwrap_or(block);
        for line in block.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("case {case} has no row {label:?}");
    }

    /// How many cases the dump holds.
    pub fn case_count() -> usize {
        LP_STAGES.lines().filter(|l| l.starts_with("case ")).count()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{case_count, row};
    use super::*;

    #[test]
    fn isp_to_lp_is_bit_exact_against_ts26173() {
        for case in 0..case_count() {
            let isp_row = row(case, "isp");
            assert_eq!(isp_row.len(), LP_ORDER, "case {case}: isp length");
            let mut isp = [Word16(0); LP_ORDER];
            for (slot, &v) in isp.iter_mut().zip(isp_row.iter()) {
                *slot = Word16(v);
            }

            let got = isp_to_lp(&isp);
            let want = row(case, "a_back");
            assert_eq!(want.len(), LP_ORDER + 1, "case {case}: a_back length");

            for i in 0..=LP_ORDER {
                assert_eq!(
                    got[i].0, want[i],
                    "case {case}: a[{i}] = {} but the reference gives {}",
                    got[i].0, want[i]
                );
            }
        }
    }

    #[test]
    fn the_leading_coefficient_is_unity_in_q12() {
        // a[0] = 1.0 by definition; the recursion must not overwrite it.
        for case in 0..case_count() {
            let isp_row = row(case, "isp");
            let mut isp = [Word16(0); LP_ORDER];
            for (slot, &v) in isp.iter_mut().zip(isp_row.iter()) {
                *slot = Word16(v);
            }
            assert_eq!(isp_to_lp(&isp)[0].0, 4096, "case {case}");
        }
    }

    #[test]
    fn the_last_coefficient_carries_the_last_isp() {
        // The sixteenth ISP is not a root — it is a[16], converted Q15 to Q12.
        for case in 0..case_count() {
            let isp_row = row(case, "isp");
            let mut isp = [Word16(0); LP_ORDER];
            for (slot, &v) in isp.iter_mut().zip(isp_row.iter()) {
                *slot = Word16(v);
            }
            let a = isp_to_lp(&isp);
            let mut ctx = DspContext::default();
            let expected = shr_r(&mut ctx, isp[LP_ORDER - 1], 3);
            assert_eq!(a[LP_ORDER].0, expected.0, "case {case}");
        }
    }
}
