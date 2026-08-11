//! The assembled AMR-WB decoder, 3GPP TS 26.190 §6.
//!
//! Wires the verified stages into a decoder: payload in, 16 kHz PCM out.
//!
//! # Two excitations, and why
//!
//! The single most error-prone thing in this file is that there are **two**
//! excitation signals per subframe and they must not be conflated:
//!
//! - `exc` — the plain sum of adaptive and algebraic contributions. This is
//!   written back to the adaptive-codebook history, because it is what the
//!   *encoder* believes it produced. Anything else and the two pitch
//!   predictors drift apart over seconds.
//! - `exc2` — the same sum built from the *enhanced* innovation and gain
//!   (phase dispersion, noise enhancer, pitch enhancer). This is what the
//!   synthesis filter runs on, and it never re-enters the history.
//!
//! Using one buffer for both produces audio that is recognisably speech and
//! steadily wrong — the failure mode no per-stage test can catch.

use super::codebook::{self, L_SUBFR};
use super::conceal::Erasure;
use super::enhance::{
    agc2, enhance_strength, pitch_enhance_with, stability_factor, voice_factor,
    DispersionLevel, NoiseEnhancer, PhaseDispersion,
};
use super::excitation::{Excitation, Upsampler, L_SUBFR16K};
use super::gain::{FrameContext, FrameQuality, GainDecoder};
use super::highband::{self, BandFilter, NoiseGenerator, NoiseShaper, TiltFilter};
use super::lp::autocorr::LP_ORDER;
use super::math::scale_sig;
use super::lp::isf::{interpolate_isp, isf_to_isp, INTERPOL_FRAC, NB_SUBFR};
use super::lp::isp_to_lp::isp_to_lp_order;
use super::lp::isf_dequant::{IsfDecoder, IsfQuantizer, ISF_INIT};
use super::ltp::{self, PIT_SHARP};
use super::params::{FrameParams, SubframeParams};
use super::synthesis::{deemphasis, HighPass50, SynthesisFilter, PREEMPH_FAC};
use crate::codecs::amr::mode::AmrMode;
use crate::fixed_point::arith::{add, mult, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult};
use crate::fixed_point::oper32::l_extract;
use crate::fixed_point::shift::{l_shl, shl, shr};
use crate::fixed_point::types::{DspContext, Word16};

/// Samples one frame produces, at 16 kHz.
pub const FRAME_SIZE_16K: usize = 320;

/// Frame sizes in bits, by mode index.
const FRAME_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

/// A complete AMR-WB decoder.
#[derive(Debug, Clone)]
pub struct Decoder {
    isf: IsfDecoder,
    gains: GainDecoder,
    excitation: Excitation,
    dispersion: PhaseDispersion,
    noise_enhancer: NoiseEnhancer,
    synthesis: SynthesisFilter,
    deemph_memory: Word16,
    high_pass: HighPass50,
    upsampler: Upsampler,
    noise: NoiseGenerator,
    tilt_filter: TiltFilter,
    shaper: NoiseShaper,
    band_pass: BandFilter,
    low_pass_7k: BandFilter,
    /// Previous frame's ISPs, for interpolation.
    isp_old: [Word16; LP_ORDER],
    /// Previous frame's ISFs, for the stability measure.
    isf_old: [Word16; LP_ORDER],
    /// Discontinuous transmission: the receive-side state machine and the
    /// background history a `SID_FIRST` is synthesised from.
    ///
    /// Its bookkeeping runs on every frame, speech included. Deferring it to
    /// the frames that actually carry comfort noise leaves the staleness
    /// counter wrong, and the symptom appears several frames later.
    dtx: super::dtx::DtxDecoder,
    /// Spectral tilt of the innovation, carried to the next subframe.
    tilt_code: Word16,
    /// Whether any frame has been decoded yet.
    started: bool,
    /// Consecutive frames the encoder marked as containing no voice activity.
    ///
    /// Selects a different high-band gain curve: during background noise the
    /// band is attenuated less, so the noise floor sounds continuous rather
    /// than gated.
    vad_history: i16,
    /// Everything remembered across frames in order to conceal a lost one.
    erasure: Erasure,
    /// Scalar trace of the last decode.
    ///
    /// Exists to be diffed against the instrumented reference decoder — see
    /// `tools/trace-amr-reference.sh`. Comparing intermediates found every bug
    /// in this file so far; reasoning from output PCM found none of them.
    #[cfg(test)]
    pub(crate) trace: Vec<(&'static str, i64)>,
    /// Vector trace of the last decode, same purpose.
    #[cfg(test)]
    pub(crate) vtrace: Vec<(&'static str, Vec<i16>)>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder in its reset state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            isf: IsfDecoder::new(),
            gains: GainDecoder::new(),
            excitation: Excitation::new(),
            dispersion: PhaseDispersion::new(),
            noise_enhancer: NoiseEnhancer::new(),
            synthesis: SynthesisFilter::new(),
            deemph_memory: Word16(0),
            high_pass: HighPass50::new(),
            upsampler: Upsampler::new(),
            noise: NoiseGenerator::new(),
            tilt_filter: TiltFilter::new(),
            shaper: NoiseShaper::new(),
            band_pass: BandFilter::band_pass(),
            low_pass_7k: BandFilter::low_pass_7k(),
            // Never overwritten before use, unlike isp_old below: the
            // stability measure reads it on frame 0, and a zeroed history
            // would report a huge spectral jump on a perfectly ordinary first
            // frame. The reference seeds it to the same flat spectrum the ISF
            // decoder starts from.
            isf_old: ISF_INIT.map(Word16),
            dtx: super::dtx::DtxDecoder::new(),
            // Seeded on the first frame via `started`, so the value here is
            // never read. Kept explicit so the reset state is diffable against
            // the reference's.
            isp_old: [Word16(0); LP_ORDER],
            tilt_code: Word16(0),
            started: false,
            vad_history: 0,
            erasure: Erasure::new(),
            #[cfg(test)]
            trace: Vec::new(),
            #[cfg(test)]
            vtrace: Vec::new(),
        }
    }

    /// Decode one intact frame's payload into 320 samples at 16 kHz.
    ///
    /// Returns `None` if the payload does not parse for this mode.
    ///
    /// This is the whole interface for a clean stream. A receiver that knows a
    /// frame arrived damaged or not at all must say so through
    /// [`Self::decode_frame`] — passing a damaged payload here decodes noise as
    /// if it were signal.
    #[must_use]
    pub fn decode(&mut self, mode: AmrMode, payload: &[u8]) -> Option<[i16; FRAME_SIZE_16K]> {
        self.decode_frame(mode, payload, FrameQuality::Good)
    }

