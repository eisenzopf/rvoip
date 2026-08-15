//! ISP/ISF conversion and interpolation, 3GPP TS 26.190 §5.2.5–§5.2.6.
//!
//! Two representations of the same thing. ISPs are the cosines of the line
//! frequencies, which is the form the polynomial arithmetic in
//! [`super::isp_to_lp`] wants. ISFs are the frequencies themselves, which is
//! the form that quantises well — they are ordered, bounded, and their
//! quantisation error stays local instead of smearing across the spectrum.
//!
//! So the codec quantises in the ISF domain and computes in the ISP domain,
//! and converts between them once per frame in each direction.
//!
//! # These tables are the transform
//!
//! Both directions interpolate a 129-entry cosine table rather than evaluating
//! a real trigonometric function. That is not an approximation the
//! implementation is free to improve on: a more accurate cosine produces
//! different bits and fails conformance. The round trip is correspondingly
//! *not* the identity, which is why the tests check each direction against the
//! reference separately rather than only checking that they compose.

use super::autocorr::LP_ORDER;
use super::isf_tables::{ACOS_SLOPE, COS_TABLE};
use super::isp_to_lp::isp_to_lp;
use crate::fixed_point::arith::{add, extract_l, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult};
use crate::fixed_point::shift::{l_shl, l_shr, shl, shr};
use crate::fixed_point::types::{DspContext, Word16};

/// Subframes per frame.
pub const NB_SUBFR: usize = 4;

/// Interpolation weights for the new frame's ISPs across the four subframes.
///
/// From the reference's `interpol_frac`: 0.45, 0.8, 0.96, 1.0 in Q15. Not
/// uniform quarters — the weighting is pushed hard toward the new frame,
/// because the analysis window that produced it is itself concentrated at the
/// end of the frame, so the "new" ISPs already describe the region the first
/// subframe sits in.
pub const INTERPOL_FRAC: [Word16; NB_SUBFR] =
    [Word16(14746), Word16(26214), Word16(31457), Word16(32767)];

/// Convert ISPs (Q15 cosines) to ISFs (Q15 normalised frequencies).
///
/// Output is normalised to `0.0..=0.5`, and the last coefficient is halved
/// because it spans twice the range of the others.
#[must_use]
pub fn isp_to_isf(isp: &[Word16; LP_ORDER]) -> [Word16; LP_ORDER] {
    let mut ctx = DspContext::default();
    let mut isf = [Word16(0); LP_ORDER];

    // Walk downwards. ISPs are descending, so the table index only ever moves
    // one way, and each search resumes where the last one stopped.
    let mut ind = 127usize;
    for i in (0..LP_ORDER).rev() {
        if i >= LP_ORDER - 2 {
            ind = 127;
        }
        // Find the table entry just above this ISP.
        while Word16(COS_TABLE[ind]).0 < isp[i].0 {
            ind -= 1;
        }

        // acos(isp) = ind*128 + (isp - table[ind]) * slope[ind] / 2048
        let delta = sub(&mut ctx, isp[i], Word16(COS_TABLE[ind]));
        let interp = l_mult(&mut ctx, delta, Word16(ACOS_SLOPE[ind]));
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let base = shl(&mut ctx, Word16(ind as i16), 7);
        let scaled = l_shl(&mut ctx, interp, 4);
        let frac = round(&mut ctx, scaled);
        isf[i] = add(&mut ctx, frac, base);
    }

    // The last ISF covers twice the range of the others; halve it so the whole
    // vector shares one scale for quantisation.
    isf[LP_ORDER - 1] = shr(&mut ctx, isf[LP_ORDER - 1], 1);
    isf
}

/// Convert ISFs (Q15 normalised frequencies) back to ISPs (Q15 cosines).
///
/// The inverse of [`isp_to_isf`] up to table interpolation, which is not exact
/// in either direction.
///
/// # Panics
///
/// If any input is negative, which means it is not an ISF.
/// Convert ISFs to ISPs in place, at any order.
///
/// The high band uses order 20, so this cannot be fixed at [`LP_ORDER`].
/// Otherwise identical to [`isf_to_isp`].
///
/// # Panics
///
/// If any value is negative, which means it is not an ISF.
pub fn isf_to_isp_in_place(ctx: &mut DspContext, isf: &mut [Word16]) {
    let m = isf.len();
    // Undo the halving applied to the last coefficient, which spans twice the
    // range of the others.
    isf[m - 1] = shl(ctx, isf[m - 1], 1);

    for slot in isf.iter_mut() {
        let ind =
            usize::try_from(shr(ctx, *slot, 7).0).expect("ISFs are non-negative by construction");
        let offset = Word16(slot.0 & 0x007f);

        let step = sub(ctx, Word16(COS_TABLE[ind + 1]), Word16(COS_TABLE[ind]));
        let interp = l_mult(ctx, step, offset);
        let shifted = l_shr(ctx, interp, 8);
        *slot = add(ctx, Word16(COS_TABLE[ind]), extract_l(shifted));
    }
}

