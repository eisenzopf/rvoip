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
use crate::fixed_point::shift::{l_shl, l_shr_r, norm_l, shr, shr_r};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Expand one interlaced root set into its polynomial, in Q23.
///
/// `isp` is read with a stride of two, starting from whichever of the two
/// interlaced sets the caller wants. `f` receives `n + 1` coefficients.
///
/// Each step folds in one more root pair via the standard
/// `f[k] ← f[k] - 2q·f[k-1] + f[k-2]` recursion, worked downwards so both
/// referenced neighbours still hold their previous-order values.
fn expand_roots(isp: &[Word16], f: &mut [Word32], n: usize) {
    expand_roots_scaled(isp, f, n, Word16(1024), Word16(256));
}

/// [`expand_roots`] at a caller-chosen scale.
///
/// Order 20 runs the same recursion four times smaller and shifts back
/// afterwards, because twenty accumulations overflow Q23 where sixteen do not.
fn expand_roots_scaled(isp: &[Word16], f: &mut [Word32], n: usize, unit: Word16, step: Word16) {
    let mut ctx = DspContext::default();

    // 1.0 and -2·q₀.
    f[0] = l_mult(&mut ctx, Word16(4096), unit);
    f[1] = l_mult(&mut ctx, isp[0], Word16(-step.0));

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
        f[1] = l_msu(&mut ctx, f[1], q, step);
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
    let mut a = [Word16(0); LP_ORDER + 1];
    isp_to_lp_order(isp, &mut a);
    a
}

/// The same conversion with adaptive scaling, `Isp_Az(..., 1)`.
///
/// The speech paths all pass 0 and this is the only caller that passes 1: the
/// decoder's comfort-noise branch. When the expanded polynomial has grown
/// large enough that the Q12 result would clip, every coefficient is shifted
/// down together and the filter is rescaled rather than saturated. On a
/// comfort-noise spectrum that is not a rare corner — it fires routinely,
/// which is why the branch exists at all.
#[must_use]
pub fn isp_to_lp_adaptive(isp: &[Word16; LP_ORDER]) -> [Word16; LP_ORDER + 1] {
    let mut a = [Word16(0); LP_ORDER + 1];
    isp_to_lp_scaled(isp, &mut a, true);
    a
}

/// Convert ISPs to predictor coefficients at any even order.
///
/// The high band uses order 20, so this cannot be fixed at [`LP_ORDER`].
/// Above order 16 the polynomial expansion runs four times smaller and is
/// shifted back, which is what keeps twenty accumulations inside Q23.
///
/// # Panics
///
/// If `a` is not exactly one longer than `isp`, or the order is odd.
pub fn isp_to_lp_order(isp: &[Word16], a: &mut [Word16]) {
    isp_to_lp_scaled(isp, a, false);
}

