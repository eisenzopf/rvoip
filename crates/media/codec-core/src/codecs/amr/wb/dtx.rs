//! Decoder-side DTX for AMR-WB, TS 26.173 `dtx.c`.
//!
//! # It runs on every frame, including speech
//!
//! [`DtxDecoder::receive`] is the reference's `rx_dtx_handler`, and
//! `dec_main.c` calls it as the first statement of `decoder()` with no
//! enclosing condition. Running it only on non-speech frames is a bug whose
//! symptom appears several frames later: `dec_ana_elapsed_count` goes stale,
//! so the first `SID_FIRST` after the talker stops takes the wrong branch and
//! comfort noise is built from nothing.
//!
//! # Where comfort noise comes from when the SID is empty
//!
//! A `SID_FIRST` carries thirty-five zero bits. The decoder has to synthesise
//! from its own memory of the talker's background instead, which is why
//! [`DtxDecoder::observe_speech`] keeps a ring of decoded spectra and
//! excitation energies on *speech* frames. The reference deliberately counts
//! the newest entry twice when averaging that ring, weighting the most recent
//! frame 2/8.
//!
//! # Four fields whose names the encoder also uses
//!
//! `isf_hist`, `log_en_hist`, `hist_ptr` and `cng_seed` all appear in the
//! encoder's DTX state too, and mean different things. The encoder's rings hold
//! *analysis* ISFs and *residual* energies with a per-mode offset subtracted;
//! these hold *decoded* ISFs and *excitation* energies with no offset at all.
//! The two `cng_seed`s are independent registers that happen to start at the
//! same value and stay in step only because both draw exactly 256 times per
//! comfort-noise frame.
//!
//! The distance matrix has no decoder counterpart: the dithering decision is
//! read off the wire as one bit rather than recomputed.

use super::isf_noise;
use super::lp::autocorr::LP_ORDER;
use super::lp::isf_dequant::ISF_INIT;
use super::math::{dot_product12, isqrt_n, pow2};
use crate::fixed_point::arith::{add, extract_h, extract_l, mult, sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::shr;
use crate::fixed_point::arith32::{l_add, l_deposit_h, l_mac, l_mult, l_sub};
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, shl};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Frames of history the backward analysis averages.
const DTX_HIST_SIZE: usize = 8;

/// Hangover the encoder is assumed to apply, mirrored here.
const DTX_HANG_CONST: Word16 = Word16(7);

/// `24 + DTX_HANG_CONST - 1`: how stale the decoder's own analysis may get
/// before a `SID_FIRST` is allowed to trigger a backward analysis.
const DTX_ELAPSED_FRAMES_THRESH: Word16 = Word16(24 + 7 - 1);

/// Frames without a comfort-noise update after which the output fades.
const DTX_MAX_EMPTY_THRESH: Word16 = Word16(50);

/// Reset value of the interpolation targets, Q9.
const LOG_EN_INIT: Word16 = Word16(3500);

/// How a frame arrived, as far as DTX is concerned.
///
/// Narrower than the codec's own frame types on purpose: what this machine
/// cares about is whether a frame renews the comfort-noise parameters, keeps
/// them, or says nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxFrameType {
    /// Speech bits the transport believes.
    SpeechGood,
    /// Speech bits that may be damaged.
    SpeechBad,
    /// Nothing arrived.
    SpeechLost,
    /// The first comfort-noise frame of a silence. Its payload is blank.
    SidFirst,
    /// A comfort-noise frame carrying real parameters.
    SidUpdate,
    /// A comfort-noise frame whose bits may be damaged.
    SidBad,
    /// The sender deliberately transmitted nothing.
    NoData,
}

impl RxFrameType {
    /// Whether this frame is SID-shaped, whatever the state of its bits.
    const fn is_sid(self) -> bool {
        matches!(self, Self::SidFirst | Self::SidUpdate | Self::SidBad)
    }
}

/// What the decoder should synthesise this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DtxState {
    /// Decode the frame normally.
    Speech,
    /// Synthesise comfort noise.
    ComfortNoise,
    /// Comfort noise, faded out: too long has passed with no update.
    Muted,
}

