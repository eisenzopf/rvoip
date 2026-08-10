//! AMR-NB LSF quantisation, encoder side — 3GPP TS 26.090 §5.2.5.
//!
//! Implements TS 26.073's `Lsp_lsf` (`lsp_lsf.c`), `Lsf_wt` (`lsfwt.c`),
//! `Q_plsf_3` with its `Vq_subvec3`/`Vq_subvec4` kernels (`q_plsf_3.c`), and
//! `Q_plsf_5` with `Vq_subvec`/`Vq_subvec_s` (`q_plsf_5.c`). `Reorder_lsf` and
//! `Lsf_lsp` are the decoder's, reused from [`super::super::lsp`] — the
//! reference calls the same two functions on both sides.
//!
//! # Four searches, one shape, one tie-break
//!
//! Every kernel here minimises `Σ_k mult(w[k], r[k] − cb[k])²` accumulated with
//! `L_mac`, scans its codebook exhaustively from index 0 upward, and keeps a
//! candidate only on a **strict** improvement. Ties therefore go to the lowest
//! index, and they are not rare: the Q13 weight is applied with `mult`, an
//! arithmetic `>>15` that floors, so many distinct errors collapse onto the same
//! integer before they are squared. A `<=` port produces speech that sounds
//! identical and a bitstream that is wrong on those frames.
//!
//! Two consequences of using the ETSI operators rather than wider arithmetic are
//! part of the specification, not artefacts:
//!
//! - `mult` floors toward −∞. A rounding multiply, or an `f64` path, reorders
//!   candidates whose true weighted errors differ by less than one LSB.
//! - The `L_mac` chain saturates at 2³¹−1, and `dist_min` starts at `MAX_32`.
//!   A candidate whose distance saturates can never satisfy
//!   `L_sub(dist, dist_min) < 0` against the initial state, so a saturating
//!   first candidate leaves the index at 0 — and a later saturating candidate
//!   cannot displace an earlier winner either. An `i64` accumulator changes
//!   which codeword is transmitted.
//!
//! There is no pruning and no early exit anywhere in this file.
//!
//! # Validated by
//!
//! The three-split path is checked against the committed reference bitstream
//! `testdata/amrnb_enc_mode4.amr`: the encoder trace's unquantised `lsp_new` is
//! fed in, and all three transmitted indices are compared for each of the three
//! committed frames — an *index* comparison, which is strictly stronger than
//! comparing the spectrum the index selects. Its reconstruction is then
//! compared against [`super::super::lsp::LsfDecoder`], which is bit-exact
//! against TS 26.073.
//!
//! The five-split path (12.2 kbit/s) has no committed trace, so its
//! reconstruction is cross-checked against the same bit-exact decoder and its
//! search is tested through hand-built codebooks that pin the tie-break
//! directions. See the note in [`LsfQuantiser::quantise_pair`].

use super::super::decoder_tables::{
    COS_TABLE, DICO1_LSF_3, DICO1_LSF_5, DICO2_LSF_3, DICO2_LSF_5, DICO3_LSF_3, DICO3_LSF_5,
    DICO4_LSF_5, DICO5_LSF_5, MEAN_LSF_3, MEAN_LSF_5, MR515_3_LSF, MR795_1_LSF, PRED_FAC_3,
};
use super::super::lsp::{lsf_to_lsp, reorder_lsf, M};
use crate::fixed_point::arith::{add, mult, negate, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult, l_sub};
use crate::fixed_point::shift::{l_shl, shl};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_32};

/// Minimum spacing between adjacent quantised LSFs, on the 0..16384 scale.
const LSF_GAP: Word16 = Word16(205);

/// MA prediction factor at 12.2 kbit/s, 0.65 in Q15.
///
/// One scalar for all ten coefficients, where the three-split quantiser has a
/// per-coefficient [`PRED_FAC_3`]. The two are not interchangeable.
const LSP_PRED_FAC_MR122: Word16 = Word16(21299);

/// Mode index of 4.75 kbit/s.
const MR475: u8 = 0;
/// Mode index of 5.15 kbit/s.
const MR515: u8 = 1;
/// Mode index of 7.95 kbit/s.
const MR795: u8 = 5;

/// The upper bound of the normalised-LSF scale: 0.5, i.e. 4000 Hz.
const LSF_NYQUIST: Word16 = Word16(16384);

/// Distance at which [`lsf_weights`] switches line segments, 450 Hz.
const WEIGHT_KNEE: Word16 = Word16(1843);

/// Value of the weight at zero spacing, 3.347 in Q10.
const WEIGHT_AT_ZERO: Word16 = Word16(3427);

/// Slope of the steep segment, Q15.
const WEIGHT_SLOPE_NEAR: Word16 = Word16(28160);

/// Slope of the shallow segment, Q15.
const WEIGHT_SLOPE_FAR: Word16 = Word16(6242);

/// How a three-dimensional codebook is walked.
///
/// 4.75 and 5.15 kbit/s spend eight bits on the second split of a
/// nine-bit codebook by visiting only its even-numbered codevectors. The
/// reference expresses that as a pointer that advances by six words instead of
/// three, with the transmitted index already halved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stride {
    /// Every codevector is a candidate.
    Every,
    /// Only the even-numbered codevectors are candidates; codevector `i` starts
    /// at word `6 * i`.
    EveryOther,
}

impl Stride {
    /// Words between the starts of successive candidates.
    const fn step(self) -> usize {
        match self {
            Self::Every => 3,
            Self::EveryOther => 6,
        }
    }
}

