//! ISF dequantisation, 3GPP TS 26.190 §5.2.7 and §6.1, in fixed point.
//!
//! The decoder's entry point into the spectral path: quantiser indices in,
//! sixteen ISFs out. Everything downstream — [`super::isf::isf_to_isp`],
//! [`super::isf::interpolate_isp`], [`super::isp_to_lp`] — is deterministic
//! arithmetic on the result.
//!
//! # Predictive, and therefore stateful
//!
//! The quantiser codes the ISF vector after subtracting a long-term mean *and*
//! a third of the previous frame's residual. That prediction is what buys the
//! bit rate, and it is also why this module carries state: a frame's output
//! depends on the frame before it. Decoding a frame in isolation gives the
//! wrong answer, and a state update that is subtly wrong produces a decoder
//! that drifts away from its encoder over seconds rather than failing outright.
//!
//! # Two rates
//!
//! 46 bits normally, 36 for the 6.60 kbit/s mode, which cannot afford the
//! wider refinement. Both share the first stage and differ only in the second.
//!
//! # Erasures
//!
//! On a bad frame there are no indices to decode, so the ISFs are reconstructed
//! from history instead: the previous frame's ISFs pulled slightly toward a
//! running mean. The residual state is then *estimated* rather than stored,
//! and halved, so a burst of losses decays toward the mean rather than
//! repeating one spectrum indefinitely.

use super::autocorr::LP_ORDER;
use super::isf_codebooks::{
    DICO1, DICO2, DICO21, DICO21_36B, DICO22, DICO22_36B, DICO23, DICO23_36B, DICO24, DICO25,
    MEAN_ISF,
};
use crate::fixed_point::arith::{add, mult, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult};
use crate::fixed_point::shift::shr;
use crate::fixed_point::types::{DspContext, Word16};

/// Prediction factor, 1/3 in Q15.
const MU: Word16 = Word16(10923);

/// Weight given to the previous frame's ISFs during concealment, 0.9 in Q15.
const ALPHA: Word16 = Word16(29491);

/// The complement of [`ALPHA`]: `32768 - 29491`, 0.1 in Q15.
const ONE_ALPHA: Word16 = Word16(3277);

/// Minimum spacing between adjacent ISFs, Q15 — 50 Hz.
const ISF_GAP: Word16 = Word16(128);

/// How many past frames the concealment mean averages over.
const L_MEANBUF: usize = 3;

/// The decoder's reset-state ISFs: evenly spaced, which is a flat spectrum.
///
/// Exported because the decoder seeds its own previous-frame ISF history from
/// the same values — the stability measure compares against it on frame 0.
pub const ISF_INIT: [i16; LP_ORDER] = [
    1024, 2048, 3072, 4096, 5120, 6144, 7168, 8192, 9216, 10240, 11264, 12288, 13312, 14336, 15360,
    3840,
];

/// Which of the two quantiser rates a frame uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsfQuantizer {
    /// 46 bits, seven indices. Every mode except 6.60 kbit/s.
    Bits46,
    /// 36 bits, five indices. The 6.60 kbit/s mode only.
    Bits36,
}

impl IsfQuantizer {
    /// How many indices this rate consumes.
    #[must_use]
    pub const fn index_count(self) -> usize {
        match self {
            Self::Bits46 => 7,
            Self::Bits36 => 5,
        }
    }
}

/// Carried-over spectral state, one instance per decoder.
#[derive(Debug, Clone)]
pub struct IsfDecoder {
    /// Previous frame's quantised residual, the input to the MA prediction.
    past_isfq: [Word16; LP_ORDER],
    /// Previous frame's decoded ISFs, used to conceal an erasure.
    isf_old: [Word16; LP_ORDER],
    /// The last few decoded frames, averaged into a mean during concealment.
    isf_buf: [[Word16; LP_ORDER]; L_MEANBUF],
}

impl Default for IsfDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl IsfDecoder {
    /// A decoder in its reset state.
    ///
    /// The residual starts at zero, but the ISF history starts at a flat
    /// spectrum rather than zero — a zeroed history would make an erasure in
    /// the first few frames conceal toward a degenerate spectrum.
    #[must_use]
    pub fn new() -> Self {
        let init = ISF_INIT.map(Word16);
        Self {
            past_isfq: [Word16(0); LP_ORDER],
            isf_old: init,
            isf_buf: [init; L_MEANBUF],
        }
    }