/// Decoder DTX state — the reference's `dtx_decState`.
///
/// The several flags are the reference's own, each set in one place and read
/// in another; collapsing them into an enum would merge states the machine
/// treats separately.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct DtxDecoder {
    since_last_sid: Word16,
    /// One over the observed SID period, Q15: the interpolation slope.
    true_sid_period_inv: Word16,
    /// Current and previous energy targets, Q9.
    log_en: Word16,
    old_log_en: Word16,
    /// Current and previous comfort-noise spectra.
    isf: [Word16; LP_ORDER],
    isf_old: [Word16; LP_ORDER],
    /// Excitation generator. Independent of the encoder's register of the same
    /// name, and of the high band's.
    cng_seed: Word16,
    /// Ring of decoded speech spectra, for the backward analysis a `SID_FIRST`
    /// needs.
    isf_hist: [Word16; LP_ORDER * DTX_HIST_SIZE],
    /// Ring of decoded speech energies, Q7.
    log_en_hist: [Word16; DTX_HIST_SIZE],
    hist_ptr: usize,
    dtx_hangover_count: Word16,
    dec_ana_elapsed_count: Word16,
    /// Set by [`Self::receive`], read by [`Self::comfort_noise`].
    sid_frame: bool,
    valid_data: bool,
    dtx_hangover_added: bool,
    /// The *previous* frame's state.
    global_state: DtxState,
    /// Whether the comfort-noise parameters have ever been renewed.
    data_updated: bool,
    /// Dithering generator, separate from the excitation's.
    dither_seed: Word16,
    /// Latched from the last `SID_UPDATE` and sticky across the gaps.
    dither: bool,
    /// Consecutive frames whose VAD bit said no speech.
    vad_history: Word16,
}

impl Default for DtxDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DtxDecoder {
    /// A DTX decoder in its reset state, as `dtx_dec_reset` leaves it.
    #[must_use]
    pub fn new() -> Self {
        let mut isf = [Word16(0); LP_ORDER];
        for (slot, &value) in isf.iter_mut().zip(&ISF_INIT) {
            *slot = Word16(value);
        }
        let mut isf_hist = [Word16(0); LP_ORDER * DTX_HIST_SIZE];
        for frame in isf_hist.chunks_exact_mut(LP_ORDER) {
            frame.copy_from_slice(&isf);
        }
        Self {
            since_last_sid: Word16(0),
            // 0.25 in Q15: a four-frame period until one is observed.
            true_sid_period_inv: Word16(8192),
            log_en: LOG_EN_INIT,
            old_log_en: LOG_EN_INIT,
            isf,
            isf_old: isf,
            cng_seed: Word16(21845),
            isf_hist,
            // The Q9 reset target expressed in the ring's Q7.
            log_en_hist: [Word16(LOG_EN_INIT.0 >> 3); DTX_HIST_SIZE],
            hist_ptr: 0,
            dtx_hangover_count: DTX_HANG_CONST,
            dec_ana_elapsed_count: Word16(32767),
            sid_frame: false,
            valid_data: false,
            dtx_hangover_added: false,
            global_state: DtxState::Speech,
            data_updated: false,
            dither_seed: Word16(21845),
            dither: false,
            vad_history: Word16(0),
        }
    }