/// One frame's three-split quantisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quantised {
    /// The three transmitted split indices, Q0.
    pub indices: [u16; 3],
    /// The quantised LSPs, Q15 — what the decoder will reconstruct.
    pub lsp: [Word16; M],
}

/// One 12.2 kbit/s frame's five-split quantisation of both LSF sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuantisedPair {
    /// The five transmitted matrix indices, Q0.
    pub indices: [u16; 5],
    /// Quantised LSPs at the 2nd subframe, Q15.
    pub mid: [Word16; M],
    /// Quantised LSPs at the 4th subframe, Q15.
    pub new: [Word16; M],
}

/// The MA prediction memory shared by both quantisers, TS 26.073 `Q_plsfState`.
///
/// A single ten-word vector of the previous frame's *quantised* residual, reset
/// to zero. It persists across frames, never across subframes, and it is the
/// only state either quantiser has. The five-split quantiser stores the
/// **second** of its two residuals here; storing the first, or an average,
/// leaves the decoder predicting against something the encoder never used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LsfQuantiser {
    past_rq: [Word16; M],
}

impl LsfQuantiser {
    /// A quantiser in the state `Q_plsf_reset` leaves behind: no prediction
    /// memory.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            past_rq: [Word16(0); M],
        }
    }

    /// The carried quantised residual, Q15 on the normalised-LSF scale.
    #[must_use]
    pub const fn prediction_memory(&self) -> &[Word16; M] {
        &self.past_rq
    }

    /// Quantise one LSP set with the three-split VQ — `Q_plsf_3`.
    ///
    /// `lsp` is the unquantised 4th-subframe LSP vector, Q15. Returns the three
    /// transmitted indices and the LSPs the decoder will rebuild from them,
    /// Q15, and advances the prediction memory.
    ///
    /// Used by every rate except 12.2 kbit/s. `mode_index` selects the codebook
    /// triple, and only three groupings exist: 4.75/5.15 share one, 7.95 has its
    /// own, and 5.90/6.70/7.40/10.2 share the third.
    ///
    /// The reference's `MRDTX` branch — which searches eight predictor seed
    /// vectors instead of using the carried memory — is deliberately absent. It
    /// is unreachable from `lsp()`, whose call is guarded by
    /// `used_mode != MRDTX`; only `dtx_enc()` enters it, and comfort noise is
    /// not implemented here.
    ///
    /// # Panics
    ///
    /// If `mode_index` is 12.2 kbit/s or higher, which has no three-split
    /// quantiser. The books would produce a plausible spectrum from indices
    /// that mean something else.
    pub fn quantise(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        lsp: &[Word16; M],
    ) -> Quantised {
        assert!(mode_index < 7, "12.2 kbit/s quantises through quantise_pair");

        let lsf = lsp_to_lsf(ctx, lsp);
        let weights = lsf_weights(ctx, &lsf);

        // MA prediction, per coefficient. `mult` floors; a rounding multiply
        // moves the residual by an LSB and with it the chosen codeword.
        let mut predicted = [Word16(0); M];
        let mut residual = [Word16(0); M];
        for i in 0..M {
            let carried = mult(ctx, self.past_rq[i], Word16(PRED_FAC_3[i]));
            predicted[i] = add(ctx, Word16(MEAN_LSF_3[i]), carried);
            residual[i] = sub(ctx, lsf[i], predicted[i]);
        }

        let (first, first_size, second_size, third, third_size) = match mode_index {
            MR475 | MR515 => (&DICO1_LSF_3[..], 256, 256, &MR515_3_LSF[..], 128),
            MR795 => (&MR795_1_LSF[..], 512, 512, &DICO3_LSF_3[..], 512),
            _ => (&DICO1_LSF_3[..], 256, 512, &DICO3_LSF_3[..], 512),
        };
        let second_stride = if matches!(mode_index, MR475 | MR515) {
            Stride::EveryOther
        } else {
            Stride::Every
        };

        let mut split0 = [residual[0], residual[1], residual[2]];
        let mut split1 = [residual[3], residual[4], residual[5]];
        let mut split2 = [residual[6], residual[7], residual[8], residual[9]];

        let indices = [
            search_split3(
                ctx,
                &mut split0,
                first,
                &[weights[0], weights[1], weights[2]],
                first_size,
                Stride::Every,
            ),
            search_split3(
                ctx,
                &mut split1,
                &DICO2_LSF_3,
                &[weights[3], weights[4], weights[5]],
                second_size,
                second_stride,
            ),
            search_split4(
                ctx,
                &mut split2,
                third,
                &[weights[6], weights[7], weights[8], weights[9]],
                third_size,
            ),
        ];

        // The kernels overwrite the residual with the codevector they chose;
        // both the reconstruction and the state update read it back.
        residual[0..3].copy_from_slice(&split0);
        residual[3..6].copy_from_slice(&split1);
        residual[6..10].copy_from_slice(&split2);

        let mut quantised = [Word16(0); M];
        for i in 0..M {
            quantised[i] = add(ctx, residual[i], predicted[i]);
            // The memory takes the *quantised* residual, and takes it before
            // the spacing pass below — `past_rq` never sees a reordered value.
            self.past_rq[i] = residual[i];
        }

        reorder_lsf(ctx, &mut quantised, LSF_GAP);
        Quantised {
            indices,
            lsp: lsf_to_lsp(ctx, &quantised),
        }
    }

    /// Quantise both of a 12.2 kbit/s frame's LSP sets with the five-split
    /// matrix VQ — `Q_plsf_5`.
    ///
    /// `mid` and `new` are the unquantised LSPs at the 2nd and 4th subframes,
    /// Q15. Each of the five indices quantises the same coefficient *pair* in
    /// both sets at once, which is what makes this a matrix quantiser: the four
    /// dimensions of submatrix `k` are `{r1[2k], r1[2k+1], r2[2k], r2[2k+1]}`.
    ///
    /// Both residuals are formed against the **same** predicted vector, and the
    /// prediction memory afterwards holds the second residual only.
    ///
    /// # Ground truth
    ///
    /// There is no committed encoder trace at 12.2 kbit/s, so this path is
    /// validated by round-tripping through the bit-exact
    /// [`super::super::lsp::LsfDecoder::decode_pair`] — which proves the
    /// reconstruction, the index packing and the state update agree with the
    /// decoder — plus kernel-level tests that pin the tie-break directions the
    /// search depends on. The *choice* of codeword against real 12.2 input is
    /// not yet compared with the reference; that needs a mode-7 trace.
    pub fn quantise_pair(
        &mut self,
        ctx: &mut DspContext,
        mid: &[Word16; M],
        new: &[Word16; M],
    ) -> QuantisedPair {
        let lsf1 = lsp_to_lsf(ctx, mid);
        let lsf2 = lsp_to_lsf(ctx, new);
        let wf1 = lsf_weights(ctx, &lsf1);
        let wf2 = lsf_weights(ctx, &lsf2);

        let mut predicted = [Word16(0); M];
        let mut r1 = [Word16(0); M];
        let mut r2 = [Word16(0); M];
        for i in 0..M {
            let carried = mult(ctx, self.past_rq[i], LSP_PRED_FAC_MR122);
            predicted[i] = add(ctx, Word16(MEAN_LSF_5[i]), carried);
            r1[i] = sub(ctx, lsf1[i], predicted[i]);
            r2[i] = sub(ctx, lsf2[i], predicted[i]);
        }

        let mut indices = [0u16; 5];
        for (k, slot) in indices.iter_mut().enumerate() {
            let a = 2 * k;
            let mut pair1 = [r1[a], r1[a + 1]];
            let mut pair2 = [r2[a], r2[a + 1]];
            let w1 = [wf1[a], wf1[a + 1]];
            let w2 = [wf2[a], wf2[a + 1]];

            // Submatrix 2 alone has a signed codebook: nine transmitted bits
            // address 256 entries plus a sign.
            *slot = if k == 2 {
                search_matrix_signed(ctx, &mut pair1, &mut pair2, &DICO3_LSF_5, &w1, &w2, 256)
            } else {
                let (book, size): (&[i16], usize) = match k {
                    0 => (&DICO1_LSF_5, 128),
                    1 => (&DICO2_LSF_5, 256),
                    3 => (&DICO4_LSF_5, 256),
                    _ => (&DICO5_LSF_5, 64),
                };
                search_matrix(ctx, &mut pair1, &mut pair2, book, &w1, &w2, size)
            };

            r1[a] = pair1[0];
            r1[a + 1] = pair1[1];
            r2[a] = pair2[0];
            r2[a + 1] = pair2[1];
        }

        let mut q1 = [Word16(0); M];
        let mut q2 = [Word16(0); M];
        for i in 0..M {
            q1[i] = add(ctx, r1[i], predicted[i]);
            q2[i] = add(ctx, r2[i], predicted[i]);
            // The 4th-subframe residual only. Storing `r1`, or both, detunes
            // every following frame's prediction.
            self.past_rq[i] = r2[i];
        }

        reorder_lsf(ctx, &mut q1, LSF_GAP);
        reorder_lsf(ctx, &mut q2, LSF_GAP);

        QuantisedPair {
            indices,
            mid: lsf_to_lsp(ctx, &q1),
            new: lsf_to_lsp(ctx, &q2),
        }
    }
}

