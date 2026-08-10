//! The AMR-NB spectral path, 3GPP TS 26.090 §5.2, in fixed point.
//!
//! Quantiser indices to per-subframe LP coefficients: LSF dequantisation,
//! spacing enforcement, LSF→LSP, LSP→LP, and interpolation across the four
//! subframes.
//!
//! # LSP is not ISP
//!
//! This is the module where the temptation to reuse the wideband code is
//! strongest and most wrong. Line Spectrum Pairs and Immittance Spectrum Pairs
//! are different representations, not the same one at different orders:
//!
//! - [`lsp_to_lp`] multiplies `F2(z)` by `(1 - z⁻¹)` — the reference subtracts
//!   `f2[i-1]`. Wideband multiplies by `(1 - z⁻²)`, subtracting `f2[i-2]`.
//! - Narrowband's final shift is 13; wideband's is 12.
//! - Wideband scales both polynomials by its last ISP, which is a predictor
//!   coefficient rather than a root. Narrowband has no such term.
//! - The cosine table has 65 entries against wideband's 129, indexed by the top
//!   *eight* bits rather than nine.
//!
//! Every one of those would compile, run, and produce speech-shaped output.

use super::decoder_tables::{
    COS_TABLE, DICO1_LSF_3, DICO2_LSF_3, DICO3_LSF_3, MEAN_LSF_3, MR515_3_LSF, MR795_1_LSF,
    PAST_RQ_INIT, PRED_FAC_3,
};
use crate::fixed_point::arith::{add, extract_l, mult, sub};
use crate::fixed_point::arith32::{l_add, l_msu, l_mult, l_sub};
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr, l_shr_r, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Order of the narrowband LP filter.
pub const M: usize = 10;

/// Coefficients per subframe, plus the leading 1.0.
pub const MP1: usize = M + 1;

/// Subframes per frame.
pub const NB_SUBFR: usize = 4;

/// Interpolated coefficients for a whole frame.
pub const AZ_SIZE: usize = NB_SUBFR * MP1;

/// Minimum spacing between adjacent LSFs, Q15 — 50 Hz at 8 kHz.
const LSF_GAP: Word16 = Word16(205);

/// Weight on the previous frame's LSFs during concealment, 0.9 in Q15.
const ALPHA: Word16 = Word16(29491);

/// The complement of [`ALPHA`].
const ONE_ALPHA: Word16 = Word16(3277);

/// Which codebook triple a mode uses for the 3-split quantiser.
///
/// Three groupings, not eight: most modes share one set, 4.75 and 5.15 swap the
/// third split for a smaller book, and 7.95 swaps the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Books {
    /// 5.90, 6.70, 7.40, 10.2 and the SID frame.
    Standard,
    /// 4.75 and 5.15 — a reduced third split, and the second index is doubled.
    Reduced,
    /// 7.95 — a wider first split.
    Wide,
}

impl Books {
    const fn for_mode(mode_index: u8) -> Self {
        match mode_index {
            0 | 1 => Self::Reduced,
            5 => Self::Wide,
            _ => Self::Standard,
        }
    }

    const fn first(self) -> &'static [i16] {
        match self {
            Self::Wide => &MR795_1_LSF,
            _ => &DICO1_LSF_3,
        }
    }

    const fn third(self) -> &'static [i16] {
        match self {
            Self::Reduced => &MR515_3_LSF,
            _ => &DICO3_LSF_3,
        }
    }
}

/// Carried spectral state: the MA predictor and the previous frame's LSFs.
#[derive(Debug, Clone)]
pub struct LsfDecoder {
    /// Previous quantised residual, the MA prediction's input.
    past_residual: [Word16; M],
    /// Previous decoded LSFs, for concealment.
    past_lsf: [Word16; M],
}