    /// Classify a frame — `rx_dtx_handler`.
    ///
    /// Must be called for **every** frame, speech included: it maintains the
    /// staleness counter that decides whether a later `SID_FIRST` may analyse
    /// backwards.
    pub fn receive(&mut self, ctx: &mut DspContext, frame_type: RxFrameType) -> DtxState {
        let continuing = matches!(self.global_state, DtxState::ComfortNoise | DtxState::Muted)
            && matches!(
                frame_type,
                RxFrameType::NoData | RxFrameType::SpeechBad | RxFrameType::SpeechLost
            );

        let state = if frame_type.is_sid() || continuing {
            // Staying muted, once muted. Note `SpeechBad` is deliberately
            // absent: the reference's own comment table claims it keeps the
            // mute and the code does not, and the code is what ships.
            let stays_muted = self.global_state == DtxState::Muted
                && matches!(
                    frame_type,
                    RxFrameType::SidBad
                        | RxFrameType::SidFirst
                        | RxFrameType::SpeechLost
                        | RxFrameType::NoData
                );
            let mut state = if stays_muted {
                DtxState::Muted
            } else {
                DtxState::ComfortNoise
            };
            self.since_last_sid = add(ctx, self.since_last_sid, Word16(1));
            if frame_type != RxFrameType::SidUpdate
                && sub(ctx, self.since_last_sid, DTX_MAX_EMPTY_THRESH).0 > 0
            {
                state = DtxState::Muted;
            }
            state
        } else {
            self.since_last_sid = Word16(0);
            DtxState::Speech
        };

        // Bookkeeping, whatever the decision.
        if !self.data_updated && frame_type == RxFrameType::SidUpdate {
            self.dec_ana_elapsed_count = Word16(0);
        }
        self.dec_ana_elapsed_count = add(ctx, self.dec_ana_elapsed_count, Word16(1));
        self.dtx_hangover_added = false;

        // What the *encoder* is presumed to be doing, inferred from what
        // arrived. The decoder is guessing at the far end's own hangover
        // counter so it knows whether a SID_FIRST has enough speech behind it
        // to analyse.
        let encoder_in_dtx = frame_type.is_sid()
            || (frame_type == RxFrameType::NoData
                && (self.global_state != DtxState::Speech
                    || self.vad_history.0 >= DTX_HANG_CONST.0));
        if encoder_in_dtx {
            if sub(ctx, self.dec_ana_elapsed_count, DTX_ELAPSED_FRAMES_THRESH).0 > 0 {
                self.dtx_hangover_added = true;
                self.dec_ana_elapsed_count = Word16(0);
                self.dtx_hangover_count = Word16(0);
            } else if self.dtx_hangover_count.0 == 0 {
                self.dec_ana_elapsed_count = Word16(0);
            } else {
                self.dtx_hangover_count = sub(ctx, self.dtx_hangover_count, Word16(1));
            }
        } else {
            self.dtx_hangover_count = DTX_HANG_CONST;
        }

        if state != DtxState::Speech {
            self.sid_frame = false;
            self.valid_data = false;
            match frame_type {
                RxFrameType::SidFirst => self.sid_frame = true,
                RxFrameType::SidUpdate => {
                    self.sid_frame = true;
                    self.valid_data = true;
                }
                RxFrameType::SidBad => {
                    self.sid_frame = true;
                    // Cancels whatever the branch above just set: damaged SID
                    // bits are not a reason to re-analyse.
                    self.dtx_hangover_added = false;
                }
                _ => {}
            }
        }
        state
    }

    /// Record a decoded speech frame — `dtx_dec_activity_update`.
    ///
    /// Only on frames that took the speech path: this is the memory a
    /// `SID_FIRST` synthesises from, and it must describe the talker's
    /// background rather than the comfort noise the decoder itself produced.
    pub fn observe_speech(
        &mut self,
        ctx: &mut DspContext,
        isf: &[Word16; LP_ORDER],
        excitation: &[Word16],
    ) {
        self.hist_ptr = (self.hist_ptr + 1) % DTX_HIST_SIZE;
        let base = self.hist_ptr * LP_ORDER;
        self.isf_hist[base..base + LP_ORDER].copy_from_slice(isf);

        let mut energy = Word32(0);
        for &sample in excitation {
            energy = l_mac(ctx, energy, sample, sample);
        }
        let energy = l_shr(ctx, energy, 1);
        let (exponent, mantissa) = super::math::log2(ctx, energy);
        let mut log_en = shl(ctx, Word16(exponent), 7);
        let fraction = shr(ctx, mantissa, 8);
        log_en = add(ctx, log_en, fraction);
        // No per-mode offset here: the encoder's ring subtracts one and this
        // one does not, because they measure different signals.
        self.log_en_hist[self.hist_ptr] = sub(ctx, log_en, Word16(1024));
    }

    /// Update the VAD history the encoder-state inference reads.
    pub fn observe_vad(&mut self, ctx: &mut DspContext, voice_active: bool) {
        self.vad_history = if voice_active {
            Word16(0)
        } else {
            add(ctx, self.vad_history, Word16(1))
        };
    }