    /// Decode one frame, saying how it arrived.
    ///
    /// # Why a separate entry point rather than an argument on `decode`
    ///
    /// Because the overwhelmingly common case is a good frame, and every
    /// existing caller of [`Self::decode`] is asserting exactly that. Adding a
    /// quality argument there would have made every call site restate the
    /// default, and — worse — would have let a caller that had never thought
    /// about erasures pass whatever was handy. A second entry point makes
    /// concealment something a receiver opts into deliberately, once it
    /// actually has the information (an RTP Q bit, a lost sequence number, a
    /// `NO_DATA` frame type) to opt in with.
    ///
    /// `payload` is still read on a [`FrameQuality::Bad`] frame: damaged means
    /// *some* bits are wrong, and the algebraic codebook and the LTP filter
    /// select bit are decoded from it regardless. Only on
    /// [`FrameQuality::Unusable`] is the payload ignored entirely — pass an
    /// empty slice for a frame that never arrived, along with the mode the
    /// stream was last using, since a lost frame does not carry one.
    ///
    /// Returns `None` if a `Good` or `Bad` payload does not parse for this
    /// mode.
    ///
    /// Deliberately one function: the subframe loop is a single sequence whose
    /// order matters at every step, and splitting it would hide the two
    /// excitations' divergence behind a call boundary.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn decode_frame(
        &mut self,
        mode: AmrMode,
        payload: &[u8],
        quality: FrameQuality,
    ) -> Option<[i16; FRAME_SIZE_16K]> {
        self.decode_frame_with_state(mode, payload, quality, None)
    }

    /// [`decode_frame`](Self::decode_frame), optionally with the DTX state
    /// already decided.
    ///
    /// `rx_dtx_handler` runs **once per frame** in the reference, and its side
    /// effects — the staleness counter, the hangover bookkeeping — are not
    /// idempotent. A caller that has already classified this frame passes the
    /// result in rather than letting it run again.
    #[allow(clippy::too_many_lines)]
    fn decode_frame_with_state(
        &mut self,
        mode: AmrMode,
        payload: &[u8],
        quality: FrameQuality,
        decided: Option<super::dtx::DtxState>,
    ) -> Option<[i16; FRAME_SIZE_16K]> {
        let frame_bits = *FRAME_BITS.get(mode.index() as usize)?;
        let bad = quality != FrameQuality::Good;
        let unusable = quality == FrameQuality::Unusable;
        let params = match FrameParams::parse(mode, payload) {
            Some(params) => params,
            // A frame that never arrived has nothing to parse. Every field of
            // this placeholder is overwritten before it is read — the lag by
            // the concealer, the innovation by the noise generator, the gains
            // and spectrum from history — so it is not a guess about the
            // talker, it is a shape.
            None if unusable => placeholder_params(),
            None => return None,
        };
        self.erasure.begin_frame(quality);
        let mut ctx = DspContext::default();

        // `rx_dtx_handler`, which the reference runs before anything else and
        // on every frame. A speech frame only updates counters here, but those
        // counters are what a later `SID_FIRST` reads.
        let dtx_state = if let Some(state) = decided {
            state
        } else {
            let rx_type = match quality {
                FrameQuality::Good => super::dtx::RxFrameType::SpeechGood,
                FrameQuality::Unusable => super::dtx::RxFrameType::SpeechLost,
                FrameQuality::Bad => super::dtx::RxFrameType::SpeechBad,
            };
            self.dtx.receive(&mut ctx, rx_type)
        };
        if dtx_state != super::dtx::DtxState::Speech {
            return Some(self.synthesise_comfort_noise(&mut ctx, dtx_state, frame_bits, None));
        }
        #[cfg(test)]
        {
            self.trace.clear();
            self.vtrace.clear();
        }
        macro_rules! vtrace {
            ($n:expr, $v:expr) => {
                #[cfg(test)]
                self.vtrace
                    .push(($n, $v.iter().map(|w: &Word16| w.0).collect()));
            };
        }
        macro_rules! trace {
            ($n:expr, $v:expr) => {
                #[cfg(test)]
                self.trace.push(($n, i64::from($v)));
            };
        }

        // --- spectrum ---
        let quantizer = if frame_bits <= 132 {
            IsfQuantizer::Bits36
        } else {
            IsfQuantizer::Bits46
        };
        // The VAD flag counts up while the encoder sees no speech and resets
        // the moment it does. A bad frame's flag says nothing about the
        // talker, so the count is frozen rather than advanced: letting an
        // erasure vote here would swing the high-band gain curve on evidence
        // the decoder does not have.
        if !bad {
            if params.vad_flag {
                self.vad_history = 0;
            } else {
                self.vad_history = self.vad_history.saturating_add(1);
            }
        }

        trace!("bfi", i16::from(bad));
        trace!("unusable", i16::from(unusable));
        trace!("state", i16::try_from(self.erasure.severity()).unwrap_or(-1));
        trace!("prev_bfi", i16::from(self.erasure.previous_frame_was_bad()));
        trace!("vad_hist", self.vad_history);
        let isf = self.isf.decode(quantizer, &params.isf_indices, bad);
        let stab_fac = stability_factor(&mut ctx, &isf, &self.isf_old);
        // The previous frame's ISFs, needed again below for the 6.60 high
        // band, so capture before the update.
        let isf_previous = self.isf_old;
        self.isf_old = isf;

        trace!("stab_fac", stab_fac.0);
        let isp_new = isf_to_isp(&isf);
        if !self.started {
            // Nothing to interpolate from on the first frame.
            self.isp_old = isp_new;
            self.started = true;
        }
        let coefficients = interpolate_isp(&self.isp_old, &isp_new);
        self.isp_old = isp_new;

        // --- excitation and synthesis, subframe by subframe ---
        let mut out = [0i16; FRAME_SIZE_16K];
        // The frame's excitation, kept for the DTX background history below.
        let mut frame_excitation = [Word16(0); NB_SUBFR * L_SUBFR];
        let mut frame_q_new = 0i16;
        let gain_bits = if frame_bits <= 177 { 6 } else { 7 };
        let dispersion = DispersionLevel::for_frame_bits(frame_bits);

        for (sf, subframe) in params.subframes.iter().enumerate().take(NB_SUBFR) {
            // A suspect lag is rebuilt from the pitch contour before it is
            // used, because a wrong lag rings for hundreds of milliseconds
            // where a wrong pulse costs one subframe.
            let (pitch_lag, pitch_frac) = self.erasure.pitch_lag(
                &mut ctx,
                subframe.pitch_lag,
                subframe.pitch_frac,
                quality,
            );

            // Adaptive codebook. One extra sample: the low-pass reads ahead.
            let (buffer, offset) = self.excitation.buffer_mut();
            ltp::predict(
                buffer,
                offset,
                pitch_lag as usize,
                pitch_frac,
                L_SUBFR + 1,
            );
            #[cfg(test)]
            let pred_snapshot: Vec<i16> =
                buffer[offset..offset + L_SUBFR].iter().map(|w| w.0).collect();
            #[cfg(test)]
            self.vtrace.push(("pred", pred_snapshot));
            // Below 12.65 the filter is always on; above it the encoder picks.
            // A frame that carried no bits has no choice to read, and the
            // reference forces the filter off rather than defaulting it on.
            if !unusable && !subframe.ltp_filter {
                let smoothed = ltp::low_pass(&buffer[offset - 1..]);
                buffer[offset..offset + L_SUBFR].copy_from_slice(&smoothed);
            }

            trace!("T0", pitch_lag);
            trace!("T0_frac", pitch_frac);
            trace!("tilt_code_used", self.tilt_code.0);
            // Algebraic codebook, shaped by the previous subframe's tilt and
            // sharpened at the pitch period. A damaged frame still decodes its
            // own pulses: they are what a checksum failure is cheapest to be
            // wrong about.
            let mut code: [Word16; L_SUBFR] = if unusable {
                self.erasure.lost_innovation(&mut ctx)
            } else {
                codebook::decode(&subframe.pulses, frame_bits)?.map(Word16)
            };
            let mut discard = Word16(0);
            ltp::preemphasis(&mut code, self.tilt_code, &mut discard);
            ltp::sharpen(
                &mut code,
                ltp::sharpening_lag(pitch_lag as usize, pitch_frac),
                PIT_SHARP,
            );

            vtrace!("code", code);
            // `prev_bfi` comes from the erasure tracker rather than from the
            // gain decoder's own memory, because TS 26.173 sets it once per
            // *frame*: all four subframes of the first good frame after an
            // erasure cap a sudden jump in code gain, not just the first.
            let gains = self.gains.decode(
                subframe.gain_index,
                gain_bits,
                &code,
                FrameContext {
                    quality,
                    erasure_state: self.erasure.severity(),
                    vad_history: self.vad_history,
                    previous_frame_bad: self.erasure.previous_frame_was_bad(),
                },
            );
            // Only a subframe that decoded cleanly is evidence about the pitch
            // contour. Feeding a concealed lag back would let one erasure
            // define the next five subframes' idea of the talker.
            if !bad {
                self.erasure.note_good_subframe(pitch_lag, gains.pitch);
            }

            trace!("gain_pit", gains.pitch.0);
            trace!("L_gain_code", gains.code.0);
            // Scale the buffer first: the adaptive-only copy below must be at
            // the new scale, which is what the reference captures.
            let (q_new, gain_code_word) = self.excitation.rescale_to(gains.code);

            trace!("Q_new", q_new);
            trace!("gain_code_scaled", gain_code_word.0);
            // The adaptive-only excitation, before the total is built over it.
            // This is what voice_factor and the enhanced path start from.
            let (buffer, offset) = self.excitation.buffer_mut();
            let mut exc2 = [Word16(0); L_SUBFR];
            exc2.copy_from_slice(&buffer[offset..offset + L_SUBFR]);

            // The plain total, written back to the history.
            self.excitation.build(&code, gains.pitch, gain_code_word, q_new);
            // Snapshot this subframe, keeping the whole frame in one
            // exponent. `rescale_to` rescales the entire excitation history
            // when a subframe needs a different one, so the reference's buffer
            // is uniform by the end and a single `Scale_sig` undoes it.
            // Copying each subframe as built and scaling once at the end
            // instead mixes four exponents, and the energies a later SID
            // averages come out wrong on exactly the frames where q moved.
            if q_new != frame_q_new && sf > 0 {
                scale_sig(&mut ctx, &mut frame_excitation[..sf * L_SUBFR], q_new - frame_q_new);
            }
            frame_q_new = q_new;
            {
                let (buffer, offset) = self.excitation.buffer_mut();
                frame_excitation[sf * L_SUBFR..(sf + 1) * L_SUBFR]
                    .copy_from_slice(&buffer[offset..offset + L_SUBFR]);
            }

            #[cfg(test)]
            {
                let (b, o) = self.excitation.buffer_mut();
                let t: Vec<i16> = b[o..o + L_SUBFR].iter().map(|w| w.0).collect();
                self.vtrace.push(("exc_total", t));
            }
            // voice_factor works on the adaptive part scaled down by 3 bits.
            // Scale_sig rounds; a truncating shift here is wrong for half of
            // all samples and the error reaches the tilt, both enhancers, and
            // the low-rate sharpening.
            let mut scaled = exc2;
            scale_sig(&mut ctx, &mut scaled, -3);
            let voice_fac = voice_factor(
                &mut ctx,
                &scaled,
                -3,
                gains.pitch,
                &code,
                gain_code_word,
            );
            // Tilt for the next subframe: 0.5 voiced, 0 unvoiced.
            let quartered = shr(&mut ctx, voice_fac, 2);
            self.tilt_code = add(&mut ctx, quartered, Word16(8192));

            // At the low rates a strongly voiced subframe gets an extra
            // sharpened copy blended in later. It is built from the
            // adaptive-only excitation *before* the total is assembled over
            // it, so it has to be computed here.
            let pit_sharp = shl(&mut ctx, gains.pitch, 1);
            let sharpened = if frame_bits <= 177 && pit_sharp.0 > 16384 {
                let mut sharp_exc = [Word16(0); L_SUBFR];
                for (i, slot) in sharp_exc.iter_mut().enumerate() {
                    let widened = mult(&mut ctx, scaled[i], pit_sharp);
                    let product =
                        crate::fixed_point::arith32::l_mult(&mut ctx, widened, gains.pitch);
                    let halved = crate::fixed_point::shift::l_shr(&mut ctx, product, 1);
                    *slot = round(&mut ctx, halved);
                }
                Some(sharp_exc)
            } else {
                None
            };

            trace!("voice_fac", voice_fac.0);
            // The enhanced path, for the listener only.
            // Phase dispersion takes the HIGH HALF of the Q16 code gain, not
            // a rounded or rescaled version of it -- the reference splits
            // L_gain_code with L_Extract and passes the high word.
            let (gain_code_hi, _) = l_extract(gains.code);
            self.dispersion
                .apply(&mut ctx, &mut code, gain_code_hi, gains.pitch, dispersion);
            let enhanced_gain =
                self.noise_enhancer
                    .apply(&mut ctx, gains.code, voice_fac, stab_fac);
            let strength = enhance_strength(&mut ctx, voice_fac);
            let code2 = pitch_enhance_with(&mut ctx, &code, strength);

            let lifted = l_shl(&mut ctx, enhanced_gain, q_new);
            let enhanced_gain_word = round(&mut ctx, lifted);
            for i in 0..L_SUBFR {
                let mut acc =
                    crate::fixed_point::arith32::l_mult(&mut ctx, code2[i], enhanced_gain_word);
                acc = l_shl(&mut ctx, acc, 5);
                acc = crate::fixed_point::arith32::l_mac(&mut ctx, acc, exc2[i], gains.pitch);
                let acc = l_shl(&mut ctx, acc, 1);
                exc2[i] = round(&mut ctx, acc);
            }

            // Blend the sharpened copy in, matching its loudness to the
            // excitation it replaces so the sharpening is heard as periodicity
            // rather than as a level jump.
            if let Some(mut blended) = sharpened {
                for (i, slot) in blended.iter_mut().enumerate() {
                    *slot = add(&mut ctx, *slot, exc2[i]);
                }
                agc2(&mut ctx, &exc2, &mut blended);
                exc2 = blended;
            }

            vtrace!("exc2_final", exc2);
            // --- synthesis and high band ---
            // Extracted because the comfort-noise path drives exactly this
            // sequence with a different predictor and excitation; a second
            // copy would be a second thing to keep bit-exact.
            let a = coefficients[sf];
            let synthesised = self.synthesise(
                &mut ctx,
                &SynthesisInputs {
                    a: &a,
                    excitation: &exc2,
                    q_new,
                    subframe: sf,
                    frame_bits,
                    hf_gain: if bad { None } else { subframe.hf_gain },
                    isf: &isf,
                    isf_previous: &isf_previous,
                },
            );
            let base = sf * L_SUBFR16K;
            for (slot, sample) in out[base..base + L_SUBFR16K].iter_mut().zip(&synthesised) {
                *slot = sample.0;
            }

            self.excitation.advance();
        }

        // `dtx_dec_activity_update`, which sits below the comfort-noise return
        // in the reference and therefore runs only on frames that took this
        // path. It is the memory a `SID_FIRST` reconstructs the talker's
        // background from, so it must describe decoded speech and never the
        // comfort noise the decoder itself produced.
        // The whole frame's excitation, read out *after* the loop rather than
        // snapshotted per subframe. `rescale_to` rescales the entire history
        // when a subframe needs a different exponent, so a snapshot taken as
        // each subframe was built is in that subframe's own scaling and the
        // four do not share one. The reference reads the finished buffer and
        // applies a single `Scale_sig(exc, L_FRAME, -Q_new)`; anything else
        // mixes exponents and corrupts the energies a later SID averages.
        scale_sig(&mut ctx, &mut frame_excitation, -frame_q_new);
        self.dtx.observe_speech(&mut ctx, &isf, &frame_excitation);
        self.dtx.observe_vad(&mut ctx, params.vad_flag);
        self.dtx.commit(dtx_state);

        Some(out)
    }

    /// Decode a comfort-noise or empty frame.
    ///
    /// `payload` is the five bytes of a SID, or empty for `NO_DATA`. The
    /// caller supplies the frame type because it is a transport fact — a
    /// `SID_FIRST` and a `SID_UPDATE` are the same frame type on the wire and
    /// differ only in one bit of the payload, which
    /// [`super::bitstream::sid_is_update`] reads.
    ///
    /// Returns `None` when the frame is not comfort noise after all: a
    /// `NO_DATA` arriving while the stream is in speech is a gap for the
    /// concealer, not something to synthesise from.
    ///
    /// # Panics
    /// If a `SID_UPDATE`'s payload is not the five bytes of a SID frame.
    pub fn decode_comfort_noise(
        &mut self,
        frame_type: super::dtx::RxFrameType,
        payload: &[u8],
        mode_index: u8,
    ) -> Option<[i16; FRAME_SIZE_16K]> {
        let frame_bits = *FRAME_BITS.get(mode_index as usize)?;
        let mut ctx = DspContext::default();
        let state = self.dtx.receive(&mut ctx, frame_type);
        if state == super::dtx::DtxState::Speech {
            // The reference's own table: `RX_NO_DATA | SPEECH -> SPEECH`, and
            // the SPEECH branch conceals. A gap arriving mid-talk-spurt is a
            // lost frame, which happens whenever the `SID_FIRST` that opened a
            // silence was itself lost. Refusing it here dropped every
            // following `NO_DATA` until speech resumed.
            //
            // The state machine has already been advanced by the `receive`
            // above and must not run again, hence the pre-decided state.
            let mode =
                AmrMode::new(crate::codecs::amr::mode::AmrVariant::WideBand, mode_index).ok()?;
            return self.decode_frame_with_state(
                mode,
                &[],
                FrameQuality::Unusable,
                Some(state),
            );
        }
        // Only a SID_UPDATE's bits are worth reading. A SID_FIRST's are blank
        // and a SID_BAD's are not trusted; both fall back to what the decoder
        // already had, or to its own backward analysis.
        let sid = if frame_type == super::dtx::RxFrameType::SidUpdate {
            super::bitstream::parse_sid(payload).map(|(isf_indices, energy, dither)| {
                super::dtx::SidFields {
                    isf_indices,
                    energy_index: Word16(i16::try_from(energy).expect("six bits")),
                    dither,
                }
            })
        } else {
            None
        };
        Some(self.synthesise_comfort_noise(&mut ctx, state, frame_bits, sid))
    }

    /// Synthesise one frame of comfort noise.
    ///
    /// Four differences from the speech path, all of them the reference's:
    /// one predictor for the whole frame rather than four interpolated ones,
    /// a `Q_new` of zero, a fixed high-band gain at 23.85 kbit/s when the SID
    /// arrived intact, and a partial decoder reset afterwards.
    ///
    /// The 6.60 kbit/s high band does *not* take its usual extrapolated
    /// shaper here: the reference guards that branch on the frame being
    /// speech, so comfort noise uses the order-16 shaper — and, unlike the
    /// speech path, never clears its wide tail.
    fn synthesise_comfort_noise(
        &mut self,
        ctx: &mut DspContext,
        state: super::dtx::DtxState,
        frame_bits: usize,
        sid: Option<super::dtx::SidFields>,
    ) -> [i16; FRAME_SIZE_16K] {
        let (isf, excitation) = self.dtx.comfort_noise(ctx, state, sid);
        let isp = isf_to_isp(&isf);
        // One predictor, used for all four subframes. There is no
        // interpolation here and no `isp_old` update: the partial reset below
        // makes the next speech frame re-seed instead.
        // `Isp_Az(ispnew, Aq, M, 1)`: adaptive scaling *enabled*, which is
        // unique to this call. Every speech path passes 0.
        let a = super::lp::isp_to_lp::isp_to_lp_adaptive(&isp);

        let mut out = [0i16; FRAME_SIZE_16K];
        for sf in 0..NB_SUBFR {
            let sub: [Word16; L_SUBFR] = excitation[sf * L_SUBFR..(sf + 1) * L_SUBFR]
                .try_into()
                .expect("one subframe");
            // 23.85 kbit/s applies a fixed gain rather than the tilt estimate
            // while comfort noise is playing, but only where the SID's bits
            // were usable.
            let hf_gain = if frame_bits >= 477 && sid.is_some() {
                Some(1)
            } else {
                None
            };
            let isf_previous = self.isf_old;
            let synthesised = self.synthesise(
                ctx,
                &SynthesisInputs {
                    a: &a,
                    excitation: &sub,
                    q_new: 0,
                    subframe: sf,
                    // Reported as a wideband frame so the 6.60 extrapolation
                    // branch is not taken; see the doc comment.
                    frame_bits: frame_bits.max(133),
                    hf_gain,
                    isf: &isf,
                    isf_previous: &isf_previous,
                },
            );
            let base = sf * L_SUBFR16K;
            for (slot, sample) in out[base..base + L_SUBFR16K].iter_mut().zip(&synthesised) {
                *slot = sample.0;
            }
        }

        self.reset_after_comfort_noise();
        self.isf_old = isf;
        self.dtx.commit(state);
        out
    }

    /// `Reset_decoder(st, 0)`: what a comfort-noise frame clears.
    ///
    /// Not the seeds, not the high-band filter memories and not the DTX state
    /// — the next speech frame has to continue the same noise. `started`
    /// going back to false is how the ISP memory recovers: the frame after
    /// comfort noise re-seeds rather than interpolating from a stale value.
    fn reset_after_comfort_noise(&mut self) {
        // `Reset_decoder(st, 0)`. The excitation history and its scaling, the
        // ISF predictor, the pitch-lag history, the innovation tilt, the phase
        // dispersion memory and the noise enhancer's slow threshold.
        self.excitation = super::excitation::Excitation::new();
        self.isf = IsfDecoder::new();
        self.erasure = Erasure::new();
        self.dispersion = PhaseDispersion::new();
        self.noise_enhancer = NoiseEnhancer::new();
        self.tilt_code = Word16(0);
        // Not the seeds, not a single high-band filter memory, not the
        // synthesis or de-emphasis state, and not `isf_old` -- the caller
        // writes the comfort-noise spectrum there afterwards. The next speech
        // frame has to continue the same background, not restart it.
        self.started = false;
    }
}