/// The conversion, with adaptive scaling optional.
///
/// # Panics
///
/// If `a` is not exactly one longer than `isp`, or the order is odd.
fn isp_to_lp_scaled(isp: &[Word16], a: &mut [Word16], adaptive: bool) {
    let order = isp.len();
    assert_eq!(a.len(), order + 1, "a must be one longer than isp");
    assert_eq!(order % 2, 0, "the predictor order must be even");
    let nc = order / 2;
    let wide = nc > 8;

    let mut ctx = DspContext::default();
    let mut f1 = vec![Word32(0); nc + 1];
    let mut f2 = vec![Word32(0); nc + 1];

    if wide {
        expand_roots_scaled(&isp[0..], &mut f1, nc, Word16(256), Word16(64));
        expand_roots_scaled(&isp[1..], &mut f2, nc - 1, Word16(256), Word16(64));
        for v in f1.iter_mut().take(nc + 1) {
            *v = l_shl(&mut ctx, *v, 2);
        }
        for v in f2.iter_mut().take(nc) {
            *v = l_shl(&mut ctx, *v, 2);
        }
    } else {
        expand_roots(&isp[0..], &mut f1, nc);
        expand_roots(&isp[1..], &mut f2, nc - 1);
    }

    // Multiply F2(z) by (1 - z⁻²), restoring the roots at z = ±1.
    for i in (2..nc).rev() {
        f2[i] = l_sub(&mut ctx, f2[i], f2[i - 2]);
    }

    let last = isp[order - 1];
    for i in 0..nc {
        let (hi, lo) = l_extract(f1[i]);
        let scaled = mpy_32_16(hi, lo, last);
        f1[i] = l_add(&mut ctx, f1[i], scaled);

        let (hi, lo) = l_extract(f2[i]);
        let scaled = mpy_32_16(hi, lo, last);
        f2[i] = l_sub(&mut ctx, f2[i], scaled);
    }

    a[0] = Word16(4096);
    // The largest magnitude seen while forming the coefficients, which is what
    // the adaptive branch below sizes its shift from.
    let mut tmax = Word32(0);
    for i in 1..nc {
        let j = order - i;
        let sum = l_add(&mut ctx, f1[i], f2[i]);
        tmax = Word32(tmax.0 | sum.0.saturating_abs());
        a[i] = extract_l(l_shr_r(&mut ctx, sum, 12));
        let diff = l_sub(&mut ctx, f1[i], f2[i]);
        tmax = Word32(tmax.0 | diff.0.saturating_abs());
        a[j] = extract_l(l_shr_r(&mut ctx, diff, 12));
    }

    // Rescale and redo the loop when the result would not fit Q12.
    let q = if adaptive { 4 - norm_l(tmax) } else { 0 };
    let q_sug = if q > 0 {
        let q_sug = 12 + q;
        for i in 1..nc {
            let j = order - i;
            let sum = l_add(&mut ctx, f1[i], f2[i]);
            a[i] = extract_l(l_shr_r(&mut ctx, sum, q_sug));
            let diff = l_sub(&mut ctx, f1[i], f2[i]);
            a[j] = extract_l(l_shr_r(&mut ctx, diff, q_sug));
        }
        a[0] = shr(&mut ctx, a[0], q);
        q_sug
    } else {
        12
    };
    let q = q.max(0);

    let (hi, lo) = l_extract(f1[nc]);
    let scaled = mpy_32_16(hi, lo, last);
    let centre = l_add(&mut ctx, f1[nc], scaled);
    a[nc] = extract_l(l_shr_r(&mut ctx, centre, q_sug));
    a[order] = shr_r(&mut ctx, last, 3 + q);
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

    /// The indented rows belonging to a named block.
    ///
    /// Block headers sit at the left margin and their rows are indented, so
    /// this needs no knowledge of which blocks exist.
    fn block_lines(block: &str) -> impl Iterator<Item = &'static str> + '_ {
        LP_STAGES
            .lines()
            .skip_while(move |l| l.trim_end() != block)
            .skip(1)
            .take_while(|l| l.starts_with(' '))
    }

    /// Whether the dump holds a block with this name.
    pub fn has_block(block: &str) -> bool {
        LP_STAGES.lines().any(|l| l.trim_end() == block)
    }

    /// Whether a block holds a row with this label.
    pub fn block_has(block: &str, label: &str) -> bool {
        block_lines(block).any(|l| l.split_whitespace().next() == Some(label))
    }

    /// One labelled row of a named block, as 32-bit integers.
    ///
    /// Needed where a dumped value does not fit a `Word16` -- the Q16 code
    /// gain, for instance.
    pub fn block_row_i32(block: &str, label: &str) -> Vec<i32> {
        for line in block_lines(block) {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("block {block:?} has no row {label:?}");
    }

    /// One labelled row of a named block, as integers.
    pub fn block_row(block: &str, label: &str) -> Vec<i16> {
        for line in block_lines(block) {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("block {block:?} has no row {label:?}");
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