/// Convert ISFs (Q15 normalised frequencies) back to ISPs (Q15 cosines).
///
/// The inverse of [`isp_to_isf`] up to table interpolation, which is not exact
/// in either direction.
///
/// # Panics
///
/// If any input is negative, which means it is not an ISF.
#[must_use]
pub fn isf_to_isp(isf: &[Word16; LP_ORDER]) -> [Word16; LP_ORDER] {
    let mut ctx = DspContext::default();
    let mut isp = [Word16(0); LP_ORDER];

    isp[..LP_ORDER - 1].copy_from_slice(&isf[..LP_ORDER - 1]);
    // Undo the halving [`isp_to_isf`] applied to the last coefficient.
    isp[LP_ORDER - 1] = shl(&mut ctx, isf[LP_ORDER - 1], 1);

    for slot in &mut isp {
        // High bits select the interval, low seven interpolate within it.
        // ISFs are normalised frequencies and so never negative; a negative
        // here would mean the caller passed ISPs, which would index the cosine
        // table out of bounds a line later anyway.
        let ind = usize::try_from(shr(&mut ctx, *slot, 7).0)
            .expect("ISFs are non-negative by construction");
        let offset = Word16(slot.0 & 0x007f);

        let step = sub(&mut ctx, Word16(COS_TABLE[ind + 1]), Word16(COS_TABLE[ind]));
        let interp = l_mult(&mut ctx, step, offset);
        let shifted = l_shr(&mut ctx, interp, 8);
        *slot = add(&mut ctx, Word16(COS_TABLE[ind]), extract_l(shifted));
    }

    isp
}

/// Interpolate between two frames' ISPs and convert each subframe to LP
/// coefficients.
///
/// Returns four coefficient sets, one per subframe, each `a[0..=16]` in Q12.
/// The fourth is the new frame's ISPs unmodified, since its weight is 1.0.
///
/// Interpolating in the ISP domain rather than the coefficient domain is what
/// keeps every intermediate filter stable: ISPs stay ordered under a convex
/// combination, and ordered ISPs are exactly the condition for a minimum-phase
/// synthesis filter. Interpolating coefficients directly gives no such
/// guarantee and can produce an unstable filter from two stable endpoints.
#[must_use]
pub fn interpolate_isp(
    isp_old: &[Word16; LP_ORDER],
    isp_new: &[Word16; LP_ORDER],
) -> [[Word16; LP_ORDER + 1]; NB_SUBFR] {
    let mut ctx = DspContext::default();
    let mut az = [[Word16(0); LP_ORDER + 1]; NB_SUBFR];

    for (k, frame) in az.iter_mut().enumerate().take(NB_SUBFR - 1) {
        let fac_new = INTERPOL_FRAC[k];
        let complement = sub(&mut ctx, Word16(32767), fac_new);
        let fac_old = add(&mut ctx, complement, Word16(1));

        let mut isp = [Word16(0); LP_ORDER];
        for (i, slot) in isp.iter_mut().enumerate() {
            let acc = l_mult(&mut ctx, isp_old[i], fac_old);
            let acc = l_mac(&mut ctx, acc, isp_new[i], fac_new);
            *slot = round(&mut ctx, acc);
        }
        *frame = isp_to_lp(&isp);
    }

    // The last subframe's weight is exactly 1.0, so no blend is needed.
    az[NB_SUBFR - 1] = isp_to_lp(isp_new);
    az
}

#[cfg(test)]
mod tests {
    use super::super::isp_to_lp::tests_support::{case_count, row};
    use super::*;