impl Default for LsfDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl LsfDecoder {
    /// A decoder in its reset state.
    ///
    /// The residual is seeded from the reference's `past_rq_init` rather than
    /// zeroed: a zeroed predictor makes the first frame decode as if the
    /// previous spectrum had been exactly the long-term mean, which it was not.
    #[must_use]
    pub fn new() -> Self {
        let mut past_residual = [Word16(0); M];
        for (slot, &v) in past_residual.iter_mut().zip(PAST_RQ_INIT.iter()) {
            *slot = Word16(v);
        }
        Self {
            past_residual,
            past_lsf: MEAN_LSF_3.map(Word16),
        }
    }

    /// Decode one frame's LSFs into the cosine domain.
    ///
    /// `indices` holds the three split indices. On an erasure the indices are
    /// ignored and the previous LSFs are used, pulled toward the long-term mean.
    ///
    /// # Panics
    ///
    /// If `indices` holds fewer than three entries on a good frame.
    pub fn decode(&mut self, mode_index: u8, indices: &[u16], bad_frame: bool) -> [Word16; M] {
        let mut ctx = DspContext::default();

        let mut lsf = if bad_frame {
            self.conceal(&mut ctx)
        } else {
            assert!(indices.len() >= 3, "the 3-split quantiser needs three indices");
            self.decode_good(&mut ctx, mode_index, indices)
        };

        // Quantisation can leave LSFs too close together or out of order, and
        // either makes the synthesis filter ring. Note this walks ALL ten
        // coefficients, where the wideband equivalent stops one short.
        reorder_lsf(&mut ctx, &mut lsf, LSF_GAP);
        self.past_lsf = lsf;

        lsf_to_lsp(&mut ctx, &lsf)
    }

    fn decode_good(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        indices: &[u16],
    ) -> [Word16; M] {
        let books = Books::for_mode(mode_index);
        let mut residual = [Word16(0); M];

        // First split: three coefficients.
        let base = usize::from(indices[0]) * 3;
        for (i, slot) in residual[0..3].iter_mut().enumerate() {
            *slot = Word16(books.first()[base + i]);
        }

        // Second split: three more. The reduced books address only every
        // second entry, so the index is doubled rather than the book halved.
        let index = if books == Books::Reduced {
            usize::from(indices[1]) * 2
        } else {
            usize::from(indices[1])
        };
        let base = index * 3;
        for (i, slot) in residual[3..6].iter_mut().enumerate() {
            *slot = Word16(DICO2_LSF_3[base + i]);
        }

        // Third split: four, so the index scales by four not three.
        let base = usize::from(indices[2]) * 4;
        for (i, slot) in residual[6..10].iter_mut().enumerate() {
            *slot = Word16(books.third()[base + i]);
        }

        // Undo the prediction. Unlike wideband's single scalar, narrowband has
        // a per-coefficient prediction factor.
        let mut lsf = [Word16(0); M];
        for i in 0..M {
            let predicted = mult(ctx, self.past_residual[i], Word16(PRED_FAC_3[i]));
            let anchor = add(ctx, Word16(MEAN_LSF_3[i]), predicted);
            lsf[i] = add(ctx, residual[i], anchor);
            self.past_residual[i] = residual[i];
        }
        lsf
    }

    /// Reconstruct LSFs for an erased frame.
    fn conceal(&mut self, ctx: &mut DspContext) -> [Word16; M] {
        let mut lsf = [Word16(0); M];
        for i in 0..M {
            let held = mult(ctx, self.past_lsf[i], ALPHA);
            let pulled = mult(ctx, Word16(MEAN_LSF_3[i]), ONE_ALPHA);
            lsf[i] = add(ctx, held, pulled);
        }
        // No transmitted residual, so estimate what one would have been.
        for i in 0..M {
            let predicted = mult(ctx, self.past_residual[i], Word16(PRED_FAC_3[i]));
            let anchor = add(ctx, Word16(MEAN_LSF_3[i]), predicted);
            self.past_residual[i] = sub(ctx, lsf[i], anchor);
        }
        lsf
    }
}