    /// Decode one frame's ISFs.
    ///
    /// `indices` must hold [`IsfQuantizer::index_count`] entries. On an
    /// erasure, pass `bad_frame` and the indices are ignored.
    ///
    /// # Panics
    ///
    /// If `indices` is shorter than the quantiser requires, or an index is out
    /// of range for its codebook.
    pub fn decode(
        &mut self,
        quantizer: IsfQuantizer,
        indices: &[u16],
        bad_frame: bool,
    ) -> [Word16; LP_ORDER] {
        let mut ctx = DspContext::default();

        let mut isf = if bad_frame {
            self.conceal(&mut ctx)
        } else {
            assert!(
                indices.len() >= quantizer.index_count(),
                "the {quantizer:?} quantiser needs {} indices, got {}",
                quantizer.index_count(),
                indices.len()
            );
            self.decode_good(&mut ctx, quantizer, indices)
        };

        // Quantisation can leave ISFs too close together or out of order, and
        // either makes the synthesis filter ring. Enforcing a floor here is
        // cheaper and more robust than trusting the codebooks never to.
        reorder_isf(&mut ctx, &mut isf, ISF_GAP);

        self.isf_old = isf;
        isf
    }

    /// Decode from indices, then undo the mean removal and MA prediction.
    fn decode_good(
        &mut self,
        ctx: &mut DspContext,
        quantizer: IsfQuantizer,
        indices: &[u16],
    ) -> [Word16; LP_ORDER] {
        let mut isf = [Word16(0); LP_ORDER];

        // First stage: two vectors covering the low nine and high seven.
        copy_entry(&mut isf[0..9], &DICO1, indices[0], 9);
        copy_entry(&mut isf[9..16], &DICO2, indices[1], 7);

        // Second stage refines in narrower splits, differently per rate.
        match quantizer {
            IsfQuantizer::Bits46 => {
                add_entry(ctx, &mut isf[0..3], &DICO21, indices[2], 3);
                add_entry(ctx, &mut isf[3..6], &DICO22, indices[3], 3);
                add_entry(ctx, &mut isf[6..9], &DICO23, indices[4], 3);
                add_entry(ctx, &mut isf[9..12], &DICO24, indices[5], 3);
                add_entry(ctx, &mut isf[12..16], &DICO25, indices[6], 4);
            }
            IsfQuantizer::Bits36 => {
                add_entry(ctx, &mut isf[0..5], &DICO21_36B, indices[2], 5);
                add_entry(ctx, &mut isf[5..9], &DICO22_36B, indices[3], 4);
                add_entry(ctx, &mut isf[9..16], &DICO23_36B, indices[4], 7);
            }
        }

        // Undo the prediction: add back the mean and a third of the last
        // residual. The residual saved for next time is the value *before*
        // that reconstruction, which is what the encoder also holds.
        for (i, slot) in isf.iter_mut().enumerate() {
            let residual = *slot;
            let with_mean = add(ctx, residual, Word16(MEAN_ISF[i]));
            let predicted = mult(ctx, MU, self.past_isfq[i]);
            *slot = add(ctx, with_mean, predicted);
            self.past_isfq[i] = residual;
        }

        // Only good frames enter the history; concealed ones are not evidence
        // about the talker.
        self.isf_buf.rotate_right(1);
        self.isf_buf[0] = isf;

        isf
    }