    /// Synthesise one frame of comfort noise — `dtx_dec`.
    ///
    /// `sid` is the five decoded SID fields when this frame carried a usable
    /// payload. Returns the spectrum to synthesise through and the excitation
    /// to drive it with.
    pub fn comfort_noise(
        &mut self,
        ctx: &mut DspContext,
        state: DtxState,
        sid: Option<SidFields>,
    ) -> ([Word16; LP_ORDER], [Word16; 256]) {
        if self.dtx_hangover_added && self.sid_frame {
            self.analyse_backwards(ctx);
        }

        if self.sid_frame {
            self.isf_old = self.isf;
            self.old_log_en = self.log_en;

            // Gated on `valid_data`, not on whether the caller supplied a
            // payload: a SID_FIRST's thirty-five bits are blank, and decoding
            // them here would overwrite the backward analysis that just
            // reconstructed the spectrum from the decoder's own history.
            if let Some(fields) = sid.filter(|_| self.valid_data) {
                // The observed period, from which the interpolation slope
                // follows. Capped at 32 frames.
                let mut period = self.since_last_sid;
                if period.0 > 32 {
                    period = Word16(32);
                }
                self.true_sid_period_inv = if period.0 >= 2 {
                    div_s(Word16(1 << 10), shl(ctx, period, 10))
                } else {
                    Word16(1 << 14)
                };

                self.isf = isf_noise::dequantise(ctx, &fields.isf_indices);
                let scaled = shl(ctx, fields.energy_index, 9);
                self.log_en = mult(ctx, scaled, Word16(12483));
                self.dither = fields.dither;

                // Nothing to interpolate from at a cold start, or when the
                // previous frame was speech.
                if !self.data_updated || self.global_state == DtxState::Speech {
                    self.isf_old = self.isf;
                    self.old_log_en = self.log_en;
                }
            }
        }
        if self.sid_frame && self.valid_data {
            self.since_last_sid = Word16(0);
        }

        // Interpolate between the previous target and this one.
        let mut factor = shl(ctx, self.since_last_sid, 10);
        factor = mult(ctx, factor, self.true_sid_period_inv);
        if factor.0 > 1024 {
            factor = Word16(1024);
        }
        factor = shl(ctx, factor, 4);
        let mut energy = l_mult(ctx, factor, self.log_en);
        let mut isf = [Word16(0); LP_ORDER];
        for (slot, &target) in isf.iter_mut().zip(&self.isf) {
            *slot = mult(ctx, factor, target);
        }
        let complement = sub(ctx, Word16(16384), factor);
        energy = l_mac(ctx, energy, complement, self.old_log_en);
        for (slot, &previous) in isf.iter_mut().zip(&self.isf_old) {
            let blended = mult(ctx, complement, previous);
            let summed = add(ctx, *slot, blended);
            *slot = shl(ctx, summed, 1);
        }

        if self.dither {
            self.apply_dithering(ctx, &mut isf, &mut energy);
        }

        // The fade. Once muted, each frame drops the target another 3/8 dB and
        // restarts the interpolation from where it is, so the noise decays
        // smoothly rather than cutting off.
        if state == DtxState::Muted {
            let mut period = self.since_last_sid;
            if period.0 > 32 {
                period = Word16(32);
            }
            if period.0 <= 0 {
                // The reference guards its own division here.
                period = Word16(8);
            }
            self.true_sid_period_inv = div_s(Word16(1 << 10), shl(ctx, period, 10));
            self.since_last_sid = Word16(0);
            self.old_log_en = self.log_en;
            self.log_en = sub(ctx, self.log_en, Word16(64));
        }

        let excitation = self.excitation(ctx, energy);

        // The interpolation timer restarts whenever the parameters were
        // renewed -- by a usable SID *or* by the backward analysis a SID_FIRST
        // triggers. Only accounting for the first leaves every later
        // interpolation a frame out of step.
        if self.sid_frame && (self.valid_data || self.dtx_hangover_added) {
            self.since_last_sid = Word16(0);
            self.data_updated = true;
        }
        (isf, excitation)
    }

    /// Commit the state this frame decided, once synthesis is done.
    #[allow(clippy::missing_const_for_fn)]
    pub fn commit(&mut self, state: DtxState) {
        self.global_state = state;
    }