#[cfg(test)]
mod comfort_noise_tests {
    use super::*;
    use crate::codecs::amr::{storage, AmrFrameType, AmrVariant};

    /// The whole DTX stream the reference produced, decoded frame by frame.
    ///
    /// **Not a bit-exactness claim.** What this asserts is that every frame
    /// type in a real DTX stream is accepted and produces a full frame, that
    /// comfort noise is synthesised rather than silence returned, and that the
    /// seven speech frames before the first SID are still sample-exact — which
    /// is what says extracting the synthesis changed nothing.
    ///
    /// The comfort-noise frames are *not* yet exact:
    /// `the_comfort_noise_synthesis_is_not_yet_bit_exact` below measures how
    /// far off, and is ignored rather than deleted so the gap stays visible.
    #[test]
    fn a_real_dtx_stream_decodes_through_every_frame_type() {
        let bits: &[u8] = include_bytes!("../testdata/amrwb_dtx_mode2.amr");
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
        let cn_mode = 2u8;

        let mut decoder = Decoder::new();
        let mut speech = 0usize;
        let mut comfort = 0usize;
        let mut gaps = 0usize;
        let mut silent_comfort = 0usize;
        let mut got: Vec<i16> = Vec::new();

        for (n, frame) in frames.iter().enumerate() {
            let out = match frame.frame_type {
                AmrFrameType::Speech(_) => {
                    speech += 1;
                    decoder
                        .decode_frame(mode, &frame.data, FrameQuality::Good)
                        .unwrap_or_else(|| panic!("speech frame {n} refused"))
                }
                AmrFrameType::Sid(_) => {
                    comfort += 1;
                    let update = crate::codecs::amr::wb::bitstream::sid_is_update(&frame.data);
                    let kind = if update {
                        crate::codecs::amr::wb::dtx::RxFrameType::SidUpdate
                    } else {
                        crate::codecs::amr::wb::dtx::RxFrameType::SidFirst
                    };
                    let out = decoder
                        .decode_comfort_noise(kind, &frame.data, cn_mode)
                        .unwrap_or_else(|| panic!("SID frame {n} refused"));
                    if out.iter().all(|&s| s == 0) {
                        silent_comfort += 1;
                    }
                    out
                }
                AmrFrameType::NoData => {
                    gaps += 1;
                    decoder
                        .decode_comfort_noise(
                            crate::codecs::amr::wb::dtx::RxFrameType::NoData,
                            &[],
                            cn_mode,
                        )
                        .unwrap_or_else(|| panic!("gap frame {n} refused"))
                }
                other @ AmrFrameType::SpeechLost => {
                    panic!("unexpected frame type {other:?} at {n}")
                }
            };
            assert_eq!(out.len(), FRAME_SIZE_16K);
            got.extend_from_slice(&out);
        }

        // And against the reference decoder's own output for this exact
        // stream, masked to the 14 bits AMR-WB defines.
        let want: Vec<i16> = include_bytes!("../testdata/amrwb_dtx_mode2.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(got.len(), want.len(), "frame count differs");
        // Every speech frame before the first SID is exact, which is what
        // says the extraction above changed nothing.
        for (i, (&mine, &theirs)) in got.iter().zip(&want).take(7 * FRAME_SIZE_16K).enumerate() {
            assert_eq!(mine & !3, theirs, "speech sample {i} differs");
        }

        assert_eq!(speech, 77, "the fixture's speech count moved");
        assert_eq!(comfort, 12, "the fixture's SID count moved");
        assert_eq!(gaps, 61, "the fixture's gap count moved");
        assert_eq!(
            silent_comfort, 0,
            "comfort noise came out silent, which is what a stubbed CN path returns"
        );
    }

    /// The whole DTX stream, sample for sample against the reference decoder.
    ///
    /// Speech, comfort noise and gaps, 150 frames, including two speech-to-DTX
    /// transitions and two back. Two defects had to go before this passed, and
    /// neither was in the DTX kernel:
    ///
    /// The background energy history is measured on the excitation *brought
    /// back out of the frame's scaling* — the reference does
    /// `Scale_sig(exc, L_FRAME, -Q_new)` first. Skipping it inflated every
    /// stored energy by `2^Q_new`, which reached the output as comfort noise
    /// hundreds of times too loud while the spectrum was already exact.
    ///
    /// And `Reset_decoder(st, 0)` clears more than the excitation: the ISF
    /// predictor, the pitch-lag history, the innovation tilt, the phase
    /// dispersion memory and the noise enhancer's threshold. An incomplete
    /// reset is invisible until the first speech frame *after* the silence.
    #[test]
    fn the_whole_dtx_stream_matches_the_reference_decoder() {
        let bits: &[u8] = include_bytes!("../testdata/amrwb_dtx_mode2.amr");
        let want: Vec<i16> = include_bytes!("../testdata/amrwb_dtx_mode2.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
        let cn_mode = 2u8;

        let mut decoder = Decoder::new();
        let mut got: Vec<i16> = Vec::new();
        // Speech, SID, gap. Pinned below, because a stream that turned out to
        // be all speech would exercise none of the comfort-noise path this
        // test exists for.
        let mut kinds = [0usize; 3];
        for frame in &frames {
            let out = match frame.frame_type {
                AmrFrameType::Speech(_) => {
                    kinds[0] += 1;
                    decoder
                        .decode_frame(mode, &frame.data, FrameQuality::Good)
                        .expect("speech decodes")
                }
                AmrFrameType::Sid(_) => {
                    let kind = if crate::codecs::amr::wb::bitstream::sid_is_update(&frame.data) {
                        crate::codecs::amr::wb::dtx::RxFrameType::SidUpdate
                    } else {
                        crate::codecs::amr::wb::dtx::RxFrameType::SidFirst
                    };
                    kinds[1] += 1;
                    decoder
                        .decode_comfort_noise(kind, &frame.data, cn_mode)
                        .expect("SID decodes")
                }
                _ => {
                    kinds[2] += 1;
                    decoder
                        .decode_comfort_noise(
                            crate::codecs::amr::wb::dtx::RxFrameType::NoData,
                            &[],
                            mode.index(),
                        )
                        .expect("gap decodes")
                }
            };
            got.extend_from_slice(&out);
        }

        // `zip` below stops at the shorter side, so without these a truncated
        // decode compares its own prefix and passes. Measured: a mutation
        // cutting the fixture to three frames left this test green.
        assert_eq!(kinds, [77, 12, 61], "the fixture's frame-type mix changed");
        assert_eq!(got.len(), want.len(), "decoded {} samples, want {}", got.len(), want.len());
        assert_eq!(got.len(), 150 * FRAME_SIZE_16K, "the fixture is 150 frames");

        for (i, (&mine, &theirs)) in got.iter().zip(&want).enumerate() {
            assert_eq!(
                mine & !3,
                theirs,
                "sample {i} differs, in frame {}",
                i / FRAME_SIZE_16K
            );
        }
    }

    /// A gap while the stream is in speech is concealed, not synthesised as
    /// comfort noise and not refused.
    ///
    /// `NO_DATA` means the sender chose to send nothing, which during a talk
    /// spurt is a transport anomaly rather than a DTX statement — so the
    /// reference keeps it on the speech path, where it is concealed. This
    /// previously returned `None`, which the caller turned into an error and
    /// therefore a dropped packet.
    ///
    /// It is not a rare corner: lose the `SID_FIRST` that opens a silence and
    /// every `NO_DATA` for the rest of that gap arrives while the state
    /// machine still believes the stream is speaking.
    #[test]
    fn a_gap_during_speech_is_concealed_rather_than_refused() {
        let mut decoder = Decoder::new();
        let out = decoder
            .decode_comfort_noise(crate::codecs::amr::wb::dtx::RxFrameType::NoData, &[], 2)
            .expect("a gap during speech is concealed");
        assert_eq!(out.len(), FRAME_SIZE_16K);
    }
}

/// Everything one subframe's synthesis and high band needs.
struct SynthesisInputs<'a> {
    /// The predictor for this subframe.
    a: &'a [Word16; LP_ORDER + 1],
    /// The excitation to drive it with.
    excitation: &'a [Word16; L_SUBFR],
    /// Its scaling.
    q_new: i16,
    /// Which subframe, for the 6.60 kbit/s ISF interpolation.
    subframe: usize,
    /// The frame's bit budget, which selects the high-band shaping.
    frame_bits: usize,
    /// The transmitted high-band gain index, where there is one to believe.
    hf_gain: Option<u16>,
    /// This frame's spectrum and the previous one's, for 6.60's extrapolation.
    isf: &'a [Word16; LP_ORDER],
    isf_previous: &'a [Word16; LP_ORDER],
}

impl Decoder {
    /// Synthesise one subframe and add its high band, `cod_main.c`'s
    /// `synthesis()`.
    ///
    /// Shared between the speech path and the comfort-noise one, which drives
    /// the identical sequence with one predictor for the whole frame, a
    /// `Q_new` of zero and — at 23.85 kbit/s — a fixed high-band gain rather
    /// than the tilt estimate.
    fn synthesise(
        &mut self,
        ctx: &mut DspContext,
        inputs: &SynthesisInputs<'_>,
    ) -> [Word16; L_SUBFR16K] {
        let (high, low) = self.synthesis.filter(inputs.a, inputs.excitation, inputs.q_new);
        let mut speech = deemphasis(&high, &low, PREEMPH_FAC, &mut self.deemph_memory);
        #[cfg(test)]
        self.vtrace
            .push(("deemph", speech.iter().map(|w: &Word16| w.0).collect()));
        self.high_pass.filter(&mut speech);
        #[cfg(test)]
        self.vtrace
            .push(("hp50", speech.iter().map(|w: &Word16| w.0).collect()));
        let upsampled = self.upsampler.process(&speech);
        #[cfg(test)]
        self.vtrace
            .push(("upsampled", upsampled.iter().map(|w: &Word16| w.0).collect()));

        // --- high band ---
        let mut hf = self.noise.fill(ctx);
        let mut energy_source = *inputs.excitation;
        scale_sig(ctx, &mut energy_source, -3);
        highband::match_energy(ctx, &energy_source, &mut hf, inputs.q_new - 3);

        let tilt = highband::spectral_tilt(ctx, &mut self.tilt_filter, &mut speech);
        // 23.85's four transmitted gain bits are worth no more than the
        // frame that carried them, so a bad frame falls back to the same
        // tilt estimate every other rate uses. The bits are still parsed —
        // they are part of the layout — just not believed.
        let transmitted = inputs.hf_gain;
        if let Some(index) = transmitted {
            // 23.85 transmits the gain, and applies it with an extra
            // doubling the estimated path does not have: the reference is
            // shl(mult(HF, gain), 1), not mult alone.
            let gain = highband::transmitted_gain(index);
            for s in &mut hf {
                let scaled = mult(ctx, *s, gain);
                *s = shl(ctx, scaled, 1);
            }
        } else {
            let gain = highband::gain_from_tilt(ctx, tilt, self.vad_history);
            for s in &mut hf {
                *s = mult(ctx, *s, gain);
            }
        }
        #[cfg(test)]
        self.vtrace
            .push(("hfnoise_scaled", hf.iter().map(|w: &Word16| w.0).collect()));
        if inputs.frame_bits <= 132 {
            // 6.60 kbit/s has too little spectral detail to borrow the low
            // band's filter, so a wider one is extrapolated from the ISF
            // spacing. Note the plain complement here: this interpolation
            // is *not* the one `interpolate_isp` performs, which adds one.
            let mut hf_isf = vec![Word16(0); highband::M16K_ORDER];
            let frac = INTERPOL_FRAC[inputs.subframe];
            let complement = sub(ctx, Word16(32767), frac);
            for (i, slot) in hf_isf.iter_mut().enumerate().take(LP_ORDER) {
                let acc = l_mult(ctx, inputs.isf_previous[i], complement);
                let acc = l_mac(ctx, acc, inputs.isf[i], frac);
                *slot = round(ctx, acc);
            }
            highband::extrapolate_isf(ctx, &mut hf_isf);

            let mut hf_a = vec![Word16(0); highband::M16K_ORDER + 1];
            isp_to_lp_order(&hf_isf, &mut hf_a);
            self.shaper.shape_wide(ctx, &hf_a, &mut hf);
        } else {
            self.shaper.clear_wide_tail();
            self.shaper.shape(ctx, inputs.a, &mut hf);
        }
        self.band_pass.filter(ctx, &mut hf);
        if inputs.frame_bits >= 477 {
            self.low_pass_7k.filter(ctx, &mut hf);
        }

        #[cfg(test)]
        self.vtrace
            .push(("hfband", hf.iter().map(|w: &Word16| w.0).collect()));

        let mut combined = [Word16(0); L_SUBFR16K];
        for i in 0..L_SUBFR16K {
            combined[i] = add(ctx, upsampled[i], hf[i]);
        }
        combined
    }
}

/// A parameter set for a frame that never arrived.
///
/// Only reachable from [`Decoder::decode_frame`] with
/// [`FrameQuality::Unusable`], where the concealment path overrides every
/// field: the lag comes from the pitch contour, the innovation from the noise
/// generator, the gains and spectrum from history, and the LTP filter is forced
/// off. The values here exist so the subframe loop has a shape to walk, not
/// because any of them is a guess worth making.
fn placeholder_params() -> FrameParams {
    let subframe = SubframeParams {
        pitch_lag: 64,
        pitch_frac: 0,
        ltp_filter: false,
        gain_index: 0,
        pulses: Vec::new(),
        hf_gain: None,
    };
    FrameParams {
        vad_flag: false,
        isf_indices: Vec::new(),
        subframes: vec![subframe; NB_SUBFR],
        hf_gains: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrFrameType;
    use crate::codecs::amr::mode::AmrVariant;
    use crate::codecs::amr::storage;

    fn fixture(mode_index: usize) -> (&'static [u8], &'static [u8]) {
        const BITS: [&[u8]; 9] = [
            include_bytes!("../testdata/amrwb_mode0.amr"),
            include_bytes!("../testdata/amrwb_mode1.amr"),
            include_bytes!("../testdata/amrwb_mode2.amr"),
            include_bytes!("../testdata/amrwb_mode3.amr"),
            include_bytes!("../testdata/amrwb_mode4.amr"),
            include_bytes!("../testdata/amrwb_mode5.amr"),
            include_bytes!("../testdata/amrwb_mode6.amr"),
            include_bytes!("../testdata/amrwb_mode7.amr"),
            include_bytes!("../testdata/amrwb_mode8.amr"),
        ];
        const PCM: [&[u8]; 9] = [
            include_bytes!("../testdata/amrwb_mode0.pcm"),
            include_bytes!("../testdata/amrwb_mode1.pcm"),
            include_bytes!("../testdata/amrwb_mode2.pcm"),
            include_bytes!("../testdata/amrwb_mode3.pcm"),
            include_bytes!("../testdata/amrwb_mode4.pcm"),
            include_bytes!("../testdata/amrwb_mode5.pcm"),
            include_bytes!("../testdata/amrwb_mode6.pcm"),
            include_bytes!("../testdata/amrwb_mode7.pcm"),
            include_bytes!("../testdata/amrwb_mode8.pcm"),
        ];
        (BITS[mode_index], PCM[mode_index])
    }

    /// Reference PCM as samples.
    fn reference(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    /// Decode a mode and report where it first departs from the reference.
    fn compare(mode_index: usize) -> (usize, usize, i64) {
        compare_from(mode_index, 0)
    }

    /// As [`compare`], but skipping the first `skip` frames while the filter
    /// memories fill.
    fn compare_from(mode_index: usize, skip: usize) -> (usize, usize, i64) {
        let (bits, pcm) = fixture(mode_index);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode =
            AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("index"))
                .expect("mode");

        let mut dec = Decoder::new();
        let mut matched = 0usize;
        let mut total = 0usize;
        let mut worst = 0i64;

        for (f, frame) in frames.iter().enumerate() {
            let Some(got) = dec.decode(mode, &frame.data) else {
                break;
            };
            if f < skip {
                continue;
            }
            for (i, &g) in got.iter().enumerate() {
                let index = f * FRAME_SIZE_16K + i;
                if index >= want.len() {
                    break;
                }
                total += 1;
                // AMR-WB's output is defined as 14-bit linear: the reference
                // harness masks the low two bits before writing (decoder.c
                // line 211, `synth[i] & 0xfffC`). Comparing unmasked 16-bit
                // output against it is comparing different things.
                let g = g & !3i16;
                let delta = i64::from(g) - i64::from(want[index]);
                worst = worst.max(delta.abs());
                if delta == 0 {
                    matched += 1;
                }
            }
        }
        (matched, total, worst)
    }

    /// Diagnostic for the assembly's remaining error, kept because it is how
    /// the next step starts rather than because it asserts anything.
    ///
    /// Run with `--ignored --nocapture`, and compare against the instrumented
    /// reference produced by `tools/trace-amr-reference.sh`. The two emit the
    /// same intermediates under the same names.
    ///
    /// Verified matching for mode 12.65, frame 0, subframes 0 and 1: every
    /// scalar (`T0`, gains, `Q_new`, `voice_fac`) and every vector (`pred`,
    /// `code`, `exc_total`, `exc2_final`, `hfband`) is identical to the
    /// reference. The remaining error is therefore downstream of the
    /// excitation.
    #[test]
    #[ignore = "diagnostic, not an assertion"]
    fn where_does_it_diverge() {
        let mode_index = 2;
        let (bits, pcm) = fixture(mode_index);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");
        let mut dec = Decoder::new();
        let got = dec.decode(mode, &frames[0].data).expect("decodes");

        for (k, v) in &dec.trace {
            println!("S {k} = {v}");
        }
        // Per-frame accuracy: uniform means a per-sample residual, degrading
        // means state drifting between frames. They need different fixes.
        let mut dec2 = Decoder::new();
        for (f, frame) in frames.iter().enumerate().take(8) {
            let g = dec2.decode(mode, &frame.data).expect("decodes");
            let base = f * FRAME_SIZE_16K;
            let n = (0..FRAME_SIZE_16K)
                .filter(|&i| (g[i] & !3i16) == want[base + i])
                .count();
            let worst = (0..FRAME_SIZE_16K)
                .map(|i| (i32::from(g[i] & !3i16) - i32::from(want[base + i])).abs())
                .max()
                .unwrap_or(0);
            println!("  frame {f}: {n}/320 exact, worst {worst}");
        }
        let first_bad = (0..FRAME_SIZE_16K).find(|&i| got[i] != want[i]);
        let first_nz = (0..FRAME_SIZE_16K).find(|&i| want[i] != 0);
        println!("first non-zero {first_nz:?}, first mismatch {first_bad:?}");
        println!("got  {:?}", &got[20..30]);
        println!("want {:?}", &want[20..30]);

        for sf in 0..4 {
            let n = (sf * 80..(sf + 1) * 80).filter(|&i| got[i] == want[i]).count();
            println!("  subframe {sf}: {n}/80 exact");
        }
    }


    /// The 6.60 kbit/s high band uses a different filter from every other mode.
    ///
    /// It extrapolates an order-20 predictor from the ISF spacing rather than
    /// borrowing the low band's order-16 one. Guarding it separately because a
    /// regression there would show up only at this one rate, and only in the
    /// 5.5-7.5 kHz band, which is easy to miss in an aggregate figure.
    #[test]
    fn the_low_rate_high_band_uses_the_extrapolated_filter() {
        let (matched, total, worst) = compare(0);
        assert_eq!(matched, total, "6.60 kbit/s is not exact, worst {worst}");
    }


    /// 6.60 kbit/s is the one mode not yet bit-exact.
    ///
    /// It shapes its high band with an extrapolated order-20 filter rather
    /// than the low band's own. That path is implemented and tested
    /// ([`super::highband::extrapolate_isf`]) but not wired, because
    /// `isp_to_lp` and the noise shaper are both fixed at order 16. The
    /// residual is one LSB of the 14-bit output, so this bounds it rather than
    /// asserting exactness.
    #[test]
    fn the_low_rate_mode_is_close_but_not_yet_exact() {
        for mode_index in 0..1 {
            let (bits, pcm) = fixture(mode_index);
            let want = reference(pcm);
            let (_, frames) = storage::read(bits).expect("fixture parses");
            let mode =
                AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("index"))
                    .expect("mode");

            let mut dec = Decoder::new();
            let mut worst = 0i32;
            for (f, frame) in frames.iter().enumerate() {
                let got = dec.decode(mode, &frame.data).expect("decodes");
                // Two frames of warm-up: the synthesis, de-emphasis, high-pass
                // and resampler memories all start empty.
                if f < 2 {
                    continue;
                }
                for (i, &g) in got.iter().enumerate() {
                    let index = f * FRAME_SIZE_16K + i;
                    if index >= want.len() {
                        break;
                    }
                    let delta = i32::from(g & !3i16) - i32::from(want[index]);
                    worst = worst.max(delta.abs());
                }
            }
            assert!(
                worst <= 4,
                "mode {mode_index}: worst error {worst}, above one 14-bit LSB"
            );
        }
    }

    /// The erased 12.65 kbit/s stream and the reference's output for it.
    fn erased_fixture() -> (Vec<crate::codecs::amr::AmrPayloadFrame>, Vec<i16>) {
        let bits: &[u8] = include_bytes!("../testdata/amrwb_erased.amr");
        let want = reference(include_bytes!("../testdata/amrwb_erased.pcm"));
        let (_, frames) = storage::read(bits).expect("fixture parses");
        (frames, want)
    }

    /// Bit-exact against TS 26.173 on a stream with erased frames.
    ///
    /// Concealment is where a decoder is least likely to be right by accident:
    /// it is pure state machine, it only runs once something has already gone
    /// wrong, and getting it wrong sounds like a bad network rather than like a
    /// bug. Every other fixture here is a clean stream, so until this existed
    /// the `Bad` path had never been taken end to end.
    ///
    /// The erasure pattern in `tools/build-amr-erasure-fixtures.sh` is chosen
    /// to move the state machine rather than to be representative: one isolated
    /// loss, a burst of three, the first good frame after the burst — where the
    /// gain concealer limits against the last known-good value — and then an
    /// alternating pair that keeps the severity counter from settling. That
    /// last part matters more for wideband than narrowband, because wideband
    /// *halves* the counter on a good frame instead of clearing it, so the
    /// alternating frames leave it at a value a clear-on-good machine never
    /// visits.
    #[test]
    fn concealment_matches_the_reference_sample_for_sample() {
        let (frames, want) = erased_fixture();
        let erased: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.quality_ok)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            erased,
            vec![5, 10, 11, 12, 20, 22],
            "the fixture's erasure pattern moved; this test assumes it"
        );
        assert_eq!(frames.len(), 25, "the fixture should hold 25 frames");
        assert_eq!(
            want.len(),
            frames.len() * FRAME_SIZE_16K,
            "the reference PCM does not cover every frame"
        );

        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");
        let mut dec = Decoder::new();
        let mut compared = 0usize;