    /// Reconstruct ISFs for an erased frame from history.
    fn conceal(&mut self, ctx: &mut DspContext) -> [Word16; LP_ORDER] {
        // A running mean over the long-term mean and the last few frames. The
        // 8192 is 0.25 in Q15 and there are four terms, so this is their
        // average.
        let mut reference = [Word16(0); LP_ORDER];
        for (i, slot) in reference.iter_mut().enumerate() {
            let mut acc = l_mult(ctx, Word16(MEAN_ISF[i]), Word16(8192));
            for past in &self.isf_buf {
                acc = l_mac(ctx, acc, past[i], Word16(8192));
            }
            *slot = round(ctx, acc);
        }

        // Hold the previous spectrum, nudged toward that mean.
        let mut isf = [Word16(0); LP_ORDER];
        for (i, slot) in isf.iter_mut().enumerate() {
            let held = mult(ctx, ALPHA, self.isf_old[i]);
            let pulled = mult(ctx, ONE_ALPHA, reference[i]);
            *slot = add(ctx, held, pulled);
        }

        // There is no transmitted residual to store, so estimate what one
        // would have been and halve it. Halving is what makes a burst of
        // losses decay toward the mean instead of holding one spectrum.
        for i in 0..LP_ORDER {
            let carried = mult(ctx, self.past_isfq[i], MU);
            let predicted = add(ctx, reference[i], carried);
            let estimate = sub(ctx, isf[i], predicted);
            self.past_isfq[i] = shr(ctx, estimate, 1);
        }

        isf
    }
}

/// Copy one codebook entry into a destination split.
fn copy_entry(dst: &mut [Word16], book: &[i16], index: u16, dim: usize) {
    let base = index as usize * dim;
    for (i, slot) in dst.iter_mut().enumerate() {
        *slot = Word16(book[base + i]);
    }
}

/// Add one codebook entry onto a destination split.
fn add_entry(ctx: &mut DspContext, dst: &mut [Word16], book: &[i16], index: u16, dim: usize) {
    let base = index as usize * dim;
    for (i, slot) in dst.iter_mut().enumerate() {
        *slot = add(ctx, *slot, Word16(book[base + i]));
    }
}

/// Force a minimum spacing between adjacent ISFs.
///
/// Note this walks forward and only ever raises values, so it restores
/// ordering as a side effect: an ISF below its predecessor's floor is lifted
/// to it. The last ISF is deliberately left alone.
fn reorder_isf(ctx: &mut DspContext, isf: &mut [Word16; LP_ORDER], min_dist: Word16) {
    let mut floor = min_dist;
    for slot in isf.iter_mut().take(LP_ORDER - 1) {
        if slot.0 < floor.0 {
            *slot = floor;
        }
        floor = add(ctx, *slot, min_dist);
    }
}

#[cfg(test)]
mod tests {
    use super::super::isp_to_lp::tests_support::{block_row, has_block};
    use super::*;

    fn vector(block: &str, label: &str) -> Vec<i16> {
        block_row(block, label)
    }

    /// Replay the oracle's frame sequence and compare every frame.
    fn replay(block: &str, quantizer: IsfQuantizer) {
        assert!(has_block(block), "fixture block {block} missing");
        let mut dec = IsfDecoder::new();
        let mut frames = 0;

        for f in 0.. {
            let label = format!("ind{f}");
            if !super::super::isp_to_lp::tests_support::block_has(block, &label) {
                break;
            }
            frames += 1;

            let indices: Vec<u16> = vector(block, &label)
                .iter()
                .map(|&v| u16::try_from(v).expect("index is non-negative"))
                .collect();
            assert_eq!(
                indices.len(),
                quantizer.index_count(),
                "frame {f}: index count"
            );

            // The oracle marks its final frame bad; everything before is good.
            let next_exists =
                super::super::isp_to_lp::tests_support::block_has(block, &format!("ind{}", f + 1));
            let bad = !next_exists;

            let got = dec.decode(quantizer, &indices, bad);
            let want = vector(block, &format!("isfq{f}"));
            for i in 0..LP_ORDER {
                assert_eq!(
                    got[i].0,
                    want[i],
                    "{block} frame {f}{}: isf[{i}] = {} but the reference gives {}",
                    if bad { " (erased)" } else { "" },
                    got[i].0,
                    want[i]
                );
            }

            // The residual state matters as much as the output: a wrong update
            // shows up only in later frames, so check it directly.
            let want_past = vector(block, &format!("past{f}"));
            for (i, (&got, &want)) in dec.past_isfq.iter().zip(want_past.iter()).enumerate() {
                assert_eq!(
                    got.0, want,
                    "{block} frame {f}: past_isfq[{i}] = {} but the reference gives {want}",
                    got.0
                );
            }
        }

        assert!(
            frames >= 2,
            "{block}: only {frames} frames, prediction untested"
        );
    }

    #[test]
    fn the_46_bit_quantizer_is_bit_exact_against_ts26173() {
        replay("isfdq0", IsfQuantizer::Bits46);
    }