    /// Average the ring, counting the newest frame twice — the `SID_FIRST`
    /// path.
    fn analyse_backwards(&mut self, ctx: &mut DspContext) {
        let newest = self.hist_ptr;
        let duplicate = (newest + 1) % DTX_HIST_SIZE;
        let (from, to) = (newest * LP_ORDER, duplicate * LP_ORDER);
        let copy: [Word16; LP_ORDER] = self.isf_hist[from..from + LP_ORDER]
            .try_into()
            .expect("one spectrum");
        self.isf_hist[to..to + LP_ORDER].copy_from_slice(&copy);
        self.log_en_hist[duplicate] = self.log_en_hist[newest];

        let mut log_en = Word16(0);
        for &value in &self.log_en_hist {
            log_en = add(ctx, log_en, value);
        }
        let mut sums = [Word32(0); LP_ORDER];
        for frame in self.isf_hist.chunks_exact(LP_ORDER) {
            for (acc, &value) in sums.iter_mut().zip(frame) {
                *acc = l_add(ctx, *acc, Word32(i32::from(value.0)));
            }
        }

        // Q10 to Q9, then the +2 the level conversion undoes.
        log_en = shr(ctx, log_en, 1);
        log_en = add(ctx, log_en, Word16(1024));
        if log_en.0 < 0 {
            log_en = Word16(0);
        }
        self.log_en = log_en;
        for (slot, &sum) in self.isf.iter_mut().zip(&sums) {
            *slot = extract_l(l_shr(ctx, sum, 3));
        }
    }

    /// Build the excitation for one comfort-noise frame.
    ///
    /// Byte-identical to the encoder's, from a separate register.
    fn excitation(&mut self, ctx: &mut DspContext, energy: Word32) -> [Word16; 256] {
        let energy = l_shr(ctx, energy, 9);
        let exponent = extract_h(energy);
        let remainder = l_sub(ctx, energy, l_deposit_h(exponent));
        let mantissa = extract_l(l_shr(ctx, remainder, 1));
        let exponent = add(ctx, exponent, Word16(15));
        let level32 = pow2(ctx, exponent.0, mantissa);
        let shift = norm_l(level32);
        let level = extract_h(l_shl(ctx, level32, shift));
        let level_exp = sub(ctx, Word16(15), Word16(shift));

        let mut excitation = [Word16(0); 256];
        for slot in &mut excitation {
            let sample = self.next_noise(ctx);
            *slot = shr(ctx, sample, 4);
        }
        let measured = dot_product12(ctx, &excitation, &excitation);
        let (energy, energy_exp) = isqrt_n(ctx, measured);
        let gain = mult(ctx, level, extract_h(energy));
        let combined = add(ctx, level_exp, Word16(energy_exp));
        // +4 scales by sqrt(256), the length the energy was measured over.
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

    /// One step of the dithering generator, which is a separate register.
    fn next_dither(&mut self, ctx: &mut DspContext) -> Word16 {
        let product = l_mult(ctx, self.dither_seed, Word16(31821));
        let halved = l_shr(ctx, product, 1);
        self.dither_seed = extract_l(l_add(ctx, halved, Word32(13849)));
        self.dither_seed
    }

    /// Perturb the interpolated spectrum and energy — `CN_dithering`.
    ///
    /// Only when the encoder said the background was moving. Without it a
    /// non-stationary background is reproduced as a steady tone, which is
    /// more noticeable than the noise it replaced.
    fn apply_dithering(
        &mut self,
        ctx: &mut DspContext,
        isf: &mut [Word16; LP_ORDER],
        energy: &mut Word32,
    ) {
        /// Energy perturbation scale.
        const GAIN_FACTOR: Word16 = Word16(75);
        /// Where the per-coefficient spectral perturbation starts.
        const ISF_FACTOR_LOW: Word16 = Word16(256);
        /// How much it grows per coefficient.
        const ISF_FACTOR_STEP: Word16 = Word16(2);
        /// Minimum spacing dithering must not violate.
        const ISF_DITH_GAP: Word16 = Word16(448);

        // A triangular variate, from two draws.
        let a = self.next_dither(ctx);
        let b = self.next_dither(ctx);
        let half_a = shr(ctx, a, 1);
        let half_b = shr(ctx, b, 1);
        let triangular = add(ctx, half_a, half_b);
        let perturbation = mult(ctx, triangular, GAIN_FACTOR);
        let widened = l_mult(ctx, perturbation, Word16(1));
        *energy = l_add(ctx, *energy, widened);

        let mut factor = ISF_FACTOR_LOW;
        for slot in isf.iter_mut() {
            let draw = self.next_dither(ctx);
            let delta = mult(ctx, draw, factor);
            *slot = add(ctx, *slot, delta);
            factor = add(ctx, factor, ISF_FACTOR_STEP);
        }
        // The spacing the perturbation must not destroy.
        let mut floor = ISF_DITH_GAP;
        for slot in isf.iter_mut().take(LP_ORDER - 1) {
            if slot.0 < floor.0 {
                *slot = floor;
            }
            floor = add(ctx, *slot, ISF_DITH_GAP);
        }
        if isf[LP_ORDER - 2].0 > 16384 {
            isf[LP_ORDER - 2] = Word16(16384);
        }
    }
}

/// The five fields a usable SID payload carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidFields {
    /// The comfort-noise ISF codebook indices.
    pub isf_indices: [u16; 5],
    /// The quantised frame energy.
    pub energy_index: Word16,
    /// Whether to dither the synthesised noise.
    pub dither: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_handler_runs_on_speech_and_keeps_the_staleness_counter_moving() {
        // The bug the module header warns about: skipping the handler on
        // speech leaves the counter stale, and the first SID_FIRST after the
        // talker stops then takes the wrong branch.
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        for _ in 0..30 {
            assert_eq!(
                dtx.receive(&mut ctx, RxFrameType::SpeechGood),
                DtxState::Speech
            );
            dtx.commit(DtxState::Speech);
        }
        assert_eq!(
            dtx.dec_ana_elapsed_count,
            Word16(32767),
            "the counter must saturate rather than wrap"
        );
        assert_eq!(
            dtx.receive(&mut ctx, RxFrameType::SidFirst),
            DtxState::ComfortNoise
        );
        assert!(
            dtx.dtx_hangover_added,
            "a SID_FIRST after a talk spurt must permit the backward analysis"
        );
    }

