//! Encoder-side DTX for AMR-WB, TS 26.173 `dtx.c`.
//!
//! # What runs, and how often
//!
//! Discontinuous transmission is not "stop encoding when the VAD says quiet".
//! Every non-speech frame still runs the full front end and still buffers its
//! spectrum and energy; the encoder builds a complete SID payload on *every*
//! frame it marks `MRDTX`, and roughly seven in eight of those are then thrown
//! away by the frame-type layer and sent as `NO_DATA`. The bits of a transmitted
//! SID therefore depend on every discarded frame before it, through
//! its ring of buffered frames, the distance matrix and the noise seed. An
//! implementation that computed the SID lazily — only when one is about to go
//! out — would produce different bits.
//!
//! # The two `dtx_buffer` call sites disagree, deliberately
//!
//! The reference buffers a frame's spectrum and residual energy from two
//! different places, and they do not agree on their inputs:
//!
//! | | on an `MRDTX` frame | on a speech-mode frame the VAD called quiet |
//! |---|---|---|
//! | predictor | the **unquantised** interpolated LP of subframe 4 | the **quantised** per-subframe LP |
//! | `Residu` calls | one, spanning all 256 samples | four, one per subframe |
//! | ISFs | unquantised | quantised, overwritten in place by the quantiser |
//!
//! Both feed the same ring. Factoring them into one helper that "picks the
//! residual" loses the distinction, and the resulting SID is plausible and
//! wrong several frames later. [`DtxEncoder::buffer`] therefore takes the
//! already-computed energy and ISFs, and the two call sites in the encoder
//! compute them separately.

use crate::codecs::amr::wb::lp::autocorr::LP_ORDER;
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_add, l_mac, l_mult, l_sub};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Frames of spectrum and energy the SID is averaged over.
pub const DTX_HIST_SIZE: usize = 8;

/// Hangover before a talk spurt is allowed to become comfort noise.
const DTX_HANG_CONST: Word16 = Word16(7);

/// `24 + DTX_HANG_CONST - 1`: how stale the decoder's own analysis may get.
const DTX_ELAPSED_FRAMES_THRESH: Word16 = Word16(24 + 7 - 1);

/// Replace an outlier only when it is this much further out than the most
/// central frame. 14564 is 1/2.25 in Q15.
const INV_MED_THRESH: Word16 = Word16(14564);

/// Summed absolute deviation of the eight log energies, above which the
/// decoder is told to dither. Q7, so 180 is about 1.4 in log2 units.
const GAIN_THR: Word16 = Word16(180);

/// Per-mode energy offset subtracted in `dtx_buffer`, Q7, indexed by the
/// *speech* mode. Never by `MRDTX`: the reference snapshots the requested mode
/// before the DTX handler can overwrite it.
const EN_ADJUST: [i16; 9] = [230, 179, 141, 128, 122, 115, 115, 115, 115];

/// The reset ISF vector, `cod_main.c`'s `isf_init`.
const ISF_INIT: [i16; LP_ORDER] = [
    1024, 2048, 3072, 4096, 5120, 6144, 7168, 8192, 9216, 10240, 11264, 12288, 13312, 14336,
    15360, 3840,
];

/// Where row `r` of the flattened upper-triangular distance matrix begins.
///
/// `D` holds the 8x8 inter-frame distances above the diagonal, rows of length
/// 7, 6, 5, 4, 3, 2, 1 laid end to end. Indices are by *age*: 0 is the newest
/// frame, 7 the oldest.
const fn row_offset(row: usize) -> usize {
    row * (15 - row) / 2
}

/// Total entries in the flattened matrix.
const D_LEN: usize = 28;

/// What the encoder decided to do with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxDecision {
    /// Encode and transmit as speech in the requested mode.
    Speech,
    /// Build a SID payload. Whether it is actually transmitted is the frame
    /// type layer's decision, not this one's.
    ComfortNoise,
}