        for (f, frame) in frames.iter().enumerate() {
            // A cleared storage quality bit is SPEECH_BAD, not SPEECH_LOST:
            // the payload arrived and is still decoded for its pulses. Passing
            // `Unusable` here would substitute noise for perfectly good bits.
            let quality = if frame.quality_ok {
                FrameQuality::Good
            } else {
                FrameQuality::Bad
            };
            let got = dec
                .decode_frame(mode, &frame.data, quality)
                .expect("frame decodes");
            for (i, &sample) in got.iter().enumerate() {
                let index = f * FRAME_SIZE_16K + i;
                assert_eq!(
                    sample & !3i16,
                    want[index],
                    "frame {f} ({quality:?}) sample {i}"
                );
                compared += 1;
            }
        }
        assert_eq!(
            compared,
            want.len(),
            "compared fewer samples than the fixture holds"
        );

        // Negative control. If the erasures did not actually reach the
        // concealment path — a quality bit that failed to take, a `bad` flag
        // that never left `decode` — then decoding the same stream as if it
        // were clean would produce the same audio, and the comparison above
        // would be asserting nothing at all.
        let mut naive = Decoder::new();
        let mut differs = false;
        for (f, frame) in frames.iter().enumerate() {
            let got = naive.decode(mode, &frame.data).expect("frame decodes");
            for (i, &sample) in got.iter().enumerate() {
                if (sample & !3i16) != want[f * FRAME_SIZE_16K + i] {
                    differs = true;
                }
            }
        }
        assert!(
            differs,
            "decoding the erased stream as clean matched the reference, so \
             concealment never ran"
        );
    }

    /// The erasure state machine's own bookkeeping, checked against the
    /// reference's per-frame trace rather than against the audio it produces.
    ///
    /// `prev_bfi` is a *frame* flag in TS 26.173 — set for all four subframes
    /// of a recovery frame — and the severity counter halves on a good frame
    /// rather than clearing. Both are invisible in a per-sample comparison
    /// until they are wrong for several frames in a row, so they are pinned
    /// directly.
    #[test]
    fn the_erasure_state_machine_tracks_the_reference_frame_by_frame() {
        let (frames, _) = erased_fixture();
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");
        let mut dec = Decoder::new();

        // From the instrumented reference on this exact fixture: severity at
        // the top of each frame, then whether the frame before it was bad.
        let want_state = [
            0usize, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 3, 1, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0,
        ];
        let want_prev = [
            false, false, false, false, false, false, true, false, false, false, false, true,
            true, true, false, false, false, false, false, false, false, true, false, true,
            false,
        ];

        for (f, frame) in frames.iter().enumerate() {
            let quality = if frame.quality_ok {
                FrameQuality::Good
            } else {
                FrameQuality::Bad
            };
            dec.decode_frame(mode, &frame.data, quality)
                .expect("frame decodes");
            let scalar = |name: &str| {
                let Some((_, value)) = dec.trace.iter().find(|(k, _)| *k == name) else {
                    panic!("frame {f}: no {name} in the trace");
                };
                *value
            };
            assert_eq!(
                usize::try_from(scalar("state")).expect("state is non-negative"),
                want_state[f],
                "frame {f}: severity counter"
            );
            assert_eq!(
                scalar("prev_bfi") == 1,
                want_prev[f],
                "frame {f}: previous-frame flag"
            );
        }
    }

    /// A frame that never arrived needs no payload at all.
    ///
    /// The lost path reads nothing from the bitstream — innovation from the
    /// noise generator, lag from the pitch contour, gains and spectrum from
    /// history — so a receiver holding only a gap in sequence numbers can still
    /// ask for a frame's worth of audio.
    #[test]
    fn a_lost_frame_decodes_without_a_payload() {
        let (bits, _) = fixture(2);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");

        let mut dec = Decoder::new();
        for frame in frames.iter().take(4) {
            dec.decode(mode, &frame.data).expect("decodes");
        }
        let concealed = dec
            .decode_frame(mode, &[], FrameQuality::Unusable)
            .expect("a lost frame still produces audio");
        assert_eq!(concealed.len(), FRAME_SIZE_16K);
        assert!(
            concealed.iter().any(|&s| s != 0),
            "a lost frame produced silence rather than concealment"
        );
    }

    /// The `SPEECH_LOST` path, against the reference, for a whole stream.
    ///
    /// A lost frame and a damaged one are different inputs, not two names for
    /// one: a damaged frame still carries usable codebook pulses and an LTP
    /// filter select bit, and the reference decodes both, while a lost frame's
    /// innovation becomes noise. The fixture generator asserts the two decode
    /// differently, so conflating them would fail visibly here rather than
    /// merely be wrong.
    #[test]
    fn a_lost_stream_matches_the_reference_sample_for_sample() {
        let bits: &[u8] = include_bytes!("../testdata/amrwb_lost.amr");
        let want: Vec<i16> = include_bytes!("../testdata/amrwb_lost.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");

        let lost: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.frame_type == AmrFrameType::SpeechLost)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            lost,
            vec![5, 10, 11, 12, 20, 22],
            "the fixture's loss pattern moved; this test assumes it"
        );
        assert_eq!(want.len(), frames.len() * FRAME_SIZE_16K);

        let mut dec = Decoder::new();
        let mut compared = 0usize;
        for (f, frame) in frames.iter().enumerate() {
            // A lost frame carries no mode of its own, so the caller supplies
            // the one the stream was last using. The decoder does not remember
            // it, deliberately: only the receiver knows what the sequence
            // numbers imply.
            let quality = if frame.frame_type == AmrFrameType::SpeechLost {
                FrameQuality::Unusable
            } else {
                FrameQuality::Good
            };
            let got = dec
                .decode_frame(mode, &frame.data, quality)
                .expect("frame decodes");
            for (i, &sample) in got.iter().enumerate() {
                assert_eq!(
                    sample & !3,
                    want[f * FRAME_SIZE_16K + i],
                    "frame {f} ({quality:?}) sample {i}"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, want.len(), "compared fewer samples than the fixture holds");
    }

    /// A sustained outage must fade out, not hold a tone.
    ///
    /// The severity counter is what makes this happen, and it is the one piece
    /// of the state machine that differs between the variants — wideband halves
    /// it on a good frame where narrowband clears it. A decoder that never
    /// advances it buzzes indefinitely on the last good pitch pulse, which is
    /// the classic sound of concealment that compiles but does not run.
    #[test]
    fn a_long_outage_fades_rather_than_buzzing() {
        let (bits, _) = fixture(2);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");

        let mut dec = Decoder::new();
        for frame in frames.iter().take(6) {
            dec.decode(mode, &frame.data).expect("decodes");
        }

        let energy = |f: &[i16; FRAME_SIZE_16K]| -> i64 {
            f.iter().map(|&s| i64::from(s).abs()).sum()
        };
        let first = energy(
            &dec.decode_frame(mode, &[], FrameQuality::Unusable)
                .expect("decodes"),
        );
        let mut last = first;
        for _ in 0..11 {
            last = energy(
                &dec.decode_frame(mode, &[], FrameQuality::Unusable)
                    .expect("decodes"),
            );
        }
        assert!(
            last * 4 < first,
            "twelve lost frames only fell from {first} to {last}"
        );
    }

    /// Bit-exact against TS 26.173 for every mode.
    ///
    /// The conformance claim: every sample of every frame of every fixture is
    /// identical to the reference decoder's output, at all nine rates.
    ///
    /// The sample count is pinned, not floored. Its narrowband counterpart
    /// asserts `total > 0`; this had neither, and a mutation that truncated
    /// the fixture to three frames left it green while its siblings failed.
    /// An equality is stronger than a floor and costs nothing: the fixture is
    /// 25 frames of 320 samples and is not going to change silently.
    #[test]
    fn the_decoder_matches_the_reference_sample_for_sample() {
        for mode_index in 0..9 {
            let (matched, total, worst) = compare(mode_index);
            assert_eq!(
                total,
                25 * FRAME_SIZE_16K,
                "mode {mode_index} compared {total} samples; the fixture is 25 frames"
            );
            assert_eq!(
                matched, total,
                "mode {mode_index}: {matched} of {total} samples match, worst error {worst}"
            );
        }
    }
}