/// Convert LSPs (cosines, Q15) to LSFs (normalised frequencies, 0..16384) —
/// `Lsp_lsf`.
///
/// The inverse of [`super::super::lsp::lsf_to_lsp`], and the encoder's only
/// user of it. Table lookup on the same 65-point cosine table plus a linear
/// correction through the tabulated inverse slope.
///
/// The table cursor is **carried across coefficients**, descending from 63 and
/// only ever decreasing within a call. That is a valid search precisely because
/// LSPs arrive sorted descending, and it is not equivalent to searching each
/// coefficient independently if they ever were not.
///
/// The `L_shl(...,3)` is rounded with `round`, not shifted down — the reference
/// picks up half an LSB here that a plain shift would drop.
///
/// # Panics
///
/// Never in practice: the only fallible step converts a table index below 64
/// into an `i16`.
#[must_use]
pub fn lsp_to_lsf(ctx: &mut DspContext, lsp: &[Word16; M]) -> [Word16; M] {
    let mut lsf = [Word16(0); M];
    let mut ind = COS_TABLE.len() - 2;

    for i in (0..M).rev() {
        // `ind > 0` cannot actually stop this loop: COS_TABLE[0] is 32767, so
        // the condition fails at the latest there for any Word16 input. It is
        // written so a malformed input cannot index out of bounds.
        while ind > 0 && sub(ctx, Word16(COS_TABLE[ind]), lsp[i]).0 < 0 {
            ind -= 1;
        }

        let step = sub(ctx, lsp[i], Word16(COS_TABLE[ind]));
        // Both operands are non-positive here — the slope table is entirely
        // negative — so the product is non-negative. A sign slip is silent.
        let scaled = l_mult(ctx, step, Word16(super::super::decoder_tables::ACOS_SLOPE[ind]));
        let promoted = l_shl(ctx, scaled, 3);
        let fine = round(ctx, promoted);
        let coarse = shl(
            ctx,
            Word16(i16::try_from(ind).expect("cosine table index fits in i16")),
            8,
        );
        lsf[i] = add(ctx, fine, coarse);
    }
    lsf
}