/// Encoder DTX state — the reference's `dtx_encState`.
#[derive(Debug, Clone)]
pub struct DtxEncoder {
    /// Ring of the last eight frames' ISF vectors, newest at `hist_ptr`.
    isf_hist: [Word16; LP_ORDER * DTX_HIST_SIZE],
    /// Ring of the last eight frames' log energies, Q7.
    log_en_hist: [Word16; DTX_HIST_SIZE],
    /// Index of the newest slot in both rings.
    hist_ptr: usize,
    /// The energy index most recently written into a SID.
    log_en_index: Word16,
    /// Noise generator for the comfort-noise excitation. Separate from the
    /// high band's, and stepped 256 times per SID frame.
    cng_seed: Word16,
    /// Frames of hangover left before quiet becomes comfort noise.
    dtx_hangover_count: Word16,
    /// Frames since the decoder last had a chance to analyse real speech.
    /// Starts saturated, so the first talk spurt gets the full hangover.
    dec_ana_elapsed_count: Word16,
    /// Flattened upper-triangular ISF distance matrix, twice the true squared
    /// distances (`L_mac` doubles, uniformly).
    distances: [Word32; D_LEN],
    /// Row sums of `distances`, by age.
    sum_distances: [Word32; DTX_HIST_SIZE],
}

impl Default for DtxEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DtxEncoder {
    /// A DTX encoder in its reset state, as `dtx_enc_reset` leaves it.
    #[must_use]
    pub fn new() -> Self {
        let mut isf_hist = [Word16(0); LP_ORDER * DTX_HIST_SIZE];
        for slot in isf_hist.chunks_exact_mut(LP_ORDER) {
            for (out, &value) in slot.iter_mut().zip(&ISF_INIT) {
                *out = Word16(value);
            }
        }
        Self {
            isf_hist,
            log_en_hist: [Word16(0); DTX_HIST_SIZE],
            hist_ptr: 0,
            log_en_index: Word16(0),
            // The reference's RANDOM_INITSEED, the same value the high-band
            // noise generator starts from -- a separate register at the same
            // seed, not a shared one.
            cng_seed: Word16(21845),
            dtx_hangover_count: DTX_HANG_CONST,
            dec_ana_elapsed_count: Word16(32767),
            distances: [Word32(0); D_LEN],
            // The reference leaves `sumD[7]` uninitialised -- its reset loop
            // stops at DTX_HIST_SIZE - 1. It cannot escape: the first
            // `find_frame_indices` overwrites it from `sumD[6]` before any
            // reader. Zeroing all eight is exact, and was confirmed by filling
            // the reference's whole struct with 0xAA before reset and getting a
            // bit-identical SID sequence.
            sum_distances: [Word32(0); DTX_HIST_SIZE],
        }
    }

    /// Whether this frame should be coded as speech or as comfort noise —
    /// `tx_dtx_handler`.
    ///
    /// Runs before the VAD flag is written, and on a comfort-noise decision the
    /// caller must switch to the 35-bit SID frame size.
    ///
    /// The behaviour that surprises: after a long talk spurt the first *seven*
    /// quiet frames still go out as speech — extra hangover that lets the
    /// decoder run its own backwards noise analysis — and the eighth becomes
    /// comfort noise. But after a *short* burst inside silence, the very next
    /// quiet frame is comfort noise with no hangover at all, because
    /// `dec_ana_elapsed_count` was reset by the DTX the burst interrupted.
    pub fn classify(&mut self, ctx: &mut DspContext, voice_active: bool) -> TxDecision {
        self.dec_ana_elapsed_count = add(ctx, self.dec_ana_elapsed_count, Word16(1));

        if voice_active {
            self.dtx_hangover_count = DTX_HANG_CONST;
            return TxDecision::Speech;
        }
        if self.dtx_hangover_count.0 == 0 {
            self.dec_ana_elapsed_count = Word16(0);
            return TxDecision::ComfortNoise;
        }
        self.dtx_hangover_count = sub(ctx, self.dtx_hangover_count, Word16(1));
        let staleness = add(ctx, self.dec_ana_elapsed_count, self.dtx_hangover_count);
        if sub(ctx, staleness, DTX_ELAPSED_FRAMES_THRESH).0 < 0 {
            TxDecision::ComfortNoise
        } else {
            TxDecision::Speech
        }
    }

    /// The hangover counter, which 23.85 kbit/s reads to scale its high-band
    /// gain.
    ///
    /// The only rate that does, and the reason mode 8's bitstream differs
    /// between DTX on and off even on ordinary speech frames.
    #[must_use]
    pub const fn hangover_count(&self) -> Word16 {
        self.dtx_hangover_count
    }

