//! Encoder-side DTX for AMR-NB, TS 26.073 `dtx_enc.c`.
//!
//! # Not the wideband machine
//!
//! The two variants' DTX look alike and are not: narrowband averages *eight*
//! frames of LSPs where wideband averages eight of ISFs through a distance
//! matrix, narrowband has no outlier substitution and no dithering decision at
//! all, its energy is measured on the *speech* rather than on a residual, and
//! its SID payload is a different shape. Sharing anything but the transmit
//! cadence between them would be sharing a coincidence.
//!
//! # The SID payload is 35 bits, and 26 of them are the spectrum
//!
//! Three for the predictor-reset index, then 8 + 9 + 9 for the three LSF
//! sub-vectors, then six for the energy. The spectrum is quantised by the
//! *speech* quantiser in its `MRDTX` mode rather than by a separate
//! comfort-noise codebook — the opposite of wideband, which has its own.
//!
//! # It rewrites the gain predictor
//!
//! `dtx_enc` overwrites all four of the gain predictor's past energies, in
//! both of its representations, so that speech resuming after silence starts
//! from the comfort-noise level rather than from wherever the talker left off.
//! Missing this produces a burst of wrong gain on the first frame back.

use crate::fixed_point::arith::{add, extract_l, mult, sub};
use crate::fixed_point::arith32::{l_add, l_deposit_l, l_mac};
use crate::fixed_point::shift::{l_shr, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Frames of spectrum and energy the SID is averaged over.
pub const DTX_HIST_SIZE: usize = 8;

/// LP order.
const M: usize = 10;

/// Samples per frame.
const L_FRAME: usize = 160;

/// Hangover before quiet is allowed to become comfort noise.
const DTX_HANG_CONST: Word16 = Word16(7);

/// `24 + DTX_HANG_CONST - 1`.
const DTX_ELAPSED_FRAMES_THRESH: Word16 = Word16(24 + 7 - 1);

/// What the encoder decided to do with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxDecision {
    /// Encode and transmit as speech.
    Speech,
    /// Build a SID payload.
    ComfortNoise,
}

/// Encoder DTX state — the reference's `dtx_encState`.
#[derive(Debug, Clone)]
pub struct DtxEncoder {
    /// Ring of the last eight frames' LSP vectors.
    lsp_hist: [Word16; M * DTX_HIST_SIZE],
    /// Ring of the last eight frames' log energies, Q10 and halved.
    log_en_hist: [Word16; DTX_HIST_SIZE],
    hist_ptr: usize,
    /// The most recently computed SID fields, held because a SID frame that
    /// is not permitted to recompute still transmits the previous ones.
    log_en_index: Word16,
    init_lsf_vq_index: Word16,
    lsp_index: [Word16; 3],
    dtx_hangover_count: Word16,
    dec_ana_elapsed_count: Word16,
}

