//! The AMR-NB spectral path, 3GPP TS 26.090 §5.2, in fixed point.
//!
//! Quantiser indices to per-subframe LP coefficients: LSF dequantisation,
//! spacing enforcement, LSF→LSP, LSP→LP, and interpolation across the four
//! subframes.
//!
//! Implements TS 26.073's `D_plsf_3`, `D_plsf_5`, `D_plsf_reset`,
//! `Init_D_plsf_3`, `Reorder_lsf`, `Lsf_lsp`, `Lsp_Az`, `Int_lpc_1to3` and
//! `Int_lpc_1and3`.
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
//!
//! # Two quantisers, not one parameterised quantiser
//!
//! Seven of the eight rates dequantise one LSF set per frame from three split
//! indices ([`LsfDecoder::decode`], TS 26.073 `D_plsf_3`). 12.2 kbit/s
//! dequantises *two* sets from five indices ([`LsfDecoder::decode_pair`],
//! `D_plsf_5`) and interpolates through the mid-frame set
//! ([`interpolate_lsp_mid`], `Int_lpc_1and3`) instead of straight across the
//! frame ([`interpolate_lsp`], `Int_lpc_1to3`).
//!
//! They share the carried state and nothing else. Their long-term means, their
//! predictor coefficients, their concealment weights and their codebook strides
//! are all different, and every one of those differences is small enough to
//! look like a refactoring opportunity. It is not: see [`MEAN_LSF_3`] against
//! [`MEAN_LSF_5`], and `ALPHA_3` against `ALPHA_5` below.
//!
//! Validated against the `plsf5` section of `testdata/nb_stages.txt` (five-split
//! path, including an erased frame) and the `nb0`..`nb6` blocks of
//! `testdata/stages_nb.txt` (three-split path).

use super::decoder_tables::{
    COS_TABLE, DICO1_LSF_3, DICO1_LSF_5, DICO2_LSF_3, DICO2_LSF_5, DICO3_LSF_3, DICO3_LSF_5,
    DICO4_LSF_5, DICO5_LSF_5, MEAN_LSF_3, MEAN_LSF_5, MR515_3_LSF, MR795_1_LSF, PAST_RQ_INIT,
    PRED_FAC_3,
};
use crate::fixed_point::arith::{add, extract_l, mult, negate, sub};
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
///
/// The 3-split quantiser's. 12.2 kbit/s uses [`ALPHA_5`], which is a different
/// number; the two must not be unified.
const ALPHA_3: Word16 = Word16(29491);

/// The complement of [`ALPHA_3`].
const ONE_ALPHA_3: Word16 = Word16(3277);

/// Weight on the previous frame's LSFs during concealment at 12.2 kbit/s,
/// 0.95 in Q15.
///
/// A slower leak toward the mean than [`ALPHA_3`]: the 5-split quantiser
/// resolves the spectrum finely enough that holding it longer is worth more
/// than converging faster.
const ALPHA_5: Word16 = Word16(31128);

/// The complement of [`ALPHA_5`].
///
/// `ALPHA_5 + ONE_ALPHA_5` is 32767, not 32768 — 0.95 and 0.05 were each
/// rounded into Q15 independently. The concealed spectrum therefore contracts
/// by one part in 32768 per erased frame. Do not "correct" it.
const ONE_ALPHA_5: Word16 = Word16(1639);

/// MA predictor coefficient at 12.2 kbit/s, 0.65 in Q15.
///
/// One scalar for all ten coefficients, where the 3-split quantiser has a
/// per-coefficient [`PRED_FAC_3`].
const LSP_PRED_FAC_MR122: Word16 = Word16(21299);

/// The 5-split codebooks, each paired with the first of the two LSF
/// coefficients it covers.
///
/// Entries are stride 4 and interleaved `[set1[a], set1[a+1], set2[a],
/// set2[a+1]]`: a single index quantises the same coefficient pair in *both*
/// LSF sets of the frame. That is what makes this a matrix quantiser, and it is
/// why the MA prediction below is applied once per frame rather than once per
/// set.
const SPLITS_5: [(&[i16], usize); 5] = [
    (&DICO1_LSF_5, 0),
    (&DICO2_LSF_5, 2),
    (&DICO3_LSF_5, 4),
    (&DICO4_LSF_5, 6),
    (&DICO5_LSF_5, 8),
];