    /// Record one frame's spectrum and residual energy — `dtx_buffer`.
    ///
    /// `energy` is the residual energy already computed by the caller, because
    /// the two call sites compute it differently; see the module header.
    /// `mode_index` is the *speech* mode originally requested, never the SID
    /// pseudo-mode.
    pub fn buffer(
        &mut self,
        ctx: &mut DspContext,
        isf: &[Word16; LP_ORDER],
        energy: Word32,
        mode_index: usize,
    ) {
        // Pre-increment: the newest frame lands at the *new* pointer, and that
        // pointer names the newest slot for the rest of the frame. From reset
        // the first buffered frame goes to slot 1, and slot 0 keeps its reset
        // contents until the eighth call wraps.
        self.hist_ptr = (self.hist_ptr + 1) % DTX_HIST_SIZE;
        let base = self.hist_ptr * LP_ORDER;
        self.isf_hist[base..base + LP_ORDER].copy_from_slice(isf);

        let (exponent, mantissa) = super::super::math::log2(ctx, energy);
        let mut log_en = shl(ctx, Word16(exponent), 7);
        let mantissa_q7 = shr(ctx, mantissa, 8);
        log_en = add(ctx, log_en, mantissa_q7);
        // 1024 is log2(256) in Q7 -- dividing the frame energy by its length --
        // and the per-mode offset comes off with it.
        let offset = add(ctx, Word16(1024), Word16(EN_ADJUST[mode_index]));
        self.log_en_hist[self.hist_ptr] = sub(ctx, log_en, offset);
    }

    /// Build the SID parameters for this frame — the payload half of `dtx_enc`.
    ///
    /// Returns the five ISF indices, the energy index, the dithering flag, and
    /// the decoded comfort-noise spectrum the caller needs for its local
    /// synthesis.
    pub fn build_sid(&mut self, ctx: &mut DspContext) -> SidParameters {
        let mut log_en = Word16(0);
        for &value in &self.log_en_hist {
            log_en = add(ctx, log_en, value);
        }

        let order = self.find_frame_indices(ctx);
        let averaged = self.average_isf_history(ctx, &order);
        let mut isf = [Word16(0); LP_ORDER];
        for (slot, &value) in isf.iter_mut().zip(&averaged) {
            *slot = extract_l(l_shr(ctx, value, 3));
        }

        // Sum of eight Q7 values is Q10; >> 2 makes it Q8, then +2.0 and
        // x2.625 land it in Q6 for the six-bit field.
        log_en = shr(ctx, log_en, 2);
        log_en = add(ctx, log_en, Word16(512));
        log_en = mult(ctx, log_en, Word16(21504));
        let mut index = shr(ctx, log_en, 6);
        if index.0 > 63 {
            index = Word16(63);
        }
        if index.0 < 0 {
            index = Word16(0);
        }
        self.log_en_index = index;

        let (isf_indices, decoded) = super::super::isf_noise::quantise(ctx, &isf);
        let dither = self.dithering_control(ctx);

        SidParameters {
            isf_indices,
            energy_index: index,
            dither,
            spectrum: decoded,
        }
    }

    /// The comfort-noise excitation for one frame, 256 samples.
    ///
    /// Not transmitted — it drives the encoder's own synthesis, and therefore
    /// the high-band and de-emphasis memories the next speech frame inherits.
    /// The generator is the same LCG as the high band's but a *separate*
    /// register, stepped 256 times here and shifted right by 4 rather than 3.
    pub fn excitation(&mut self, ctx: &mut DspContext, energy_index: Word16) -> [Word16; 256] {
        let mut log_en = shl(ctx, energy_index, 9);
        log_en = mult(ctx, log_en, Word16(12483));
        let integer = shr(ctx, log_en, 10);
        let exponent = add(ctx, integer, Word16(15));
        let mantissa = shl(ctx, Word16(log_en.0 & 0x3FF), 5);
        let level32 = super::super::math::pow2(ctx, exponent.0, mantissa);
        let shift = norm_l(level32);
        let level = extract_h(l_shl(ctx, level32, shift));
        let level_exp = sub(ctx, Word16(15), Word16(shift));

        let mut excitation = [Word16(0); 256];
        for slot in &mut excitation {
                let sample = self.next_noise(ctx);
            *slot = shr(ctx, sample, 4);
        }

        let measured = super::super::math::dot_product12(ctx, &excitation, &excitation);
        let (energy, energy_exp) = super::super::math::isqrt_n(ctx, measured);
        let gain = mult(ctx, level, extract_h(energy));
        // +4 scales by sqrt(256), the length the energy was measured over.
        let combined = add(ctx, level_exp, Word16(energy_exp));
        let total = add(ctx, combined, Word16(4));
        for slot in &mut excitation {
            let scaled = mult(ctx, *slot, gain);
            *slot = shl(ctx, scaled, total.0);
        }
        excitation
    }