    fn vector(case: usize, label: &str) -> [Word16; LP_ORDER] {
        let values = row(case, label);
        assert_eq!(values.len(), LP_ORDER, "case {case}: {label} length");
        let mut out = [Word16(0); LP_ORDER];
        for (slot, &v) in out.iter_mut().zip(values.iter()) {
            *slot = Word16(v);
        }
        out
    }

    #[test]
    fn isp_to_isf_is_bit_exact_against_ts26173() {
        for case in 0..case_count() {
            let got = isp_to_isf(&vector(case, "isp"));
            let want = row(case, "isf");
            for i in 0..LP_ORDER {
                assert_eq!(
                    got[i].0, want[i],
                    "case {case}: isf[{i}] = {} but the reference gives {}",
                    got[i].0, want[i]
                );
            }
        }
    }

    #[test]
    fn isf_to_isp_is_bit_exact_against_ts26173() {
        for case in 0..case_count() {
            let got = isf_to_isp(&vector(case, "isf"));
            let want = row(case, "isp_rt");
            for i in 0..LP_ORDER {
                assert_eq!(
                    got[i].0, want[i],
                    "case {case}: isp[{i}] = {} but the reference gives {}",
                    got[i].0, want[i]
                );
            }
        }
    }

    #[test]
    fn interpolation_is_bit_exact_against_ts26173() {
        for case in 0..case_count() {
            // The oracle interpolates from the reset-state ISPs, which is what
            // the codec itself starts from on the first frame.
            let old = reset_state_isp();
            let got = interpolate_isp(&old, &vector(case, "isp"));
            for (k, subframe) in got.iter().enumerate() {
                let want = row(case, &format!("az_int{k}"));
                assert_eq!(want.len(), LP_ORDER + 1, "case {case}: az_int{k} length");
                for i in 0..=LP_ORDER {
                    assert_eq!(
                        subframe[i].0, want[i],
                        "case {case} subframe {k}: a[{i}] = {} but the reference gives {}",
                        subframe[i].0, want[i]
                    );
                }
            }
        }
    }

    /// The codec's reset-state ISPs: evenly spaced line frequencies, which is
    /// a flat spectrum. The oracle builds these the same way.
    fn reset_state_isp() -> [Word16; LP_ORDER] {
        let mut isp = [Word16(0); LP_ORDER];
        for (i, slot) in isp.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let v = (f64::cos(std::f64::consts::PI * (i + 1) as f64 / (LP_ORDER + 1) as f64)
                * 32767.0) as i16;
            *slot = Word16(v);
        }
        isp
    }

    #[test]
    fn isfs_are_ordered_and_within_range() {
        // Ordering is the property that makes ISFs quantisable and the filter
        // stable; if a conversion breaks it, everything downstream is wrong.
        for case in 0..case_count() {
            let isf = isp_to_isf(&vector(case, "isp"));
            for i in 1..LP_ORDER - 1 {
                assert!(
                    isf[i].0 > isf[i - 1].0,
                    "case {case}: isf[{i}] = {} does not exceed isf[{}] = {}",
                    isf[i].0,
                    i - 1,
                    isf[i - 1].0
                );
            }
            assert!(isf[0].0 >= 0, "case {case}: first ISF is negative");
        }
    }

    #[test]
    fn the_last_subframe_skips_the_blend() {
        // Weight 1.0 means the fourth subframe must equal a direct conversion.
        for case in 0..case_count() {
            let new = vector(case, "isp");
            let az = interpolate_isp(&reset_state_isp(), &new);
            assert_eq!(az[NB_SUBFR - 1], isp_to_lp(&new), "case {case}");
        }
    }

    #[test]
    fn interpolation_moves_monotonically_toward_the_new_frame() {
        // The weights ascend, so each subframe should sit closer to the new
        // frame's coefficients than the one before it.
        for case in 0..case_count() {
            let old = reset_state_isp();
            let new = vector(case, "isp");
            let az = interpolate_isp(&old, &new);
            let target = isp_to_lp(&new);

            let distance = |a: &[Word16; LP_ORDER + 1]| -> i64 {
                a.iter()
                    .zip(target.iter())
                    .map(|(x, y)| i64::from(x.0 - y.0).pow(2))
                    .sum()
            };

            for k in 1..NB_SUBFR {
                assert!(
                    distance(&az[k]) <= distance(&az[k - 1]),
                    "case {case}: subframe {k} is further from the new frame than {}",
                    k - 1
                );
            }
        }
    }
}