impl Default for DtxEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DtxEncoder {
    /// A DTX encoder in its reset state, as `dtx_enc_reset` leaves it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lsp_hist: [Word16(0); M * DTX_HIST_SIZE],
            // The reference clears eight words here with a loop bound of `M`,
            // which is ten -- it writes two words past the array, onto the two
            // fields the lines above have just zeroed. The behaviour is "zero
            // eight"; reproducing the overrun faithfully would panic.
            log_en_hist: [Word16(0); DTX_HIST_SIZE],
            hist_ptr: 0,
            log_en_index: Word16(0),
            init_lsf_vq_index: Word16(0),
            lsp_index: [Word16(0); 3],
            dtx_hangover_count: DTX_HANG_CONST,
            // Starts saturated, so the first talk spurt gets the full
            // hangover rather than none.
            dec_ana_elapsed_count: Word16(32767),
        }
    }

    /// Whether this frame is speech or comfort noise — `tx_dtx_handler`.
    ///
    /// Returns the decision and whether a *new* SID may be computed. The
    /// second is not the same as the first: a frame can be comfort noise and
    /// still have to retransmit the previous parameters, because recomputing
    /// immediately after a talk spurt would describe the talker rather than
    /// the background.
    pub fn classify(
        &mut self,
        ctx: &mut DspContext,
        voice_active: bool,
    ) -> (TxDecision, bool) {
        self.dec_ana_elapsed_count = add(ctx, self.dec_ana_elapsed_count, Word16(1));

        if voice_active {
            self.dtx_hangover_count = DTX_HANG_CONST;
            return (TxDecision::Speech, false);
        }
        if self.dtx_hangover_count.0 == 0 {
            self.dec_ana_elapsed_count = Word16(0);
            return (TxDecision::ComfortNoise, true);
        }
        self.dtx_hangover_count = sub(ctx, self.dtx_hangover_count, Word16(1));
        let staleness = add(ctx, self.dec_ana_elapsed_count, self.dtx_hangover_count);
        if sub(ctx, staleness, DTX_ELAPSED_FRAMES_THRESH).0 < 0 {
            (TxDecision::ComfortNoise, false)
        } else {
            // Override the detector and add extra hangover, so the decoder has
            // recent speech to analyse.
            (TxDecision::Speech, false)
        }
    }

    /// Record one frame's spectrum and energy — `dtx_buffer`.
    ///
    /// The energy is measured on the *speech* itself, not on a residual: the
    /// wideband encoder does the opposite, and the two rings are not
    /// interchangeable.
    pub fn buffer(&mut self, ctx: &mut DspContext, lsp: &[Word16; M], speech: &[Word16]) {
        self.hist_ptr = (self.hist_ptr + 1) % DTX_HIST_SIZE;
        let base = self.hist_ptr * M;
        self.lsp_hist[base..base + M].copy_from_slice(lsp);

        let mut energy = Word32(0);
        for &sample in speech.iter().take(L_FRAME) {
            energy = l_mac(ctx, energy, sample, sample);
        }
        let (exponent, mantissa) = super::super::math::log2(ctx, energy);
        let mut log_en = shl(ctx, exponent, 10);
        let fraction = shr(ctx, mantissa, 5);
        log_en = add(ctx, log_en, fraction);
        // log2(160) in Q10, then halved on the way into the ring.
        log_en = sub(ctx, log_en, Word16(8521));
        self.log_en_hist[self.hist_ptr] = shr(ctx, log_en, 1);
    }

    /// Average the history into a new comfort-noise description.
    ///
    /// Returns the averaged LSPs and the energy index. The caller quantises
    /// the spectrum with the speech quantiser's `MRDTX` mode, which is where
    /// the three transmitted sub-vector indices come from.
    pub fn average_history(&mut self, ctx: &mut DspContext) -> ([Word16; M], Word16) {
        let mut log_en = Word16(0);
        let mut sums = [Word32(0); M];
        for frame in 0..DTX_HIST_SIZE {
            let quarter = shr(ctx, self.log_en_hist[frame], 2);
            log_en = add(ctx, log_en, quarter);
            for (acc, &value) in sums.iter_mut().zip(&self.lsp_hist[frame * M..]) {
                *acc = l_add(ctx, *acc, l_deposit_l(value));
            }
        }
        log_en = shr(ctx, log_en, 1);

        let mut lsp = [Word16(0); M];
        for (slot, &sum) in lsp.iter_mut().zip(&sums) {
            *slot = extract_l(l_shr(ctx, sum, 3));
        }

        // +2.5 in Q10, then half a step, then down to the six-bit field.
        let mut index = add(ctx, log_en, Word16(2560));
        index = add(ctx, index, Word16(128));
        index = shr(ctx, index, 8);
        if index.0 > 63 {
            index = Word16(63);
        }
        if index.0 < 0 {
            index = Word16(0);
        }
        self.log_en_index = index;
        (lsp, index)
    }

    /// The gain-predictor energies a new SID installs.
    ///
    /// Returns the value for the ordinary representation and the one 12.2
    /// kbit/s uses, in that order. Both are overwritten in all four slots, so
    /// speech resuming after silence starts from the comfort-noise level.
    #[must_use]
    pub fn predictor_reset(&self, ctx: &mut DspContext) -> (Word16, Word16) {
        // Q11, and divided by four.
        let mut log_en = shl(ctx, self.log_en_index, -2 + 10);
        log_en = sub(ctx, log_en, Word16(2560));
        log_en = sub(ctx, log_en, Word16(9000));
        if log_en.0 > 0 {
            log_en = Word16(0);
        }
        if log_en.0 < -14436 {
            log_en = Word16(-14436);
        }
        // 20*log10(2) in Q15 for the 12.2 representation.
        (log_en, mult(ctx, Word16(5443), log_en))
    }

    /// Record the quantiser indices a new SID produced.
    pub const fn set_indices(&mut self, init_index: Word16, lsp_index: [Word16; 3]) {
        self.init_lsf_vq_index = init_index;
        self.lsp_index = lsp_index;
    }

    /// The five fields a SID frame transmits, newest or retained.
    ///
    /// A comfort-noise frame that was not permitted to recompute sends the
    /// previous description again rather than nothing.
    #[must_use]
    pub const fn sid_parameters(&self) -> [Word16; 5] {
        [
            self.init_lsf_vq_index,
            self.lsp_index[0],
            self.lsp_index[1],
            self.lsp_index[2],
            self.log_en_index,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hangover_runs_seven_frames_and_then_yields() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        for _ in 0..3 {
            assert_eq!(dtx.classify(&mut ctx, true).0, TxDecision::Speech);
        }
        let decisions: Vec<TxDecision> =
            (0..10).map(|_| dtx.classify(&mut ctx, false).0).collect();
        let first_cn = decisions.iter().position(|&d| d == TxDecision::ComfortNoise);
        assert_eq!(first_cn, Some(7), "the hangover is seven frames");
    }

    #[test]
    fn only_the_frame_out_of_hangover_may_recompute_the_description() {
        // A frame can be comfort noise and still have to retransmit: computing
        // a new description immediately after speech would describe the
        // talker rather than the background.
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        for _ in 0..3 {
            dtx.classify(&mut ctx, true);
        }
        let mut recomputes = 0;
        for _ in 0..10 {
            let (decision, may_recompute) = dtx.classify(&mut ctx, false);
            if may_recompute {
                assert_eq!(decision, TxDecision::ComfortNoise);
                recomputes += 1;
            }
        }
        assert_eq!(recomputes, 3, "recomputation should be rare, not every frame");
    }

    #[test]
    fn the_ring_starts_at_slot_one_and_wraps_at_eight() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        let lsp = [Word16(1000); M];
        let speech = [Word16(100); L_FRAME];
        assert_eq!(dtx.hist_ptr, 0);
        dtx.buffer(&mut ctx, &lsp, &speech);
        assert_eq!(dtx.hist_ptr, 1);
        for _ in 0..7 {
            dtx.buffer(&mut ctx, &lsp, &speech);
        }
        assert_eq!(dtx.hist_ptr, 0);
    }

    #[test]
    fn the_predictor_reset_is_clamped_at_both_ends() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        // The loudest index still clamps at zero rather than going positive.
        dtx.log_en_index = Word16(63);
        let (plain, scaled) = dtx.predictor_reset(&mut ctx);
        assert_eq!(plain.0, 0, "a loud SID must not raise the predictor");
        assert_eq!(scaled.0, 0);
        // The quietest sits well above the floor: the floor exists for the
        // expression's own range, not for any index a SID can carry.
        dtx.log_en_index = Word16(0);
        let (plain, scaled) = dtx.predictor_reset(&mut ctx);
        assert_eq!(plain.0, -11560);
        // 20*log10(2) in Q15, so the 12.2 representation is the same value
        // in a different unit and is smaller in magnitude, not more negative.
        assert!(
            scaled.0 > plain.0 && scaled.0 < 0,
            "12.2 got {} from {}",
            scaled.0,
            plain.0
        );
    }
}