    /// One step of the comfort-noise generator.
    fn next_noise(&mut self, ctx: &mut DspContext) -> Word16 {
        let product = l_mult(ctx, self.cng_seed, Word16(31821));
        let halved = l_shr(ctx, product, 1);
        self.cng_seed = extract_l(l_add(ctx, halved, Word32(13849)));
        self.cng_seed
    }

    /// Tell the decoder whether the background is moving enough to need
    /// dithering — `dithering_control`.
    ///
    /// Two independent non-stationarity tests, OR'd: spectral movement, from
    /// the whole distance matrix, and energy movement, from the spread of the
    /// eight log energies. Read *after* `find_frame_indices` has folded this
    /// frame into the sums.
    fn dithering_control(&self, ctx: &mut DspContext) -> bool {
        let mut spectral = Word32(0);
        for &value in &self.sum_distances {
            spectral = l_add(ctx, spectral, value);
        }
        if l_shr(ctx, spectral, 26).0 > 0 {
            return true;
        }

        let mut mean = Word16(0);
        for &value in &self.log_en_hist {
            mean = add(ctx, mean, value);
        }
        mean = shr(ctx, mean, 3);
        let mut spread = Word16(0);
        for &value in &self.log_en_hist {
            let deviation = sub(ctx, value, mean);
            let magnitude = abs_s(ctx, deviation);
            spread = add(ctx, spread, magnitude);
        }
        spread.0 > GAIN_THR.0
    }

    /// Update the distance matrix and pick the outlier, second outlier and most
    /// central frame — `find_frame_indices`.
    ///
    /// Returns three *slot* indices, with `None` where the reference writes its
    /// −1 sentinel meaning "this frame is not enough of an outlier to replace".
    fn find_frame_indices(&mut self, ctx: &mut DspContext) -> [Option<usize>; 3] {
        // Drop the oldest frame out of every row sum. `row_offset(r) + 6 - r`
        // is the last entry of row r, which is that row's distance to the
        // frame about to fall off the end.
        for row in 0..DTX_HIST_SIZE - 1 {
            let last = self.distances[row_offset(row) + 6 - row];
            self.sum_distances[row] = l_sub(ctx, self.sum_distances[row], last);
        }

        for row in (1..DTX_HIST_SIZE).rev() {
            self.sum_distances[row] = self.sum_distances[row - 1];
        }
        self.sum_distances[0] = Word32(0);

        // Age every row by one: row r becomes row r+1, losing its last entry.
        // The reference does this with a descending loop whose stride grows
        // inside the loop body; this is the same permutation stated directly,
        // and the offsets are asserted in a test.
        for row in (0..DTX_HIST_SIZE - 2).rev() {
            for column in 0..=(5 - row) {
                self.distances[row_offset(row + 1) + column] =
                    self.distances[row_offset(row) + column];
            }
        }

        // Recompute row 0: the newest frame against each older one.
        let newest = self.hist_ptr * LP_ORDER;
        let mut slot = self.hist_ptr;
        for age in 1..DTX_HIST_SIZE {
            slot = if slot == 0 { DTX_HIST_SIZE - 1 } else { slot - 1 };
            let other = slot * LP_ORDER;
            let mut acc = Word32(0);
            for i in 0..LP_ORDER {
                let error = sub(ctx, self.isf_hist[newest + i], self.isf_hist[other + i]);
                acc = l_mac(ctx, acc, error, error);
            }
            self.distances[age - 1] = acc;
            self.sum_distances[0] = l_add(ctx, self.sum_distances[0], acc);
            self.sum_distances[age] = l_add(ctx, self.sum_distances[age], acc);
        }

        // The furthest-out frame, the most central one, and the runner-up --
        // all by age, with strict comparisons so the earliest wins a tie.
        let mut furthest = 0usize;
        let mut central = 0usize;
        let mut max = self.sum_distances[0];
        let mut min = self.sum_distances[0];
        for age in 1..DTX_HIST_SIZE {
            if l_sub(ctx, self.sum_distances[age], max).0 > 0 {
                furthest = age;
                max = self.sum_distances[age];
            }
            if l_sub(ctx, self.sum_distances[age], min).0 < 0 {
                central = age;
                min = self.sum_distances[age];
            }
        }
        let mut second = usize::MAX;
        let mut max2 = Word32(-2_147_483_647);
        for age in 0..DTX_HIST_SIZE {
            if age != furthest && l_sub(ctx, self.sum_distances[age], max2).0 > 0 {
                second = age;
                max2 = self.sum_distances[age];
            }
        }

        // Age to slot, then apply the sentinels -- in that order, as the
        // reference does, so the exclusion above compared ages.
        let to_slot = |age: usize| (self.hist_ptr + DTX_HIST_SIZE - age) % DTX_HIST_SIZE;
        let mut result = [
            Some(to_slot(furthest)),
            if second == usize::MAX {
                None
            } else {
                Some(to_slot(second))
            },
            Some(to_slot(central)),
        ];

        let shift = norm_l(max);
        let max = l_shl(ctx, max, shift);
        let min = l_shl(ctx, min, shift);
        let max2 = l_shl(ctx, max2, shift);
        // `L_mult` doubles, so this is "max / 2.25 > min": replace an outlier
        // only when it is more than 2.25 times further out than the centre.
        let scaled_max = round(ctx, max);
        let threshold_max = l_mult(ctx, scaled_max, INV_MED_THRESH);
        if l_sub(ctx, threshold_max, min).0 <= 0 {
            result[0] = None;
        }
        let scaled_second = round(ctx, max2);
        let threshold_second = l_mult(ctx, scaled_second, INV_MED_THRESH);
        if l_sub(ctx, threshold_second, min).0 <= 0 {
            result[1] = None;
        }
        result
    }