/// The split whose index spends its least significant bit on a sign.
///
/// Nine transmitted bits address a 256-entry table: the low bit negates the
/// whole quadruple instead of selecting among 512 stored entries. The stored
/// table is *not* sign-symmetric — the symmetry is created here, at decode
/// time.
const SIGNED_SPLIT_5: usize = 2;

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
///
/// One struct for both quantisers, deliberately. A stream that switches rate
/// mid-call carries this memory across the switch, and the two quantisers then
/// interpret the very same numbers against different means and predictors.
/// Splitting the state per variant would silently reset the predictor on every
/// rate change.
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
    /// A decoder seeded from the reference's `past_rq_init`, as `Init_D_plsf_3`
    /// leaves it.
    ///
    /// The residual is seeded rather than zeroed: a zeroed predictor makes the
    /// first frame decode as if the previous spectrum had been exactly the
    /// long-term mean, which it was not.
    ///
    /// This is the *comfort-noise* initialiser, and only approximately that:
    /// `Init_D_plsf_3` writes the residual and leaves the previous LSFs
    /// untouched, where this fills them with [`MEAN_LSF_3`]. A speech decoder
    /// starts from [`LsfDecoder::at_reset`] instead — `Decoder_amr_reset` calls
    /// `D_plsf_reset`, and `Init_D_plsf_3` appears only on the DTX path, where
    /// the SID frame's first parameter chooses among eight seed vectors.
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

    /// A decoder in the state `D_plsf_reset` leaves behind: no prediction
    /// memory, and the *5-split* long-term mean as the previous frame's LSFs.
    ///
    /// The mean is [`MEAN_LSF_5`] for every rate, not only 12.2. The reference's
    /// shared reset lives in a file that includes the 12.2 codebook table, so
    /// the seven 3-split rates reset to a mean their own quantiser never uses
    /// again. It shows through on the first frame of a stream — as the
    /// concealed spectrum if that frame is erased, and as the "previous frame's
    /// LSFs" that the codebook-gain averaging interpolates against in any case.
    ///
    /// This is also the state to return to after every non-speech frame, not
    /// only at start-up: the reference resets the LSF decoder unconditionally
    /// whenever the receiver hands it anything other than a speech frame.
    #[must_use]
    pub fn at_reset() -> Self {
        Self {
            past_residual: [Word16(0); M],
            past_lsf: MEAN_LSF_5.map(Word16),
        }
    }

    /// Decode one frame's LSFs into the cosine domain, three-split quantiser.
    ///
    /// `indices` holds the three split indices, Q0. On an erasure the indices
    /// are ignored and the previous LSFs are used, pulled toward the long-term
    /// mean. Returns one LSP set, Q15, for the 4th subframe.
    ///
    /// # Panics
    ///
    /// If `indices` holds fewer than three entries on a good frame, or if
    /// called for 12.2 kbit/s — that rate has its own quantiser,
    /// [`LsfDecoder::decode_pair`]. The 3-split books would decode a 12.2 frame
    /// into a plausible spectrum from indices that mean something else
    /// entirely, so this is worth a panic rather than a silent wrong answer.
    pub fn decode(&mut self, mode_index: u8, indices: &[u16], bad_frame: bool) -> [Word16; M] {
        assert!(mode_index < 7, "12.2 kbit/s decodes through decode_pair");
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

    /// The LSFs this decoder last produced, Q15.
    ///
    /// The frame assembly needs them twice after the spectral path is done:
    /// `Int_lsf` interpolates the previous frame's against these for the
    /// codebook-gain smoother, and the running LSF average is updated from
    /// them. Both want LSFs rather than the LSPs [`decode`](Self::decode)
    /// returns, and recomputing them from the LSPs would not round-trip.
    #[must_use]
    pub const fn last_lsf(&self) -> &[Word16; M] {
        &self.past_lsf
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
            let held = mult(ctx, self.past_lsf[i], ALPHA_3);
            let pulled = mult(ctx, Word16(MEAN_LSF_3[i]), ONE_ALPHA_3);
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

    /// Decode one 12.2 kbit/s frame's two LSF sets into the cosine domain.
    ///
    /// `indices` holds the five split indices, Q0, of widths 7, 8, 9, 8 and 6
    /// bits. Returns `(mid, new)`, both Q15: the LSPs quantised at the 2nd and
    /// at the 4th subframe. Feed them to [`interpolate_lsp_mid`] in that order,
    /// and carry `new` — not `mid` — into the next frame as `lsp_old`.
    ///
    /// On an erasure the indices are ignored, both sets come out identical, and
    /// the prediction memory is rebuilt from the concealed LSFs so the next good
    /// frame is decoded against something rather than against zero.
    ///
    /// # Panics
    ///
    /// If `indices` holds fewer than five entries on a good frame, or if an
    /// index overruns its codebook.
    pub fn decode_pair(
        &mut self,
        indices: &[u16],
        bad_frame: bool,
    ) -> ([Word16; M], [Word16; M]) {
        let mut ctx = DspContext::default();

        let (mut lsf_mid, mut lsf_new) = if bad_frame {
            self.conceal_pair(&mut ctx)
        } else {
            assert!(indices.len() >= 5, "the 5-split quantiser needs five indices");
            self.decode_pair_good(&mut ctx, indices)
        };

        // Both sets are spaced independently. On an erasure they start out
        // identical, so this is redundant there — but only there, and the cost
        // of keeping the two paths uniform is ten comparisons.
        reorder_lsf(&mut ctx, &mut lsf_mid, LSF_GAP);
        reorder_lsf(&mut ctx, &mut lsf_new, LSF_GAP);

        // The concealment memory is the 4th-subframe set, and it is the
        // *spaced* one — where the prediction memory written above took the
        // raw, unspaced residual. Two different vectors, deliberately.
        self.past_lsf = lsf_new;

        (lsf_to_lsp(&mut ctx, &lsf_mid), lsf_to_lsp(&mut ctx, &lsf_new))
    }

    /// Dequantise both LSF sets from five received indices.
    fn decode_pair_good(
        &mut self,
        ctx: &mut DspContext,
        indices: &[u16],
    ) -> ([Word16; M], [Word16; M]) {
        let mut residual_mid = [Word16(0); M];
        let mut residual_new = [Word16(0); M];

        for (split, &(book, first)) in SPLITS_5.iter().enumerate() {
            let (index, flip) = if split == SIGNED_SPLIT_5 {
                // Plain shift and mask, as in the reference: the index is a
                // non-negative field, so there is nothing for the arithmetic
                // operators to do differently.
                (usize::from(indices[split] >> 1), indices[split] & 1 != 0)
            } else {
                (usize::from(indices[split]), false)
            };

            // A plain multiply where the reference writes a saturating shift.
            // The field widths bound every index below its codebook's size, so
            // the shift cannot clamp; the assert is what keeps that true.
            let base = index * 4;
            assert!(
                base + 4 <= book.len(),
                "5-split index {index} overruns codebook {split}"
            );

            for k in 0..2 {
                let (mid, new) = (Word16(book[base + k]), Word16(book[base + 2 + k]));
                residual_mid[first + k] = if flip { negate(ctx, mid) } else { mid };
                residual_new[first + k] = if flip { negate(ctx, new) } else { new };
            }
        }

        let mut lsf_mid = [Word16(0); M];
        let mut lsf_new = [Word16(0); M];
        for i in 0..M {
            // One prediction, both sets: the MA predictor runs once per frame,
            // not once per LSF set. Reading it here and overwriting it two
            // lines down is the whole recurrence — hoisting the write out of
            // the loop would feed this frame's residual into this frame's
            // prediction.
            let predicted = mult(ctx, self.past_residual[i], LSP_PRED_FAC_MR122);
            let anchor = add(ctx, Word16(MEAN_LSF_5[i]), predicted);
            lsf_mid[i] = add(ctx, residual_mid[i], anchor);
            lsf_new[i] = add(ctx, residual_new[i], anchor);
            // The memory is the 4th-subframe set's residual. The mid-frame one
            // is never remembered.
            self.past_residual[i] = residual_new[i];
        }
        (lsf_mid, lsf_new)
    }

    /// Reconstruct both LSF sets for an erased 12.2 kbit/s frame.
    fn conceal_pair(&mut self, ctx: &mut DspContext) -> ([Word16; M], [Word16; M]) {
        let mut lsf = [Word16(0); M];
        for i in 0..M {
            let held = mult(ctx, self.past_lsf[i], ALPHA_5);
            let pulled = mult(ctx, Word16(MEAN_LSF_5[i]), ONE_ALPHA_5);
            lsf[i] = add(ctx, held, pulled);
        }
        // A second pass, not a continuation of the first: the residual estimate
        // for coefficient i must see the concealed LSF, and in the reference
        // that is guaranteed by the loop boundary rather than by the data
        // dependence (which happens to be per-coefficient anyway).
        for i in 0..M {
            let predicted = mult(ctx, self.past_residual[i], LSP_PRED_FAC_MR122);
            let anchor = add(ctx, Word16(MEAN_LSF_5[i]), predicted);
            self.past_residual[i] = sub(ctx, lsf[i], anchor);
        }
        // The two sets leave here identical; the spacing pass in the caller
        // cannot separate them either. A 12.2 erasure holds one spectrum for
        // the whole frame.
        (lsf, lsf)
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

/// Interpolate across a frame that carries a mid-frame LSP set, 12.2 kbit/s.
///
/// `lsp_old` is the previous frame's 4th-subframe set, `lsp_mid` and `lsp_new`
/// this frame's 2nd- and 4th-subframe sets, all Q15. Returns `AZ_SIZE`
/// coefficients, Q12, four `MP1`-long blocks with 4096 leading each.
///
/// Weights: `½old + ½mid`, `mid`, `½mid + ½new`, `new`. Only two of the four
/// subframes are interpolated at all — the other two use a transmitted set
/// unmodified. That is what 12.2's 38 bits of LSF payload buy over 10.2's 26.
///
/// Not [`interpolate_lsp`] with an extra argument. That one weights `¾old+¼new`
/// / `½+½` / `¼old+¾new` / `new` across the whole frame; this one halves twice
/// about the mid-frame set. Given `lsp_mid == lsp_new` the two still disagree,
/// because the first subframe is `½old + ½new` here and `¾old + ¼new` there.
#[must_use]
pub fn interpolate_lsp_mid(
    ctx: &mut DspContext,
    lsp_old: &[Word16; M],
    lsp_mid: &[Word16; M],
    lsp_new: &[Word16; M],
) -> [Word16; AZ_SIZE] {
    let mut az = [Word16(0); AZ_SIZE];
    let mut lsp = [Word16(0); M];

    // Halving by an arithmetic shift, exactly as the reference: `shr` floors,
    // so this is biased low by up to one LSB against a rounded mean. That bias
    // is part of the bit stream's definition, not an artefact to clean up.
    for i in 0..M {
        let half_mid = shr(ctx, lsp_mid[i], 1);
        let half_old = shr(ctx, lsp_old[i], 1);
        lsp[i] = add(ctx, half_mid, half_old);
    }
    az[0..MP1].copy_from_slice(&lsp_to_lp(ctx, &lsp));

    az[MP1..2 * MP1].copy_from_slice(&lsp_to_lp(ctx, lsp_mid));

    for i in 0..M {
        let half_mid = shr(ctx, lsp_mid[i], 1);
        let half_new = shr(ctx, lsp_new[i], 1);
        lsp[i] = add(ctx, half_mid, half_new);
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
    use crate::codecs::amr::nb::vectors;
    use std::collections::HashSet;

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

    /// Compare one decoded vector against the fixture's, element by element.
    fn agrees(what: &str, frame: usize, got: &[Word16], want: &[i16]) {
        assert_eq!(got.len(), want.len(), "plsf5 frame {frame}: {what} length");
        for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
            assert_eq!(
                g.0, w,
                "plsf5 frame {frame}: {what}[{i}] = {} but the reference gives {w}",
                g.0
            );
        }
    }

    /// Replay the `plsf5` section end to end and return how many frames it
    /// compared.
    ///
    /// A replay rather than eight isolated cases: the oracle resets the decoder
    /// once, seeds `lsp_old` with the flat-spectrum init vector and replaces it
    /// with the 4th-subframe set after each frame. A prediction memory updated
    /// from the wrong vector, or an `lsp_old` taken from the mid-frame set,
    /// therefore shows up in every frame after the first rather than nowhere.
    fn replay_five_split() -> usize {
        let rows = vectors::rows("plsf5");
        assert_eq!(rows[0].label, "seed", "plsf5 starts with the oracle's seed");

        let groups = &rows[1..];
        assert_eq!(groups.len() % 4, 0, "plsf5 rows come in groups of four");

        let mut dec = LsfDecoder::at_reset();
        let mut lsp_old = initial_lsp();
        let mut ctx = DspContext::default();
        let mut erased = Vec::new();

        for (f, group) in groups.chunks_exact(4).enumerate() {
            assert_eq!(group[0].label, "frame", "plsf5 group {f}: no frame row");
            assert_eq!(group[1].label, "lsp1", "plsf5 group {f}: no lsp1 row");
            assert_eq!(group[2].label, "lsp2", "plsf5 group {f}: no lsp2 row");
            assert_eq!(group[3].label, "az", "plsf5 group {f}: no az row");

            let head = group[0].ints();
            assert_eq!(head.len(), 6, "plsf5 frame {f}: bfi and five indices");
            let bad = head[0] != 0;
            if bad {
                erased.push(f);
            }
            let indices: Vec<u16> = head[1..]
                .iter()
                .map(|&v| u16::try_from(v).expect("index is a non-negative field"))
                .collect();

            let (lsp_mid, lsp_new) = dec.decode_pair(&indices, bad);
            agrees("lsp1", f, &lsp_mid, &group[1].i16s());
            agrees("lsp2", f, &lsp_new, &group[2].i16s());

            let az = interpolate_lsp_mid(&mut ctx, &lsp_old, &lsp_mid, &lsp_new);
            let want_az = group[3].i16s();
            assert_eq!(want_az.len(), AZ_SIZE, "plsf5 frame {f}: az length");
            agrees("az", f, &az, &want_az);

            // The 4th-subframe set, never the mid-frame one.
            lsp_old = lsp_new;
        }

        // The concealment path and its distinct state update are only covered
        // because the oracle marked one frame bad. If a regenerated fixture
        // ever drops it, this test would quietly stop testing half the module.
        assert_eq!(erased, vec![6], "plsf5 covers exactly one erased frame");

        groups.len() / 4
    }

    #[test]
    fn the_five_split_quantiser_is_bit_exact_against_ts26073() {
        assert_eq!(replay_five_split(), 8, "plsf5 carries eight frames");
    }

    #[test]
    fn the_spectral_path_is_bit_exact_against_ts26073() {
        let mut checked = 0;

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

        assert!(checked >= 14, "only {checked} three-split frames checked");

        // 12.2 kbit/s decodes two LSF sets per frame and interpolates through
        // the mid-frame one, so `amrnb_oracle.c` emits no lsp/az rows in the
        // `nb7` block — the 3-split call it makes for every other mode has no
        // counterpart there. The eighth rate's evidence is the `plsf5` section
        // of the other fixture, replayed here so this test still spans all
        // eight rates.
        assert!(!has_row("nb7", "lsp0"), "nb7 now carries its own spectral rows");
        let five_split = replay_five_split();
        assert_eq!(five_split, 8, "plsf5 carries eight frames");
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
                ordered(mode_index, f, "lsp", &lsp);
            }
        }

        // 12.2 kbit/s, from the same real bit streams: the `nb7` block carries
        // no decoded spectrum, but its `prm` rows are genuine 5-split indices
        // and both of the sets they produce must be ordered.
        let mut dec = LsfDecoder::at_reset();
        for f in 0..3 {
            if !has_row("nb7", &format!("prm{f}")) {
                break;
            }
            let prm = row("nb7", &format!("prm{f}"));
            let indices: Vec<u16> = prm[..5]
                .iter()
                .map(|&v| u16::try_from(v).expect("index"))
                .collect();
            let (mid, new) = dec.decode_pair(&indices, false);
            ordered(7, f, "mid", &mid);
            ordered(7, f, "new", &new);
        }
    }

    /// LSPs descend strictly — exactly the minimum-phase condition.
    fn ordered(mode_index: u8, frame: usize, what: &str, lsp: &[Word16; M]) {
        for i in 1..M {
            assert!(
                lsp[i].0 < lsp[i - 1].0,
                "mode {mode_index} frame {frame}: {what}[{i}] is not below {what}[{}]",
                i - 1
            );
        }
    }

    #[test]
    fn the_five_splits_cover_every_coefficient_exactly_once() {
        // Ten coefficients across five stride-4 books, two each. A book paired
        // with the wrong offset would still decode, and the fixture would catch
        // it — but only for the eight index combinations the oracle happened to
        // draw. This holds for all of them.
        let mut covered = [0u8; M];
        let sizes = [128, 256, 256, 256, 64];
        for (split, (&(book, first), entries)) in SPLITS_5.iter().zip(sizes).enumerate() {
            assert_eq!(
                book.len(),
                entries * 4,
                "split {split} is not {entries} stride-4 entries"
            );
            covered[first] += 1;
            covered[first + 1] += 1;
        }
        assert_eq!(covered, [1u8; M], "the five splits are not a partition");
    }

    #[test]
    fn the_third_splits_sign_bit_buys_a_second_codebook() {
        // The stored table is not sign-symmetric: the ninth transmitted bit
        // negates the quadruple, which turns 256 entries into 512 distinct
        // ones. If the table already contained each entry's negation, half the
        // bit would be wasted — and a reader who assumed symmetry and dropped
        // the negation would still pass a fixture that never exercised it.
        let mut seen = HashSet::new();
        for entry in DICO3_LSF_5.chunks_exact(4) {
            let quad = [entry[0], entry[1], entry[2], entry[3]];
            assert!(
                seen.insert(quad),
                "dico3 entry {quad:?} repeats, or negates an earlier one"
            );
            let flipped = quad.map(|v| -v);
            assert!(seen.insert(flipped), "dico3 already contains {flipped:?}");
        }
        assert_eq!(seen.len(), 512, "the effective codebook is not 512 wide");
    }

    #[test]
    fn the_two_quantisers_share_no_constants() {
        // Every one of these differences is small enough to look like an
        // accident of transcription. Losing any of them changes the decoded
        // spectrum without changing its shape, which is the failure mode this
        // module is most exposed to.
        assert_ne!(MEAN_LSF_3, MEAN_LSF_5, "the two long-term means are the same");
        assert_ne!(ALPHA_3.0, ALPHA_5.0, "the two concealment weights are the same");
        assert!(
            PRED_FAC_3.iter().any(|&f| f != LSP_PRED_FAC_MR122.0),
            "the per-coefficient predictor collapsed onto the 12.2 scalar"
        );

        // The two complements do not even round the same way. 0.9 and 0.1 in
        // Q15 happen to sum to exact unity; 0.95 and 0.05, rounded
        // independently, land one LSB short — so a concealed 12.2 spectrum
        // contracts slightly on every erased frame and a concealed 5.9 one does
        // not. Asserted rather than assumed, because making the second pair sum
        // to 32768 is the obvious tidy-up and it is wrong.
        assert_eq!(i32::from(ALPHA_3.0) + i32::from(ONE_ALPHA_3.0), 32768);
        assert_eq!(i32::from(ALPHA_5.0) + i32::from(ONE_ALPHA_5.0), 32767);
    }

    #[test]
    fn a_frozen_spectrum_survives_interpolation_unchanged() {
        // Nothing moves across the frame, so every subframe must get the same
        // filter. Halving twice reproduces the input exactly only for even
        // LSPs — `shr` floors — and the reset vector is all even, which is why
        // it is the one used here.
        let mut ctx = DspContext::default();
        let lsp = initial_lsp();
        assert!(lsp.iter().all(|v| v.0 % 2 == 0), "the reset LSPs are not all even");

        let az = interpolate_lsp_mid(&mut ctx, &lsp, &lsp, &lsp);
        let want = lsp_to_lp(&mut ctx, &lsp);
        for sf in 0..NB_SUBFR {
            assert_eq!(
                az[sf * MP1..(sf + 1) * MP1].iter().map(|w| w.0).collect::<Vec<_>>(),
                want.iter().map(|w| w.0).collect::<Vec<_>>(),
                "subframe {sf} does not reproduce a frozen spectrum"
            );
        }
    }

    #[test]
    fn the_transmitted_sets_reach_their_subframes_unmodified() {
        // Two of the four subframes are not interpolated at all. A weighting
        // applied where none belongs would still produce an ordered, stable
        // filter.
        let mut dec = LsfDecoder::at_reset();
        let (mid, new) = dec.decode_pair(&[29, 44, 274, 202, 5], false);
        let old = initial_lsp();

        let mut ctx = DspContext::default();
        let az = interpolate_lsp_mid(&mut ctx, &old, &mid, &new);
        assert_eq!(az[MP1..2 * MP1], lsp_to_lp(&mut ctx, &mid), "subframe 2");
        assert_eq!(az[3 * MP1..], lsp_to_lp(&mut ctx, &new), "subframe 4");
    }

    #[test]
    fn the_two_interpolators_are_not_interchangeable() {
        // A guard against later "unification". Even when the mid-frame and
        // 4th-subframe sets coincide — the case where the two schemes look most
        // alike — the first subframe is 1/2 old + 1/2 new here and 3/4 old +
        // 1/4 new there.
        let mut dec = LsfDecoder::at_reset();
        let (_, new) = dec.decode_pair(&[77, 37, 423, 36, 7], false);
        let old = initial_lsp();

        let mut ctx = DspContext::default();
        let both = interpolate_lsp_mid(&mut ctx, &old, &new, &new);
        let across = interpolate_lsp(&mut ctx, &old, &new);
        assert_ne!(
            both.map(|w| w.0),
            across.map(|w| w.0),
            "the two interpolation schemes agree, which means one is wrong"
        );
    }

    #[test]
    fn a_sustained_12k2_erasure_holds_one_spectrum_and_converges() {
        // Both sets come from one concealed vector, so a 12.2 erasure freezes
        // the spectrum for the whole frame rather than gliding through it; and
        // repeated erasures must settle on the mean rather than drift.
        let mut dec = LsfDecoder::at_reset();
        let indices = [29u16, 44, 274, 202, 5];
        for _ in 0..4 {
            dec.decode_pair(&indices, false);
        }

        let (mut previous, first_new) = dec.decode_pair(&indices, true);
        assert_eq!(previous, first_new, "an erased frame produced two spectra");

        let mut first_gap = 0i32;
        for n in 0..12 {
            let (mid, new) = dec.decode_pair(&indices, true);
            assert_eq!(mid, new, "erasure {n} produced two spectra");
            let gap: i32 = (0..M)
                .map(|i| (i32::from(new[i].0) - i32::from(previous[i].0)).abs())
                .sum();
            if n == 0 {
                first_gap = gap;
            } else if n == 11 {
                assert!(gap <= first_gap, "erasure {n} moved {gap}, not below {first_gap}");
            }
            previous = new;
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