/// Force a minimum spacing between adjacent LSFs.
///
/// Walks forward raising values only, so ordering is restored as a side effect.
/// Covers all `M` coefficients — the wideband equivalent deliberately leaves its
/// last one alone, because there it is a predictor coefficient rather than a
/// line frequency.
pub fn reorder_lsf(ctx: &mut DspContext, lsf: &mut [Word16; M], min_dist: Word16) {
    let mut floor = min_dist;
    for slot in lsf.iter_mut() {
        if slot.0 < floor.0 {
            *slot = floor;
        }
        floor = add(ctx, *slot, min_dist);
    }
}

/// Convert LSFs (normalised frequencies) to LSPs (cosines).
///
/// Table-and-interpolate over 65 points, indexed by the top eight bits. The
/// wideband version uses 129 points and nine bits; they are not interchangeable.
///
/// # Panics
///
/// If any input is negative, which means it is not an LSF.
#[must_use]
pub fn lsf_to_lsp(ctx: &mut DspContext, lsf: &[Word16; M]) -> [Word16; M] {
    let mut lsp = [Word16(0); M];
    for (i, slot) in lsp.iter_mut().enumerate() {
        let ind = usize::try_from(shr(ctx, lsf[i], 8).0).expect("LSFs are non-negative");
        let offset = Word16(lsf[i].0 & 0x00ff);

        let step = sub(ctx, Word16(COS_TABLE[ind + 1]), Word16(COS_TABLE[ind]));
        let interp = l_mult(ctx, step, offset);
        let shifted = l_shr(ctx, interp, 9);
        *slot = add(ctx, Word16(COS_TABLE[ind]), extract_l(shifted));
    }
    lsp
}

/// Expand one interlaced root set into its polynomial, in Q23.
fn lsp_polynomial(ctx: &mut DspContext, lsp: &[Word16], f: &mut [Word32; 6]) {
    f[0] = l_mult(ctx, Word16(4096), Word16(2048));
    f[1] = l_msu(ctx, Word32(0), lsp[0], Word16(512));

    for i in 2..=5 {
        f[i] = f[i - 2];
        let q = lsp[(i - 1) * 2];
        for k in (2..=i).rev() {
            let (hi, lo) = l_extract(f[k - 1]);
            let term = l_shl(ctx, mpy_32_16(hi, lo, q), 1);
            // Note the order: add the two-back term first, then subtract.
            // Mathematically commutative, but the saturating operators are not.
            f[k] = l_add(ctx, f[k], f[k - 2]);
            f[k] = l_sub(ctx, f[k], term);
        }
        f[1] = l_msu(ctx, f[1], q, Word16(512));
    }
}

/// Convert LSPs to predictor coefficients, order 10.
///
/// **Not the wideband algorithm at a smaller order.** `F2(z)` is multiplied by
/// `(1 - z⁻¹)` here and `(1 - z⁻²)` there, the final shift is 13 rather than
/// 12, and there is no trailing-coefficient scaling at all.
#[must_use]
pub fn lsp_to_lp(ctx: &mut DspContext, lsp: &[Word16; M]) -> [Word16; MP1] {
    let mut f1 = [Word32(0); 6];
    let mut f2 = [Word32(0); 6];
    lsp_polynomial(ctx, &lsp[0..], &mut f1);
    lsp_polynomial(ctx, &lsp[1..], &mut f2);

    // F1 gains a root at z = -1 and F2 loses one at z = +1.
    for i in (1..=5).rev() {
        f1[i] = l_add(ctx, f1[i], f1[i - 1]);
        f2[i] = l_sub(ctx, f2[i], f2[i - 1]);
    }

    let mut a = [Word16(0); MP1];
    a[0] = Word16(4096);
    for i in 1..=5 {
        let j = M - i + 1;
        let sum = l_add(ctx, f1[i], f2[i]);
        a[i] = extract_l(l_shr_r(ctx, sum, 13));
        let diff = l_sub(ctx, f1[i], f2[i]);
        a[j] = extract_l(l_shr_r(ctx, diff, 13));
    }
    a
}