    /// Mean of the eight buffered spectra, with up to two outliers replaced by
    /// the most central frame — `aver_isf_history`.
    ///
    /// The substitution is for the mean only: the reference saves the replaced
    /// rows and restores them afterwards, so the history is left untouched.
    fn average_isf_history(&self, ctx: &mut DspContext, order: &[Option<usize>; 3]) -> [Word32; LP_ORDER] {
        let central = order[2].expect("the most central frame is never a sentinel");
        let mut view = self.isf_hist;
        for slot in order.iter().take(2).flatten() {
            let (from, to) = (central * LP_ORDER, slot * LP_ORDER);
            view[to..to + LP_ORDER].copy_from_slice(&self.isf_hist[from..from + LP_ORDER]);
        }

        let mut sum = [Word32(0); LP_ORDER];
        for frame in view.chunks_exact(LP_ORDER) {
            for (acc, &value) in sum.iter_mut().zip(frame) {
                *acc = l_add(ctx, *acc, Word32(i32::from(value.0)));
            }
        }
        sum
    }
}

/// Everything a SID frame carries, before it is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidParameters {
    /// The five comfort-noise ISF codebook indices, 6/6/6/5/5 bits.
    pub isf_indices: [u16; 5],
    /// The quantised frame energy, 6 bits.
    pub energy_index: Word16,
    /// Whether the decoder should dither its comfort noise, 1 bit.
    pub dither: bool,
    /// The decoded spectrum, for the encoder's own synthesis. Not transmitted.
    pub spectrum: [Word16; LP_ORDER],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_row_offsets_tile_the_triangular_matrix() {
        // The flattened matrix is rows of 7, 6, 5, 4, 3, 2, 1. If `row_offset`
        // and those lengths ever disagree the shift silently reads a
        // neighbouring row, which produces plausible distances.
        let mut expected = 0;
        for row in 0..DTX_HIST_SIZE - 1 {
            assert_eq!(row_offset(row), expected, "row {row}");
            expected += 7 - row;
        }
        assert_eq!(expected, D_LEN);
    }

    #[test]
    fn the_matrix_shift_matches_the_references_stride_walk() {
        // The reference ages the matrix with a descending loop whose stride is
        // incremented inside the body. This asserts the direct permutation
        // above produces the same result as transcribing that loop literally.
        let mut direct: [i32; D_LEN] =
            std::array::from_fn(|i| i32::try_from(i).expect("D_LEN is 28"));
        for row in (0..DTX_HIST_SIZE - 2).rev() {
            for column in 0..=(5 - row) {
                direct[row_offset(row + 1) + column] = direct[row_offset(row) + column];
            }
        }

        let mut literal: [i32; D_LEN] =
            std::array::from_fn(|i| i32::try_from(i).expect("D_LEN is 28"));
        let mut i: i32 = 27;
        let mut tmp: i32 = 0;
        while i >= 12 {
            tmp += 1;
            let mut j = tmp;
            while j > 0 {
                let (dst, src) = (
                    usize::try_from(i - j + 1).expect("in range"),
                    usize::try_from(i - j - tmp).expect("in range"),
                );
                literal[dst] = literal[src];
                j -= 1;
            }
            i -= tmp;
        }

        assert_eq!(direct, literal, "the closed form and the stride walk diverge");

        // And rows 1 onward against what the reference's own instrumented run
        // printed for this exact input. Row 0 is excluded because the caller
        // overwrites it immediately afterwards, which is why the reference's
        // printout shows it as zeros.
        assert_eq!(
            &direct[row_offset(1)..],
            &[0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 13, 14, 15, 16, 18, 19, 20, 22, 23, 25][..],
            "the shifted rows do not match the reference's measured output"
        );
    }

    /// The same input sequence the reference probe was driven with.
    ///
    /// Regenerated rather than read back from the fixture, so a fixture that
    /// lost its inputs cannot silently agree with itself. The first sixteen
    /// frames are stationary on purpose: without them the dithering flag never
    /// leaves 1 and the test says nothing about it.
    fn probe_input(frame: usize, lcg: &mut u32) -> ([Word16; LP_ORDER], Word32) {
        const MEAN_NS: [i16; LP_ORDER] = [
            478, 1100, 2213, 3267, 4219, 5222, 6198, 7240, 8229, 9153, 10098, 11108, 12144,
            13184, 14165, 3803,
        ];
        let step = |lcg: &mut u32| {
            *lcg = lcg.wrapping_mul(1_103_515_245).wrapping_add(12345);
            *lcg
        };
        let mut isf = [Word16(0); LP_ORDER];
        if frame < 16 {
            for (slot, &mean) in isf.iter_mut().zip(&MEAN_NS) {
                *slot = Word16(mean);
            }
            return (isf, Word32(300_000));
        }
        for (slot, &mean) in isf.iter_mut().zip(&MEAN_NS) {
            let raw = step(lcg);
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            let jitter = (((raw >> 18) & 0x3FF) as i32 - 512) as i16
                + i16::try_from(frame * 11).expect("frame count is small");
            *slot = Word16(mean.wrapping_add(jitter));
        }
        let raw = step(lcg);
        #[allow(clippy::cast_possible_wrap)]
        let energy = (1i32 << (8 + (frame % 12))) + ((raw >> 20) & 0x3FF) as i32;
        (isf, Word32(energy))
    }

    /// Against TS 26.173's own `dtx_buffer` and `dtx_enc`, frame by frame.
    ///
    /// This is the test that decides whether the ring, the distance matrix,
    /// the outlier substitution, the energy index, the dithering decision and
    /// the comfort-noise generator are all right — every one of them feeds the
    /// five ISF indices, and a single wrong step changes them.
    #[test]
    fn sid_parameters_and_excitation_match_the_reference() {
        let text = include_str!("../../testdata/wb_dtx_enc_vectors.txt");
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        let mut lcg: u32 = 24680;
        let mut compared = 0usize;
        let mut rows = 0usize;
        let mut isf_indices_seen = std::collections::HashSet::new();
        let mut energies_seen = std::collections::HashSet::new();
        let mut dithers_seen = std::collections::HashSet::new();

        for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let fields: Vec<&str> = line.split('|').collect();
            assert_eq!(fields.len(), 5, "malformed row `{line}`");
            let frame: usize = fields[0].trim().parse().expect("frame number");
            let want_isf: Vec<u16> = fields[1]
                .split_whitespace()
                .map(|v| v.parse().expect("index"))
                .collect();
            let want_energy: i16 = fields[2].trim().parse().expect("energy index");
            let want_dither = fields[3].trim() == "1";
            let want_excitation: Vec<i16> = fields[4]
                .split_whitespace()
                .map(|v| v.parse().expect("sample"))
                .collect();
            assert_eq!(frame, rows, "the fixture skipped a frame");

            let (isf, energy) = probe_input(frame, &mut lcg);
            dtx.buffer(&mut ctx, &isf, energy, 2);
            let sid = dtx.build_sid(&mut ctx);
            let excitation = dtx.excitation(&mut ctx, sid.energy_index);

            for (split, (&got, &want)) in sid.isf_indices.iter().zip(&want_isf).enumerate() {
                assert_eq!(got, want, "frame {frame} ISF split {split}");
                isf_indices_seen.insert((split, got));
                compared += 1;
            }
            assert_eq!(sid.energy_index.0, want_energy, "frame {frame} energy index");
            energies_seen.insert(sid.energy_index.0);
            assert_eq!(sid.dither, want_dither, "frame {frame} dithering flag");
            dithers_seen.insert(sid.dither);
            compared += 2;

            for (i, (&got, &want)) in excitation.iter().zip(&want_excitation).enumerate() {
                assert_eq!(got.0, want, "frame {frame} excitation sample {i}");
                compared += 1;
            }
            rows += 1;
        }

        assert_eq!(rows, 40, "the fixture lost rows");
        assert_eq!(compared, rows * (5 + 2 + 8));
        // The fixture has to keep discriminating. A sweep that collapsed onto
        // one codevector, one energy or one dithering decision would still
        // pass every assertion above.
        assert!(isf_indices_seen.len() >= 40, "{} distinct ISF indices", isf_indices_seen.len());
        assert!(energies_seen.len() >= 15, "{} distinct energies", energies_seen.len());
        assert_eq!(dithers_seen.len(), 2, "the dithering flag never varied");
    }

    #[test]
    fn the_hangover_runs_seven_frames_from_a_cold_start() {
        // After a long talk spurt the first seven quiet frames still go out as
        // speech -- extra hangover for the decoder's own analysis -- and the
        // eighth becomes comfort noise.
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        for _ in 0..3 {
            assert_eq!(dtx.classify(&mut ctx, true), TxDecision::Speech);
        }
        let mut speech_after_spurt = 0;
        let mut first_cn = None;
        for frame in 0..12 {
            if dtx.classify(&mut ctx, false) == TxDecision::Speech {
                speech_after_spurt += 1;
            } else if first_cn.is_none() {
                first_cn = Some(frame);
            }
        }
        assert_eq!(speech_after_spurt, 7, "the extra hangover is not seven frames");
        assert_eq!(first_cn, Some(7));
    }

    #[test]
    fn a_short_burst_inside_silence_gets_no_hangover_at_all() {
        // The part that surprises: `dec_ana_elapsed_count` was reset by the DTX
        // the burst interrupted, so the very next quiet frame is comfort noise.
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        for _ in 0..12 {
            dtx.classify(&mut ctx, false);
        }
        for _ in 0..3 {
            assert_eq!(dtx.classify(&mut ctx, true), TxDecision::Speech);
        }
        assert_eq!(
            dtx.classify(&mut ctx, false),
            TxDecision::ComfortNoise,
            "a short burst should not restore the hangover"
        );
    }

    #[test]
    fn speech_rearms_the_counter_that_mode_8_reads() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        assert_eq!(dtx.hangover_count(), DTX_HANG_CONST);
        for _ in 0..3 {
            dtx.classify(&mut ctx, false);
        }
        assert!(dtx.hangover_count().0 < DTX_HANG_CONST.0);
        dtx.classify(&mut ctx, true);
        assert_eq!(
            dtx.hangover_count(),
            DTX_HANG_CONST,
            "23.85 kbit/s reads this, so a stale value changes the bitstream"
        );
    }

    #[test]
    fn the_history_ring_starts_at_slot_one_and_wraps_at_eight() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxEncoder::new();
        let isf = [Word16(1000); LP_ORDER];
        assert_eq!(dtx.hist_ptr, 0);
        dtx.buffer(&mut ctx, &isf, Word32(1_000_000), 2);
        assert_eq!(dtx.hist_ptr, 1, "the newest frame lands at the new pointer");
        for _ in 0..7 {
            dtx.buffer(&mut ctx, &isf, Word32(1_000_000), 2);
        }
        assert_eq!(dtx.hist_ptr, 0, "eight calls wrap");
    }
}