    #[test]
    fn a_long_gap_without_an_update_fades_to_mute() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.receive(&mut ctx, RxFrameType::SidFirst);
        dtx.commit(DtxState::ComfortNoise);
        let mut muted_at = None;
        for frame in 0..70 {
            let state = dtx.receive(&mut ctx, RxFrameType::NoData);
            dtx.commit(state);
            if state == DtxState::Muted && muted_at.is_none() {
                muted_at = Some(frame);
            }
        }
        assert_eq!(
            muted_at,
            Some(49),
            "DTX_MAX_EMPTY_THRESH is 50 frames without an update"
        );
    }

    #[test]
    fn damaged_speech_leaves_the_mute_but_a_gap_does_not() {
        // The reference's own comment table says SPEECH_BAD keeps the mute and
        // its code says otherwise. The code is what ships.
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.receive(&mut ctx, RxFrameType::SidFirst);
        dtx.commit(DtxState::Muted);
        assert_eq!(
            dtx.receive(&mut ctx, RxFrameType::SpeechBad),
            DtxState::ComfortNoise,
            "SPEECH_BAD is absent from the stay-muted list"
        );
        dtx.commit(DtxState::Muted);
        assert_eq!(dtx.receive(&mut ctx, RxFrameType::NoData), DtxState::Muted);
    }

    #[test]
    fn an_update_renews_the_parameters_and_a_first_does_not() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.receive(&mut ctx, RxFrameType::SidFirst);
        assert!(dtx.sid_frame && !dtx.valid_data);
        dtx.commit(DtxState::ComfortNoise);
        dtx.receive(&mut ctx, RxFrameType::SidUpdate);
        assert!(dtx.sid_frame && dtx.valid_data);
    }

    /// Against TS 26.173's own `rx_dtx_handler`, `dtx_dec` and
    /// `dtx_dec_activity_update`, frame by frame.
    ///
    /// The frame-type sequence is chosen to visit every state and both
    /// directions between them: speech, the first SID, gaps, updates, a
    /// damaged SID, a gap long enough to fade to mute, and an update that
    /// brings it back. A machine that got any transition wrong produces
    /// plausible noise from the wrong parameters, which only a frame-by-frame
    /// comparison catches.
    #[test]
    fn the_state_machine_and_comfort_noise_match_the_reference() {
        let text = include_str!("../testdata/wb_dtx_dec_vectors.txt");
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        let mut compared = 0usize;
        let mut rows = 0usize;
        let mut states = std::collections::HashSet::new();

        for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let fields: Vec<&str> = line.split('|').collect();
            assert_eq!(fields.len(), 4, "malformed row `{line}`");
            let head: Vec<i32> = fields[0]
                .split_whitespace()
                .map(|v| v.parse().expect("number"))
                .collect();
            let frame = usize::try_from(head[0]).expect("frame index");
            let raw_type = head[1];
            let want_state = match fields[1].trim() {
                "0" => DtxState::Speech,
                "1" => DtxState::ComfortNoise,
                "2" => DtxState::Muted,
                other => panic!("unknown state {other}"),
            };
            assert_eq!(frame, rows, "the fixture skipped a frame");

            // The reference's RX_* enumeration, in its own order.
            let frame_type = match raw_type {
                0 => RxFrameType::SpeechGood,
                2 => RxFrameType::SpeechLost,
                3 => RxFrameType::SpeechBad,
                4 => RxFrameType::SidFirst,
                5 => RxFrameType::SidUpdate,
                6 => RxFrameType::SidBad,
                7 => RxFrameType::NoData,
                other => panic!("unhandled RX type {other}"),
            };

            let state = dtx.receive(&mut ctx, frame_type);
            assert_eq!(state, want_state, "frame {frame} state");
            states.insert(state);
            compared += 1;

            if state == DtxState::Speech {
                // What the decoder feeds back after the ACELP path, mirroring
                // the probe exactly.
                let mut isf = [Word16(0); LP_ORDER];
                for (i, slot) in isf.iter_mut().enumerate() {
                    let i = i16::try_from(i).expect("order is 16");
                    let f = i16::try_from(frame).expect("short fixture");
                    *slot = Word16(500 + i * 900 + f * 7);
                }
                let f = i32::try_from(frame).expect("short fixture");
                let excitation: Vec<Word16> = (0..256)
                    .map(|i: i32| {
                        let value = ((i * 37 + f * 11) % 401) - 200;
                        Word16(i16::try_from(value).expect("in range"))
                    })
                    .collect();
                dtx.observe_speech(&mut ctx, &isf, &excitation);
            } else {
                let fields_in = SidFields {
                    isf_indices: [10, 20, 30, 5, 7],
                    energy_index: Word16(30),
                    dither: false,
                };
                let (isf, excitation) = dtx.comfort_noise(&mut ctx, state, Some(fields_in));
                let want_isf: Vec<i16> = fields[2]
                    .split_whitespace()
                    .map(|v| v.parse().expect("isf"))
                    .collect();
                let want_exc: Vec<i16> = fields[3]
                    .split_whitespace()
                    .map(|v| v.parse().expect("sample"))
                    .collect();
                for (i, (&got, &want)) in isf.iter().zip(&want_isf).enumerate() {
                    assert_eq!(got.0, want, "frame {frame} ISF {i}");
                    compared += 1;
                }
                for (i, (&got, &want)) in excitation.iter().zip(&want_exc).enumerate() {
                    assert_eq!(got.0, want, "frame {frame} excitation sample {i}");
                    compared += 1;
                }
            }
            dtx.commit(state);
            rows += 1;
        }

        assert_eq!(rows, 89, "the fixture lost rows");
        assert!(compared > 700, "only {compared} values compared");
        assert_eq!(states.len(), 3, "the sequence did not visit every state");
    }

    #[test]
    fn comfort_noise_is_deterministic_and_not_silent() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.receive(&mut ctx, RxFrameType::SidUpdate);
        let fields = SidFields {
            isf_indices: [10, 20, 30, 5, 7],
            energy_index: Word16(30),
            dither: false,
        };
        let (isf, excitation) = dtx.comfort_noise(&mut ctx, DtxState::ComfortNoise, Some(fields));
        assert!(excitation.iter().any(|s| s.0 != 0), "the noise is silent");
        assert!(isf.iter().any(|s| s.0 != 0), "the spectrum is empty");
        // Deliberately no monotonicity assertion here. `Reorder_isf` runs
        // inside the dequantiser, but the *interpolated* result is not
        // reordered -- the reference leaves that to the ISF-to-ISP conversion
        // downstream -- so requiring it here would assert a property the
        // reference does not have.

        let mut again = DtxDecoder::new();
        again.receive(&mut ctx, RxFrameType::SidUpdate);
        let (isf2, exc2) = again.comfort_noise(&mut ctx, DtxState::ComfortNoise, Some(fields));
        assert_eq!(isf, isf2);
        assert_eq!(excitation, exc2, "the generator must be deterministic");
    }
}