/// Interpolate between two frames' LSPs and convert each subframe to LP.
///
/// Weights are a uniform 1/4, 2/4, 3/4, 1 — unlike wideband's
/// `{0.45, 0.8, 0.96, 1.0}`, which is skewed toward the new frame because its
/// analysis window is. Narrowband's is not.
#[must_use]
pub fn interpolate_lsp(
    ctx: &mut DspContext,
    lsp_old: &[Word16; M],
    lsp_new: &[Word16; M],
) -> [Word16; AZ_SIZE] {
    let mut az = [Word16(0); AZ_SIZE];
    let mut lsp = [Word16(0); M];

    // Expressed as shifts rather than multiplies, exactly as the reference:
    // x - x/4 is 3/4 without a multiply, and the rounding differs from a
    // multiply by 24576.
    for i in 0..M {
        let quarter_new = shr(ctx, lsp_new[i], 2);
        let quarter_old = shr(ctx, lsp_old[i], 2);
        let three_quarter_old = sub(ctx, lsp_old[i], quarter_old);
        lsp[i] = add(ctx, quarter_new, three_quarter_old);
    }
    az[0..MP1].copy_from_slice(&lsp_to_lp(ctx, &lsp));

    for i in 0..M {
        let half_old = shr(ctx, lsp_old[i], 1);
        let half_new = shr(ctx, lsp_new[i], 1);
        lsp[i] = add(ctx, half_old, half_new);
    }
    az[MP1..2 * MP1].copy_from_slice(&lsp_to_lp(ctx, &lsp));

    for i in 0..M {
        let quarter_old = shr(ctx, lsp_old[i], 2);
        let quarter_new_drop = shr(ctx, lsp_new[i], 2);
        let three_quarter_new = sub(ctx, lsp_new[i], quarter_new_drop);
        lsp[i] = add(ctx, quarter_old, three_quarter_new);
    }
    az[2 * MP1..3 * MP1].copy_from_slice(&lsp_to_lp(ctx, &lsp));

    az[3 * MP1..].copy_from_slice(&lsp_to_lp(ctx, lsp_new));
    az
}