/// Perceptual weights for the LSF quantiser's distance measure — `Lsf_wt`.
///
/// `lsf` is on the 0..16384 scale; the result is Q13. Despite the reference's
/// comment ("square of weighting factors") the output is the weight itself.
///
/// The weight is a decreasing two-segment function of the spacing
/// `d[i] = lsf[i+1] − lsf[i−1]`, with virtual neighbours at 0 and at 16384 —
/// which is 0.5 on this scale, i.e. the *Nyquist* frequency of 4000 Hz. Closely
/// spaced lines get more weight, because that is where the spectrum has a
/// resonance worth preserving.
///
/// Two details that change the numbers: the branch tests the already-subtracted
/// `d − 1843` and the far arm multiplies *that*, not `d`; and the first `mult`
/// takes the raw Q0 spacing, so it is not a Q15×Q15 product. The final
/// `shl(…, 3)` is what lands the Q10 constants in Q13.
#[must_use]
pub fn lsf_weights(ctx: &mut DspContext, lsf: &[Word16; M]) -> [Word16; M] {
    let mut wf = [Word16(0); M];

    wf[0] = lsf[1];
    for i in 1..M - 1 {
        wf[i] = sub(ctx, lsf[i + 1], lsf[i - 1]);
    }
    wf[M - 1] = sub(ctx, LSF_NYQUIST, lsf[M - 2]);

    for slot in &mut wf {
        let past_knee = sub(ctx, *slot, WEIGHT_KNEE);
        *slot = if past_knee.0 < 0 {
            let drop = mult(ctx, *slot, WEIGHT_SLOPE_NEAR);
            sub(ctx, WEIGHT_AT_ZERO, drop)
        } else {
            let drop = mult(ctx, past_knee, WEIGHT_SLOPE_FAR);
            sub(ctx, WEIGHT_KNEE, drop)
        };
        *slot = shl(ctx, *slot, 3);
    }
    wf
}

/// Weighted squared error of one candidate, accumulated exactly as the
/// reference does.
///
/// `L_mult` seeds the accumulator from the first term rather than adding to
/// zero, and every later term goes through `L_mac`. Both saturate.
fn weighted_distance(
    ctx: &mut DspContext,
    residual: &[Word16],
    codeword: &[i16],
    weights: &[Word16],
) -> Word32 {
    let mut dist = Word32(0);
    for k in 0..residual.len() {
        let err = sub(ctx, residual[k], Word16(codeword[k]));
        let term = mult(ctx, weights[k], err);
        dist = if k == 0 {
            l_mult(ctx, term, term)
        } else {
            l_mac(ctx, dist, term, term)
        };
    }
    dist
}

/// Search a three-dimensional split codebook — `Vq_subvec3`.
///
/// Returns the chosen index and **overwrites `residual` with the chosen
/// codevector**; the caller's reconstruction and prediction memory both depend
/// on that. `size` is the number of candidates, already halved when `stride` is
/// [`Stride::EveryOther`].
///
/// Scans `0..size` ascending and keeps a candidate only on
/// `L_sub(dist, dist_min) < 0`, so **ties go to the lowest index**.
///
/// # Panics
///
/// If the codebook is too short for `size` candidates at this stride.
pub fn search_split3(
    ctx: &mut DspContext,
    residual: &mut [Word16; 3],
    book: &[i16],
    weights: &[Word16; 3],
    size: usize,
    stride: Stride,
) -> u16 {
    let step = stride.step();
    assert!(
        book.len() >= (size - 1) * step + 3,
        "codebook holds fewer than {size} candidates at stride {step}"
    );

    let mut dist_min = Word32(MAX_32);
    let mut index = 0usize;
    for i in 0..size {
        let base = i * step;
        let dist = weighted_distance(ctx, residual, &book[base..base + 3], weights);
        if l_sub(ctx, dist, dist_min).0 < 0 {
            dist_min = dist;
            index = i;
        }
    }

    let base = index * step;
    residual.copy_from_slice(&[
        Word16(book[base]),
        Word16(book[base + 1]),
        Word16(book[base + 2]),
    ]);
    u16::try_from(index).expect("split index fits in 16 bits")
}

/// Search a four-dimensional split codebook — `Vq_subvec4`.
///
/// As [`search_split3`], with four dimensions and no half-stride variant.
///
/// # Panics
///
/// If the codebook is too short for `size` candidates.
pub fn search_split4(
    ctx: &mut DspContext,
    residual: &mut [Word16; 4],
    book: &[i16],
    weights: &[Word16; 4],
    size: usize,
) -> u16 {
    assert!(book.len() >= size * 4, "codebook holds fewer than {size} candidates");

    let mut dist_min = Word32(MAX_32);
    let mut index = 0usize;
    for i in 0..size {
        let base = i * 4;
        let dist = weighted_distance(ctx, residual, &book[base..base + 4], weights);
        if l_sub(ctx, dist, dist_min).0 < 0 {
            dist_min = dist;
            index = i;
        }
    }

    let base = index * 4;
    for (k, slot) in residual.iter_mut().enumerate() {
        *slot = Word16(book[base + k]);
    }
    u16::try_from(index).expect("split index fits in 16 bits")
}