    #[test]
    fn the_36_bit_quantizer_is_bit_exact_against_ts26173() {
        replay("isfdq1", IsfQuantizer::Bits36);
    }

    #[test]
    fn decoded_isfs_keep_their_minimum_spacing() {
        // Ordering with a floor is what keeps the synthesis filter from
        // ringing; it is enforced after every frame, concealed ones included.
        let mut dec = IsfDecoder::new();
        for f in 0..8u16 {
            let indices: Vec<u16> = (0..7).map(|i| (f * 37 + i * 101 + 13) % 32).collect();
            let isf = dec.decode(IsfQuantizer::Bits46, &indices, f % 3 == 2);
            for i in 1..LP_ORDER - 1 {
                assert!(
                    isf[i].0 - isf[i - 1].0 >= ISF_GAP.0,
                    "frame {f}: isf[{i}] and isf[{}] are {} apart",
                    i - 1,
                    isf[i].0 - isf[i - 1].0
                );
            }
        }
    }

    #[test]
    fn sustained_erasure_decays_toward_the_mean_rather_than_holding() {
        // The halving of the estimated residual is what makes a burst decay.
        // Without it a long loss repeats one spectrum indefinitely, which is
        // the classic robotic artefact.
        let mut dec = IsfDecoder::new();
        let indices = [7u16, 19, 3, 11, 29, 5, 13];
        for _ in 0..4 {
            dec.decode(IsfQuantizer::Bits46, &indices, false);
        }

        let mut previous = dec.decode(IsfQuantizer::Bits46, &indices, true);
        let mut first_step = 0i32;
        for n in 0..12 {
            let next = dec.decode(IsfQuantizer::Bits46, &indices, true);
            let step: i32 = (0..LP_ORDER)
                .map(|i| i32::from(next[i].0 - previous[i].0).abs())
                .sum();
            if n == 0 {
                first_step = step;
            } else if n == 11 {
                assert!(
                    step < first_step,
                    "erasure {n} still moves {step}, no less than the first step {first_step}"
                );
            }
            previous = next;
        }
    }

    #[test]
    fn a_clean_frame_after_an_erasure_recovers() {
        // Concealment must not poison the prediction state so badly that the
        // next good frame is wrong; it decodes from its own indices plus a
        // residual, and that residual is what concealment estimated.
        let indices = [7u16, 19, 3, 11, 29, 5, 13];

        let mut clean = IsfDecoder::new();
        for _ in 0..3 {
            clean.decode(IsfQuantizer::Bits46, &indices, false);
        }
        let undisturbed = clean.decode(IsfQuantizer::Bits46, &indices, false);

        let mut disturbed = IsfDecoder::new();
        for _ in 0..3 {
            disturbed.decode(IsfQuantizer::Bits46, &indices, false);
        }
        disturbed.decode(IsfQuantizer::Bits46, &indices, true);
        let recovered = disturbed.decode(IsfQuantizer::Bits46, &indices, false);

        // Not identical — the prediction genuinely differs — but close.
        let drift: i32 = (0..LP_ORDER)
            .map(|i| i32::from(recovered[i].0 - undisturbed[i].0).abs())
            .sum();
        assert!(
            drift < 4000,
            "one erasure left the next good frame {drift} away from an undisturbed decode"
        );
    }

    #[test]
    fn the_two_rates_share_their_first_stage() {
        // Both quantisers use DICO1 and DICO2; only the refinement differs.
        // A mix-up there would be invisible at 46 bits and wrong at 36.
        let mut a = IsfDecoder::new();
        let mut b = IsfDecoder::new();
        let wide = [5u16, 9, 0, 0, 0, 0, 0];
        let narrow = [5u16, 9, 0, 0, 0];

        let from_46 = a.decode(IsfQuantizer::Bits46, &wide, false);
        let from_36 = b.decode(IsfQuantizer::Bits36, &narrow, false);

        // Different refinements, so not equal, but the same neighbourhood.
        let gap: i32 = (0..LP_ORDER)
            .map(|i| i32::from(from_46[i].0 - from_36[i].0).abs())
            .sum();
        assert!(
            gap < 8000,
            "the two rates disagree by {gap} on a shared first stage"
        );
    }
}