/// The decoder's reset-state LSPs.
#[must_use]
pub fn initial_lsp() -> [Word16; M] {
    super::decoder_tables::LSP_INIT.map(Word16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGES: &str = include_str!("../testdata/stages_nb.txt");

    fn block_rows(block: &str) -> impl Iterator<Item = &'static str> + '_ {
        STAGES
            .lines()
            .skip_while(move |l| l.trim_end() != block)
            .skip(1)
            .take_while(|l| l.starts_with(' '))
    }

    fn row(block: &str, label: &str) -> Vec<i16> {
        for line in block_rows(block) {
            let mut parts = line.split_whitespace();
            if parts.next() == Some(label) {
                return parts.map(|v| v.parse().expect("integer")).collect();
            }
        }
        panic!("block {block:?} has no row {label:?}");
    }

    fn has_row(block: &str, label: &str) -> bool {
        block_rows(block).any(|l| l.split_whitespace().next() == Some(label))
    }

    #[test]
    fn the_spectral_path_is_bit_exact_against_ts26073() {
        let mut checked = 0;

        // 12.2 kbit/s uses the 5-split quantiser, which is not implemented yet.
        for mode_index in 0..7u8 {
            let block = format!("nb{mode_index}");
            let mut dec = LsfDecoder::new();
            let mut lsp_old = initial_lsp();
            let mut ctx = DspContext::default();

            for f in 0.. {
                if !has_row(&block, &format!("prm{f}")) {
                    break;
                }
                let prm = row(&block, &format!("prm{f}"));
                let indices: Vec<u16> = prm[..3]
                    .iter()
                    .map(|&v| u16::try_from(v).expect("index"))
                    .collect();

                let lsp_new = dec.decode(mode_index, &indices, false);
                let want_lsp = row(&block, &format!("lsp{f}"));
                for (i, (&g, &w)) in lsp_new.iter().zip(want_lsp.iter()).enumerate() {
                    assert_eq!(
                        g.0, w,
                        "{block} frame {f}: lsp[{i}] = {} but the reference gives {w}",
                        g.0
                    );
                }

                let az = interpolate_lsp(&mut ctx, &lsp_old, &lsp_new);
                let want_az = row(&block, &format!("az{f}"));
                assert_eq!(want_az.len(), AZ_SIZE, "{block} frame {f}: az length");
                for (i, (&g, &w)) in az.iter().zip(want_az.iter()).enumerate() {
                    assert_eq!(
                        g.0, w,
                        "{block} frame {f}: a[{i}] = {} but the reference gives {w}",
                        g.0
                    );
                }

                lsp_old = lsp_new;
                checked += 1;
            }
        }

        assert!(checked >= 14, "only {checked} frames checked");
    }

    #[test]
    fn lsps_stay_ordered_and_the_filter_stays_stable() {
        // Ordered LSPs are exactly the minimum-phase condition; the spacing
        // floor is what guarantees it after quantisation.
        for mode_index in 0..7u8 {
            let block = format!("nb{mode_index}");
            let mut dec = LsfDecoder::new();
            for f in 0..3 {
                if !has_row(&block, &format!("prm{f}")) {
                    break;
                }
                let prm = row(&block, &format!("prm{f}"));
                let indices: Vec<u16> = prm[..3]
                    .iter()
                    .map(|&v| u16::try_from(v).expect("index"))
                    .collect();
                let lsp = dec.decode(mode_index, &indices, false);
                for i in 1..M {
                    assert!(
                        lsp[i].0 < lsp[i - 1].0,
                        "mode {mode_index} frame {f}: lsp[{i}] is not below lsp[{}]",
                        i - 1
                    );
                }
            }
        }
    }

    #[test]
    fn the_leading_coefficient_is_unity_in_q12() {
        let mut ctx = DspContext::default();
        let a = lsp_to_lp(&mut ctx, &initial_lsp());
        assert_eq!(a[0].0, 4096);
    }

    #[test]
    fn this_is_not_the_wideband_conversion() {
        // A guard against someone "unifying" the two later. Wideband's
        // conversion divides F2 by (1 - z^-2) and shifts by 12; running the
        // same LSPs through both must not agree.
        let mut ctx = DspContext::default();
        let lsp = initial_lsp();
        let nb = lsp_to_lp(&mut ctx, &lsp);

        let mut wide = [Word16(0); 11];
        let mut wb_input = [Word16(0); 10];
        wb_input.copy_from_slice(&lsp);
        crate::codecs::amr::wb::lp::isp_to_lp::isp_to_lp_order(&wb_input, &mut wide);

        assert_ne!(
            nb.map(|w| w.0),
            wide.map(|w| w.0),
            "the narrowband and wideband conversions agree, which means one is wrong"
        );
    }

    #[test]
    fn concealment_pulls_toward_the_long_term_mean() {
        // A sustained erasure must converge on the mean spectrum rather than
        // holding whatever was last heard.
        let mut dec = LsfDecoder::new();
        let indices = [10u16, 20, 30];
        for _ in 0..4 {
            dec.decode(4, &indices, false);
        }

        let mut previous = dec.decode(4, &indices, true);
        let mut first_gap = 0i32;
        for n in 0..12 {
            let next = dec.decode(4, &indices, true);
            let gap: i32 = (0..M)
                .map(|i| (i32::from(next[i].0) - i32::from(previous[i].0)).abs())
                .sum();
            if n == 0 {
                first_gap = gap;
            } else if n == 11 {
                assert!(gap <= first_gap, "erasure {n} moved {gap}, not below {first_gap}");
            }
            previous = next;
        }
    }
}