/// Weighted squared error of one four-word matrix candidate.
///
/// Term order is `r1[0], r1[1], r2[0], r2[1]` — the same order the codebook
/// stores them in, which is why one pointer walks both residuals.
fn matrix_distance(
    ctx: &mut DspContext,
    r1: [Word16; 2],
    r2: [Word16; 2],
    codeword: &[i16],
    w1: [Word16; 2],
    w2: [Word16; 2],
    invert: bool,
) -> Word32 {
    let mut dist = Word32(0);
    for k in 0..4 {
        let (value, weight) = if k < 2 { (r1[k], w1[k]) } else { (r2[k - 2], w2[k - 2]) };
        // The negative hypothesis is evaluated as `r + cb`, not by negating the
        // codeword: `negate(-32768)` saturates to 32767 and the two differ.
        let err = if invert {
            add(ctx, value, Word16(codeword[k]))
        } else {
            sub(ctx, value, Word16(codeword[k]))
        };
        let term = mult(ctx, weight, err);
        dist = if k == 0 {
            l_mult(ctx, term, term)
        } else {
            l_mac(ctx, dist, term, term)
        };
    }
    dist
}

/// Search a four-dimensional matrix codebook — `Vq_subvec`.
///
/// Overwrites both residual pairs with the chosen codevector. Ascending scan,
/// strict improvement, so **ties go to the lowest index**.
///
/// # Panics
///
/// If the codebook is too short for `size` candidates.
pub fn search_matrix(
    ctx: &mut DspContext,
    r1: &mut [Word16; 2],
    r2: &mut [Word16; 2],
    book: &[i16],
    w1: &[Word16; 2],
    w2: &[Word16; 2],
    size: usize,
) -> u16 {
    assert!(book.len() >= size * 4, "codebook holds fewer than {size} candidates");

    let mut dist_min = Word32(MAX_32);
    let mut index = 0usize;
    for i in 0..size {
        let base = i * 4;
        let dist = matrix_distance(ctx, *r1, *r2, &book[base..base + 4], *w1, *w2, false);
        if l_sub(ctx, dist, dist_min).0 < 0 {
            dist_min = dist;
            index = i;
        }
    }

    let base = index * 4;
    r1[0] = Word16(book[base]);
    r1[1] = Word16(book[base + 1]);
    r2[0] = Word16(book[base + 2]);
    r2[1] = Word16(book[base + 3]);
    u16::try_from(index).expect("matrix index fits in 16 bits")
}

/// Search a signed four-dimensional matrix codebook — `Vq_subvec_s`.
///
/// Each codevector is tried twice, positive then negative, and the transmitted
/// index is `2 * codevector + sign`. Both hypotheses use the same strict
/// improvement test in that order, which fixes two tie directions:
///
/// - `+cb_i` tied with `−cb_i` → **positive wins**, because it was tested first;
/// - `−cb_i` tied with `+cb_j` for `j > i` → **the earlier `i`, negative, wins**.
///
/// The chosen codevector is written back negated when the sign bit is set, with
/// the saturating `negate` — so a codeword of −32768 reads back as 32767, and
/// the encoder and decoder agree on that because the decoder negates it the
/// same way.
///
/// # Panics
///
/// If the codebook is too short for `size` candidates.
pub fn search_matrix_signed(
    ctx: &mut DspContext,
    r1: &mut [Word16; 2],
    r2: &mut [Word16; 2],
    book: &[i16],
    w1: &[Word16; 2],
    w2: &[Word16; 2],
    size: usize,
) -> u16 {
    assert!(book.len() >= size * 4, "codebook holds fewer than {size} candidates");

    let mut dist_min = Word32(MAX_32);
    let mut index = 0usize;
    let mut negative = false;

    for i in 0..size {
        let base = i * 4;
        let word = &book[base..base + 4];

        let positive = matrix_distance(ctx, *r1, *r2, word, *w1, *w2, false);
        if l_sub(ctx, positive, dist_min).0 < 0 {
            dist_min = positive;
            index = i;
            negative = false;
        }

        let inverted = matrix_distance(ctx, *r1, *r2, word, *w1, *w2, true);
        if l_sub(ctx, inverted, dist_min).0 < 0 {
            dist_min = inverted;
            index = i;
            negative = true;
        }
    }

    let base = index * 4;
    let read = |ctx: &mut DspContext, at: usize| {
        let v = Word16(book[at]);
        if negative {
            negate(ctx, v)
        } else {
            v
        }
    };
    r1[0] = read(ctx, base);
    r1[1] = read(ctx, base + 1);
    r2[0] = read(ctx, base + 2);
    r2[1] = read(ctx, base + 3);

    let index = u16::try_from(index).expect("matrix index fits in 16 bits");
    index * 2 + u16::from(negative)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::super::bitstream::parse;
    use super::super::super::lsp::LsfDecoder;

    /// The committed 7.40 kbit/s encoder trace, three frames.
    const TRACE: &str = include_str!("../../testdata/nb_enc_trace.txt");

    /// TS 26.073's own 7.40 kbit/s output for `testdata/amrnb_enc_input.pcm`.
    const MODE4: &[u8] = include_bytes!("../../testdata/amrnb_enc_mode4.amr");

    /// 7.40 kbit/s.
    const MR74: u8 = 4;

    /// One trace row's values.
    ///
    /// Panics rather than returning `None`: a test that silently compares
    /// nothing is worse than one that fails.
    fn trace(frame: usize, subframe: i32, name: &str) -> Vec<i16> {
        let want = format!("T {frame} {subframe} {name} ");
        for line in TRACE.lines() {
            if let Some(rest) = line.strip_prefix(&want) {
                return rest
                    .split_whitespace()
                    .map(|v| v.parse().expect("trace value fits in i16"))
                    .collect();
            }
        }
        panic!("the committed trace has no row {want:?}");
    }

    fn trace_lsp(frame: usize) -> [Word16; M] {
        let v = trace(frame, 0, "lsp_new");
        assert_eq!(v.len(), M, "lsp_new is ten coefficients");
        let mut out = [Word16(0); M];
        for (slot, value) in out.iter_mut().zip(v) {
            *slot = Word16(value);
        }
        out
    }

    /// Transmitted parameters of one frame of the committed bitstream.
    fn reference_parameters(frame: usize) -> Vec<u16> {
        // "#!AMR\n" magic, then ToC-prefixed frames of a fixed size at a fixed
        // mode.
        const PAYLOAD: usize = 19; // 148 bits, rounded up
        let offset = 6 + frame * (1 + PAYLOAD);
        let toc = MODE4[offset];
        assert_eq!((toc >> 3) & 0x0f, MR74, "frame {frame}: ToC mode");
        parse(MR74, &MODE4[offset + 1..offset + 1 + PAYLOAD]).expect("frame parses")
    }

    #[test]
    fn three_split_indices_match_the_reference_bitstream() {
        // The strongest test available: the *indices* TS 26.073's own encoder
        // transmitted, for the frames whose unquantised input the trace
        // records. A search that picks an equally-good neighbouring codeword
        // fails here and nowhere else.
        let mut ctx = DspContext::default();
        let mut quantiser = LsfQuantiser::new();
        let mut compared = 0;

        for frame in 0..3 {
            let got = quantiser.quantise(&mut ctx, MR74, &trace_lsp(frame)).indices;
            let want = reference_parameters(frame);
            for (split, &index) in got.iter().enumerate() {
                assert_eq!(
                    index, want[split],
                    "frame {frame} split {split}: chose {index}, TS 26.073 chose {}",
                    want[split]
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 9, "three frames of three indices");
    }

    #[test]
    fn three_split_reconstruction_matches_the_bit_exact_decoder() {
        // `Q_plsf_3` and `D_plsf_3` share the codebooks, the mean, the
        // predictor and the spacing pass, so the encoder's own reconstruction
        // has to equal what the decoder rebuilds from the same indices — and
        // the decoder is bit-exact against TS 26.073's vectors. This also
        // catches a prediction memory that advances differently on the two
        // sides, because both run three frames in sequence.
        let mut ctx = DspContext::default();
        let mut quantiser = LsfQuantiser::new();
        let mut decoder = LsfDecoder::at_reset();
        let mut compared = 0;

        for frame in 0..3 {
            let encoded = quantiser.quantise(&mut ctx, MR74, &trace_lsp(frame));
            let rebuilt = decoder.decode(MR74, &encoded.indices, false);
            for (i, (&mine, &theirs)) in encoded.lsp.iter().zip(rebuilt.iter()).enumerate() {
                assert_eq!(mine.0, theirs.0, "frame {frame} coefficient {i}");
                compared += 1;
            }
        }
        assert_eq!(compared, 30, "three frames of ten coefficients");
    }

    #[test]
    fn the_prediction_memory_carries_the_quantised_residual_across_frames() {
        // Quantising frame 1 from a fresh quantiser must differ from
        // quantising it after frame 0, or the MA prediction is not being
        // carried and every following frame is quantised against the mean.
        let mut ctx = DspContext::default();

        let mut fresh = LsfQuantiser::new();
        assert_eq!(fresh.prediction_memory(), &[Word16(0); M]);
        let cold = fresh.quantise(&mut ctx, MR74, &trace_lsp(1)).indices;

        let mut warm = LsfQuantiser::new();
        let _ = warm.quantise(&mut ctx, MR74, &trace_lsp(0));
        assert_ne!(
            warm.prediction_memory(),
            &[Word16(0); M],
            "the memory did not advance"
        );
        let hot = warm.quantise(&mut ctx, MR74, &trace_lsp(1)).indices;

        assert_ne!(cold, hot, "the prediction had no effect on the chosen indices");
        // And the warm run is the one the reference agrees with.
        assert_eq!(hot.to_vec(), reference_parameters(1)[..3].to_vec());
    }

    #[test]
    fn every_rates_codebook_triple_round_trips_through_the_decoder() {
        // Only 7.40 kbit/s has a committed trace, so the other six three-split
        // rates are checked against the decoder instead. This is what covers
        // 4.75/5.15's half-stride second split — whose read-back is at word
        // 6·index, not 3·index — and 7.95's own first-split book.
        let mut compared = 0;
        for mode_index in 0..7u8 {
            let mut ctx = DspContext::default();
            let mut quantiser = LsfQuantiser::new();
            let mut decoder = LsfDecoder::at_reset();
            for frame in 0..3 {
                let encoded = quantiser.quantise(&mut ctx, mode_index, &trace_lsp(frame));
                let rebuilt = decoder.decode(mode_index, &encoded.indices, false);
                assert_eq!(encoded.lsp, rebuilt, "mode {mode_index} frame {frame}");
                // And the indices fit the field widths the bitstream reserves.
                let widths: [u16; 3] = match mode_index {
                    0 | 1 => [256, 256, 128],
                    5 => [512, 512, 512],
                    _ => [256, 512, 512],
                };
                for (split, (&index, &size)) in
                    encoded.indices.iter().zip(widths.iter()).enumerate()
                {
                    assert!(index < size, "mode {mode_index} split {split}: {index} >= {size}");
                }
                compared += 3;
            }
        }
        assert_eq!(compared, 63, "seven rates of three frames of three indices");
    }

    #[test]
    fn five_split_round_trips_through_the_bit_exact_decoder() {
        // No mode-7 trace is committed, so the reference point is the decoder:
        // reconstruction, index packing (including the signed submatrix's
        // 2i+sign) and the `past_rq = r2` state update all have to agree, for
        // three frames in sequence.
        let mut ctx = DspContext::default();
        let mut quantiser = LsfQuantiser::new();
        let mut decoder = LsfDecoder::at_reset();
        let mut compared = 0;

        for frame in 0..3 {
            // Two genuinely different LSP sets, so the matrix quantiser is not
            // fed the same vector twice.
            let mid = trace_lsp(frame);
            let new = trace_lsp((frame + 1) % 3);
            let encoded = quantiser.quantise_pair(&mut ctx, &mid, &new);
            let (rebuilt_mid, rebuilt_new) = decoder.decode_pair(&encoded.indices, false);
            for i in 0..M {
                assert_eq!(encoded.mid[i].0, rebuilt_mid[i].0, "frame {frame} mid {i}");
                assert_eq!(encoded.new[i].0, rebuilt_new[i].0, "frame {frame} new {i}");
                compared += 2;
            }
        }
        assert_eq!(compared, 60, "three frames of two ten-coefficient sets");
    }

    #[test]
    fn the_five_split_signed_submatrix_uses_its_sign_bit() {
        // Submatrix 2's index is 2*codevector + sign, so an odd index proves
        // the negative hypothesis is reachable and packed as the low bit. If
        // the sign search were dropped, index 2 would always be even.
        let mut ctx = DspContext::default();
        let mut quantiser = LsfQuantiser::new();
        let mut odd = 0;
        for frame in 0..3 {
            let q = quantiser.quantise_pair(&mut ctx, &trace_lsp(frame), &trace_lsp((frame + 1) % 3));
            assert!(q.indices[2] < 512, "signed index is nine bits");
            odd += usize::from(q.indices[2] % 2 == 1);
        }
        assert!(odd > 0, "the negative hypothesis never won in three frames");
    }

    #[test]
    fn the_weights_switch_segments_at_the_knee() {
        // Pin the branch: a spacing one below 1843 takes the steep arm, one at
        // 1843 takes the shallow arm, and the shallow arm multiplies the
        // *subtracted* value rather than the spacing.
        let mut ctx = DspContext::default();

        // wf[0] is lsf[1] itself, so lsf[1] sets the first spacing directly.
        let mut lsf = [Word16(0); M];
        lsf[1] = Word16(1842);
        let steep = lsf_weights(&mut ctx, &lsf)[0];
        let drop = mult(&mut ctx, Word16(1842), WEIGHT_SLOPE_NEAR);
        let expected = sub(&mut ctx, WEIGHT_AT_ZERO, drop);
        assert_eq!(steep.0, shl(&mut ctx, expected, 3).0);

        lsf[1] = Word16(1843);
        let shallow = lsf_weights(&mut ctx, &lsf)[0];
        assert_eq!(shallow.0, shl(&mut ctx, WEIGHT_KNEE, 3).0, "at the knee the correction is zero");
        assert!(shallow.0 < steep.0, "the weight decreases with spacing");

        // wf[9] is measured against 16384, the Nyquist frequency on this scale.
        let mut wide = [Word16(0); M];
        wide[8] = Word16(16384 - 1843);
        assert_eq!(lsf_weights(&mut ctx, &wide)[9].0, shl(&mut ctx, WEIGHT_KNEE, 3).0);
    }

    #[test]
    fn lsp_to_lsf_inverts_the_decoders_conversion_closely() {
        // Not an identity — both directions are piecewise-linear table lookups
        // — but a sign error or a table mix-up moves it by thousands, not by
        // an LSB or two.
        let mut ctx = DspContext::default();
        let mut compared = 0;
        for frame in 0..3 {
            let lsp = trace_lsp(frame);
            let lsf = lsp_to_lsf(&mut ctx, &lsp);
            // LSFs are ascending and inside the normalised range.
            for i in 1..M {
                assert!(lsf[i].0 > lsf[i - 1].0, "frame {frame}: lsf {i} out of order");
            }
            assert!(lsf[M - 1].0 < 16384, "frame {frame}: lsf above Nyquist");
            let back = lsf_to_lsp(&mut ctx, &lsf);
            for i in 0..M {
                assert!(
                    (i32::from(back[i].0) - i32::from(lsp[i].0)).abs() <= 4,
                    "frame {frame} coefficient {i}: {} round-tripped to {}",
                    lsp[i].0,
                    back[i].0
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 30);
    }

    #[test]
    fn split_searches_break_ties_toward_the_lowest_index() {
        // Two identical codewords: the reference's strict `<` keeps the first.
        // A `<=` port would return the second, produce an identical spectrum,
        // and a wrong bitstream.
        let mut ctx = DspContext::default();
        let weights = [Word16(8192); 3];

        let book3: [i16; 12] = [
            100, 0, 0, // index 0
            100, 0, 0, // index 1: the same distance, exactly
            0, 0, 0,   // index 2: exact, so it must win outright
            7, 7, 7,
        ];
        let mut residual = [Word16(0); 3];
        assert_eq!(
            search_split3(&mut ctx, &mut residual, &book3, &weights, 2, Stride::Every),
            0,
            "the tie must go to the lower index"
        );
        assert_eq!(
            residual,
            [Word16(100), Word16(0), Word16(0)],
            "the codevector is written back"
        );
        let mut residual = [Word16(0); 3];
        assert_eq!(
            search_split3(&mut ctx, &mut residual, &book3, &weights, 3, Stride::Every),
            2,
            "an exact match still wins"
        );
        assert_eq!(residual, [Word16(0); 3]);

        let book4: [i16; 12] = [50, 0, 0, 0, 50, 0, 0, 0, 1, 1, 1, 1];
        let mut residual4 = [Word16(0); 4];
        assert_eq!(
            search_split4(&mut ctx, &mut residual4, &book4, &[Word16(8192); 4], 2),
            0
        );
        assert_eq!(residual4, [Word16(50), Word16(0), Word16(0), Word16(0)]);
    }

    #[test]
    fn the_flooring_weight_makes_equal_errors_score_differently() {
        // `mult` is an arithmetic `>>15`, so it floors toward −∞ rather than
        // toward zero. An error of +50 and one of −50 are the same size and
        // score differently, and the codeword on the negative side loses. A
        // rounding multiply — or f64 — reverses this pair.
        let mut ctx = DspContext::default();
        let weights = [Word16(8192); 3];
        // mult(8192, -50) = floor(-12.5) = -13, but mult(8192, 50) = 12.
        assert_eq!(mult(&mut ctx, Word16(8192), Word16(-50)).0, -13);
        assert_eq!(mult(&mut ctx, Word16(8192), Word16(50)).0, 12);

        // Codeword 0 leaves an error of +50, which floors *down* to 12;
        // codeword 1 leaves −50, which floors *away from zero* to −13. Equal
        // errors, unequal scores, and the codeword below the target wins.
        let book: [i16; 6] = [-50, 0, 0, 50, 0, 0];
        let mut residual = [Word16(0); 3];
        assert_eq!(
            search_split3(&mut ctx, &mut residual, &book, &weights, 2, Stride::Every),
            0,
            "the error that floors to the smaller magnitude wins"
        );
    }

    #[test]
    fn the_half_stride_search_skips_the_odd_codevectors() {
        // 4.75 and 5.15 kbit/s address only every second entry of the
        // nine-bit second-split book. The perfect match here sits at
        // codevector 1 — odd — and must be unreachable, with the read-back
        // taken from word 6*index.
        let mut ctx = DspContext::default();
        let weights = [Word16(8192); 3];
        let book: [i16; 12] = [
            500, 500, 500, // codevector 0, visited
            0, 0, 0,       // codevector 1, skipped even though it is exact
            9, 9, 9,       // codevector 2, visited
            0, 0, 0,       // codevector 3, skipped
        ];

        let mut residual = [Word16(0); 3];
        let index = search_split3(&mut ctx, &mut residual, &book, &weights, 2, Stride::EveryOther);
        assert_eq!(index, 1, "codevector 2 is candidate 1 at this stride");
        assert_eq!(
            residual,
            [Word16(9), Word16(9), Word16(9)],
            "read back from word 6*index"
        );

        // The same book scanned at full stride does reach the exact entry,
        // which is what makes the assertion above meaningful.
        let mut residual = [Word16(0); 3];
        assert_eq!(
            search_split3(&mut ctx, &mut residual, &book, &weights, 4, Stride::Every),
            1
        );
    }

    #[test]
    fn a_saturating_distance_cannot_displace_an_earlier_candidate() {
        // Both candidates saturate the L_mac chain to MAX_32, so neither
        // satisfies the strict comparison against the initial dist_min and the
        // index stays at 0 — even though candidate 1's true error is smaller.
        // An i64 accumulator would pick candidate 1.
        let mut ctx = DspContext::default();
        let weights = [Word16(32767); 4];
        let book: [i16; 8] = [
            -32768, -32768, -32768, -32768, // enormous error
            -32000, -32000, -32000, -32000, // large, but strictly smaller
        ];
        let mut residual = [Word16(32767); 4];
        let index = search_split4(&mut ctx, &mut residual, &book, &weights, 2);
        assert_eq!(index, 0, "saturation makes the two compare equal");
    }

    #[test]
    fn the_signed_matrix_search_prefers_positive_then_earlier() {
        let mut ctx = DspContext::default();
        let w = [Word16(8192); 2];

        // A codeword of all zeros ties with its own negation, so the positive
        // hypothesis — evaluated first — must win and the index must be even.
        let book: [i16; 8] = [0, 0, 0, 0, 1, 1, 1, 1];
        let mut r1 = [Word16(0); 2];
        let mut r2 = [Word16(0); 2];
        assert_eq!(
            search_matrix_signed(&mut ctx, &mut r1, &mut r2, &book, &w, &w, 2),
            0,
            "+cb ties with -cb: positive wins"
        );

        // Target −100: codevector 0 negated is exact, codevector 1 positive is
        // exact too. The earlier index wins, with its sign bit set.
        let book: [i16; 8] = [100, 100, 100, 100, -100, -100, -100, -100];
        let mut r1 = [Word16(-100); 2];
        let mut r2 = [Word16(-100); 2];
        let index = search_matrix_signed(&mut ctx, &mut r1, &mut r2, &book, &w, &w, 2);
        assert_eq!(index, 1, "index 0 with the sign bit, i.e. 2*0 + 1");
        assert_eq!(r1, [Word16(-100); 2], "the codevector is written back negated");
        assert_eq!(r2, [Word16(-100); 2]);
    }

    #[test]
    fn the_matrix_search_interleaves_the_two_residuals() {
        // A codeword is {r1[0], r1[1], r2[0], r2[1]}. Feeding two different
        // residual pairs shows the order is not {r1[0], r2[0], r1[1], r2[1]}.
        let mut ctx = DspContext::default();
        let w = [Word16(8192); 2];
        let book: [i16; 8] = [1, 2, 3, 4, 1, 3, 2, 4];
        let mut r1 = [Word16(1), Word16(2)];
        let mut r2 = [Word16(3), Word16(4)];
        assert_eq!(
            search_matrix(&mut ctx, &mut r1, &mut r2, &book, &w, &w, 2),
            0,
            "candidate 0 matches exactly under the interleaved order"
        );
    }
}
