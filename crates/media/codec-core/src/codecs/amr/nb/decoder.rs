//! The assembled AMR-NB decoder, TS 26.073 `dec_amr.c` and `sp_dec.c`.
//!
//! Payload bytes in, 160 samples of 8 kHz speech out. Every stage this drives
//! is separately bit-exact against the reference; what lives here is the
//! *wiring* — which state is carried where, in what order, and which of two
//! excitations each consumer gets.
//!
//! # Why the wiring is the hard part
//!
//! On the wideband side every stage was bit-exact in isolation while the
//! assembled decoder scored 1–3%. The errors were all in the composition:
//! state carried between stages, order of operations, and one interface
//! matched to the wrong thing. Three things are therefore explicit here rather
//! than left to read off the call order.
//!
//! **Two excitations, and they diverge.** The excitation written back into the
//! adaptive-codebook history is *not* the one the synthesis filter consumes.
//! Phase dispersion, the excitation-control module and the pitch-sharpened
//! copy all operate on a separate buffer. Using one buffer for both produces
//! audio that is recognisably speech and steadily wrong — the wideband port
//! nearly shipped exactly that.
//!
//! **The lag decoders read the pre-update previous lag.** The bad-frame
//! graceful degradation increments it, and 10.2's pitch-sharpening attenuation
//! then reads the *incremented* value. Two reads of one variable, on either
//! side of one write.
//!
//! **`Overflow` is cleared immediately before the synthesis filter**, not once
//! per frame. Testing an accumulated flag rescales the whole 194-sample
//! excitation history on the strength of an overflow that happened somewhere
//! else entirely.
//!
//! # Tracing
//!
//! Under `cfg(test)` each frame records the same intermediates, under the same
//! names, that `tools/trace-amrnb-reference.sh` dumps from the instrumented
//! reference. That harness is the first thing to reach for when the output is
//! wrong: on the wideband decoder a full session spent reasoning about output
//! PCM produced one speculative lead and no fixes, while diffing intermediates
//! found every remaining defect in a single pass.

use super::codebook::{sharpen, sharpening_factor, sharpening_state, FixedCodebook};
use super::conceal;
use super::detect::{interpolate_lsf, LsfAverage, SourceDetector};
use super::dtx::{DtxDecoder, DtxState, RxFrameType};
use super::gain::{
    decode_code_gain, decode_joint, decode_pitch_gain, CodeGainConcealer, CodeGainPredictor,
    CodeGainSmoother, FrameQuality, PitchGainConcealer, SubframeGains, WithPrevious,
};
use super::lag::{
    absolute_lag, delta_coding, delta_lag_1_3, delta_lag_1_6, delta_window, is_delta_coded,
    Excitation, LagResolution, PitchLag,
};
use super::lsp::{interpolate_lsp, interpolate_lsp_mid, LsfDecoder, AZ_SIZE, M, MP1};
use super::math::sqrt_l_exp;
use super::postfilter::{PostFilter, PostProcessor};
use super::synthesis::{
    control_excitation, match_energy, synthesis_filter, ExcitationGains, PhaseDispersion,
    EXC_ENERGY_HIST,
};
use super::{L_FRAME, L_SUBFR, PIT_MAX, SHARPMAX};
use crate::error::{CodecError, Result};
use crate::fixed_point::arith::{add, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult};
use crate::fixed_point::shift::{l_shr, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Subframes per frame.
const NB_SUBFR: usize = 4;

/// 10.2 kbit/s, which passes seven codebook parameters rather than two.
const MR102: u8 = 6;

/// 12.2 kbit/s, the only rate with a mid-frame LSP set and separate gains.
const MR122: u8 = 7;

/// 7.95 kbit/s, the other rate that quantises its two gains separately.
const MR795: u8 = 5;

/// LTP gains the source detector reads, newest last.
const LTP_HISTORY: usize = 9;

/// How the frame arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameState {
    /// Bits arrived and the transport believes them.
    Good,
    /// Bits arrived damaged.
    ///
    /// This is also the *tail* of the lost-frame path, but not its entirety:
    /// AMR-NB has no distinct lost frame type, and the reference reaches this
    /// same code after first manufacturing a parameter vector from
    /// `nodataSeed`. A frame that never arrived must therefore enter through
    /// [`Decoder::conceal_lost_frame`], which synthesises what this state then
    /// consumes. Passing stale bits here instead decodes a different
    /// excitation -- the fixed codebook reads the indices even when the frame
    /// is bad.
    Bad,
    /// The transport says the frame is usable but degraded.
    ///
    /// Distinct from [`FrameState::Bad`]: the bits are decoded normally, and
    /// the flag only softens the codebook gain and arms the excitation control.
    Degraded,
}

/// A complete AMR-NB decoder.
///
/// One instance decodes one stream. Nothing here is shared with the wideband
/// decoder, deliberately: the two codecs differ in the spectral representation,
/// the interpolation weights, the adaptive-codebook resolution, the output word
/// width and the presence of a post-filter, and every one of those differences
/// would compile and produce speech-shaped output if confused.
#[derive(Debug, Clone)]
pub struct Decoder {
    lsf: LsfDecoder,
    excitation: Excitation,
    predictor: CodeGainPredictor,
    pitch_concealer: PitchGainConcealer,
    code_concealer: CodeGainConcealer,
    smoother: CodeGainSmoother,
    dispersion: PhaseDispersion,
    detector: SourceDetector,
    lsf_average: LsfAverage,
    post_filter: PostFilter,
    post_processor: PostProcessor,

    /// Previous frame's LSPs, the interpolation's left endpoint.
    lsp_old: [Word16; M],
    /// Synthesis filter memory, the last `M` output samples.
    mem_syn: [Word16; M],
    /// Pitch-sharpening factor carried to the next subframe, Q14.
    sharp: Word16,
    /// Previous subframe's integer lag.
    old_lag: Word16,
    /// The lag as *received*, before any concealment substitution.
    ///
    /// Kept separately because concealment in background noise prefers the
    /// received lag over the extrapolated one, and by then the extrapolation
    /// has already overwritten `old_lag`.
    lag_buffer: Word16,
    /// Consecutive-erasure state machine, 0..=6.
    erasure_state: u8,
    /// Whether the previous frame was bad, and whether it was degraded.
    previous: PreviousFrame,
    /// Excitation energies of the last nine subframes.
    energy_history: [Word16; EXC_ENERGY_HIST],
    /// Pitch gains of the last nine subframes, newest last.
    ltp_gains: [Word16; LTP_HISTORY],
    /// Whether any frame has been decoded since the last reset.
    ///
    /// The reference's `dtxGlobalState` starts at `DTX`, not `SPEECH`
    /// (`dtx_dec_reset`), and only becomes `SPEECH` once a frame has been
    /// received. A frame lost before that takes the comfort-noise path rather
    /// than the concealment one, so concealing it here would silently diverge
    /// -- verified against the reference, which emits a non-zero frame where
    /// this decoder emits silence.
    started: bool,
    /// `dec_amr.c`'s `st->nodataSeed`.
    ///
    /// Advances only on a lost frame -- a good or damaged one leaves it
    /// untouched -- and is re-seeded by a full reset *and* by every
    /// comfort-noise frame, because `Decoder_amr_reset(st, MRDTX)` sets it.
    nodata_seed: Word16,
    /// The comfort-noise state, `st->dtxDecoderState`.
    dtx: DtxDecoder,

    /// Per-frame scalar trace, for diffing against the instrumented reference.
    #[cfg(test)]
    pub(crate) trace: Vec<(&'static str, i64)>,
    /// Per-frame vector trace, same purpose.
    #[cfg(test)]
    pub(crate) vtrace: Vec<(&'static str, Vec<i16>)>,
}

/// The two previous-frame flags, which are read together everywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PreviousFrame {
    bad: bool,
    degraded: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder in its reset state, as `Decoder_amr_reset` leaves it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lsf: LsfDecoder::at_reset(),
            excitation: Excitation::new(),
            predictor: CodeGainPredictor::new(),
            pitch_concealer: PitchGainConcealer::new(),
            code_concealer: CodeGainConcealer::new(),
            smoother: CodeGainSmoother::new(),
            dispersion: PhaseDispersion::new(),
            detector: SourceDetector::new(),
            lsf_average: LsfAverage::new(),
            post_filter: PostFilter::new(),
            post_processor: PostProcessor::new(),
            lsp_old: super::lsp::initial_lsp(),
            mem_syn: [Word16(0); M],
            sharp: Word16(0),
            // 40, not 0: a zero lag is not a legal pitch period, and the first
            // delta-coded subframe of the stream reads this.
            old_lag: Word16(40),
            lag_buffer: Word16(40),
            erasure_state: 0,
            previous: PreviousFrame::default(),
            energy_history: [Word16(0); EXC_ENERGY_HIST],
            ltp_gains: [Word16(0); LTP_HISTORY],
            started: false,
            nodata_seed: conceal::NODATA_SEED_INIT,
            dtx: DtxDecoder::new(),
            #[cfg(test)]
            trace: Vec::new(),
            #[cfg(test)]
            vtrace: Vec::new(),
        }
    }

    /// Decode one speech frame's payload into 160 samples at 8 kHz.
    ///
    /// `payload` is the frame's coded bits, left-aligned and zero-padded to a
    /// whole number of octets — the RFC 4867 frame body without its table of
    /// contents. Returns `None` when the payload is too short for the rate.
    ///
    /// The output is 13-bit linear: the low three bits of every sample are
    /// zero, because that is how TS 26.073 defines AMR-NB's output. Comparing
    /// unmasked output against reference PCM scores near zero for a perfectly
    /// correct decoder, which cost the wideband port a full session at its own
    /// 14-bit equivalent.
    #[must_use]
    pub fn decode(&mut self, mode_index: u8, payload: &[u8]) -> Option<[i16; L_FRAME]> {
        let params = crate::codecs::amr::nb::bitstream::parse(mode_index, payload)?;
        Some(self.decode_parameters(mode_index, &params, FrameState::Good))
    }

    /// Decode from already-unpacked parameters.
    ///
    /// Exposed because the frame state cannot be expressed in the payload: a
    /// damaged frame still has bits, and the RFC 4867 Q bit is what says not to
    /// trust them.
    ///
    /// # Panics
    ///
    /// If `params` is shorter than the rate's parameter count.
    #[allow(clippy::too_many_lines)]
    pub fn decode_parameters(
        &mut self,
        mode_index: u8,
        params: &[u16],
        state: FrameState,
    ) -> [i16; L_FRAME] {
        let mut ctx = DspContext::default();
        #[cfg(test)]
        {
            self.trace.clear();
            self.vtrace.clear();
        }

        let bad = state == FrameState::Bad;
        let degraded = state == FrameState::Degraded;

        self.advance_erasure_state(bad);

        // The first speech frame out of a silence starts from state 5 rather
        // than from wherever the erasure counter had got to, and whether it is
        // muted depends on how the silence ended. Coming out of `DTX_MUTE` the
        // frame is muted; out of plain `DTX` it is not. State 5 either way, so
        // that a SID misread as speech is attenuated quickly.
        //
        // This runs *before* the frame quality below is taken, because
        // `prev_bf` is what it writes and the concealers are what read it.
        // Taking the snapshot first leaves them looking at the value from
        // before the silence, and the first frame out of `DTX_MUTE` comes back
        // at full level instead of muted -- 160 samples wrong on that frame
        // and a tail of a dozen more while the gain smoother recovers.
        match self.dtx.global_state() {
            DtxState::Dtx => {
                self.erasure_state = 5;
                self.previous.bad = false;
            }
            DtxState::DtxMute => {
                self.erasure_state = 5;
                self.previous.bad = true;
            }
            DtxState::Speech => {}
        }

        let quality = FrameQuality {
            bad: WithPrevious {
                current: bad,
                previous: self.previous.bad,
            },
            degraded: WithPrevious {
                current: degraded,
                previous: self.previous.degraded,
            },
            background_noise: self.detector.background_noise(),
            voiced_hangover: self.detector.voiced_hangover().0,
        };

        // The gain smoother compares this frame's interpolated LSFs against the
        // *previous* frame's, so they have to be taken before the spectral path
        // overwrites them.
        let previous_lsf = *self.lsf.last_lsf();

        let az = self.decode_spectrum(&mut ctx, mode_index, params, bad);
        self.vtrace("A_t", &az);

        let mut synthesis = [Word16(0); L_FRAME];
        let mut cursor = if mode_index == MR122 { 5 } else { 3 };
        // 4.75 kbit/s transmits one gain index per *pair* of subframes, on the
        // even one. The odd subframe re-decodes the same index with the other
        // half of the table entry, so the value has to survive the two
        // parameters the lag and codebook consume in between.
        let mut shared_gain_index = 0u16;

        for subframe in 0..NB_SUBFR {
            let even = subframe % 2 == 0;
            let lag = self.decode_lag(&mut ctx, mode_index, params[cursor], subframe, bad);
            cursor += 1;
            self.trace1("T0", i64::from(lag.integer.0));
            self.trace1("T0_frac", i64::from(lag.frac.0));

            self.excitation.predict(&mut ctx, lag);
            {
                let adapt = self.excitation.subframe().to_vec();
                self.vtrace("adapt", &adapt);
            }

            // 12.2 sends its pitch gain before the codevector, every other rate
            // after it. The order is not cosmetic: 12.2 sharpens the innovation
            // with the gain it just read, where the others use the previous
            // subframe's.
            let mut gain_pitch = Word16(0);
            if mode_index == MR122 {
                gain_pitch = self.decode_pitch_gain_12k2(&mut ctx, params[cursor], bad, quality);
                cursor += 1;
            }

            let mut code =
                Self::decode_codebook(&mut ctx, mode_index, params, &mut cursor, subframe);
            self.vtrace("code_raw", &code);

            let sharpening = if mode_index == MR122 {
                shl(&mut ctx, gain_pitch, 1)
            } else {
                sharpening_factor(&mut ctx, self.sharp)
            };
            self.trace1("pit_sharp_pre", i64::from(sharpening.0));
            sharpen(&mut ctx, &mut code, lag.integer.0, sharpening);

            let (gains, mut sharpening) = self.decode_gains(
                &mut ctx,
                mode_index,
                params,
                &mut cursor,
                &code,
                even,
                gain_pitch,
                quality,
                &mut shared_gain_index,
            );
            let SubframeGains {
                pitch: mut gain_pitch,
                code: gain_code,
            } = gains;

            // 10.2 attenuates its sharpening when the *previous* lag was long,
            // and reads that lag after the bad-frame increment above.
            if mode_index == MR102 && sub(&mut ctx, self.old_lag, Word16(45)).0 > 0 {
                sharpening = shr(&mut ctx, sharpening, 2);
            }

            // The stored factor does not advance on 4.75's even subframes,
            // because that subframe's gain is provisional until the odd one
            // chooses the pair.
            if mode_index != 0 || !even {
                self.sharp = sharpening_state(gain_pitch);
            }

            let sharpening = shl(&mut ctx, sharpening, 1);
            let sharpened = (sub(&mut ctx, sharpening, Word16(16384)).0 > 0)
                .then(|| self.pitch_sharpened_copy(&mut ctx, mode_index, sharpening, gain_pitch));

            if !bad {
                self.ltp_gains.copy_within(1.., 0);
                self.ltp_gains[LTP_HISTORY - 1] = gain_pitch;
            }

            gain_pitch = self.limit_gain_in_noise(&mut ctx, mode_index, gain_pitch, bad);

            let subframe_lsf = interpolate_lsf(
                &mut ctx,
                &previous_lsf,
                self.lsf.last_lsf(),
                subframe * L_SUBFR,
            );
            let mixed = self.smoother.smooth(
                &mut ctx,
                mode_index,
                gain_code,
                &subframe_lsf,
                self.lsf_average.mean(),
                quality,
            );
            // 7.40, 7.95 and 12.2 use the unsmoothed gain. The smoother still
            // ran: its history and both counters advance for every rate.
            let mixed = if mode_index > 3 && mode_index != MR102 {
                gain_code
            } else {
                mixed
            };

            self.trace1("gain_pit", i64::from(gain_pitch.0));
            self.trace1("gain_code", i64::from(gain_code.0));
            self.trace1("gain_code_mix", i64::from(mixed.0));
            self.trace1("pit_sharp", i64::from(sharpening.0));
            self.vtrace("code", &code);

            let (pitch_factor, shift) = if mode_index == MR122 {
                (shr(&mut ctx, gain_pitch, 1), 2)
            } else {
                (gain_pitch, 1)
            };
            self.trace1("pitch_fac", i64::from(pitch_factor.0));
            self.trace1("tmp_shift", i64::from(shift));

            // The two excitations part company here. `enhanced` starts as an
            // unscaled copy of the adaptive-codebook vector; the history in
            // `self.excitation` becomes the scaled total. Everything after this
            // point has to be given the right one.
            let mut enhanced = [Word16(0); L_SUBFR];
            enhanced.copy_from_slice(self.excitation.subframe());
            self.vtrace("ltp_unscaled", &enhanced);

            self.form_total_excitation(&mut ctx, pitch_factor, gain_code, &code, shift);
            {
                let total = self.excitation.subframe().to_vec();
                self.vtrace("exc_total", &total);
            }

            self.dispersion.release();
            if self.locks_dispersion(mode_index, bad) {
                self.dispersion.lock();
            }
            self.dispersion.apply(
                &mut ctx,
                mode_index,
                &mut enhanced,
                &mut code.clone(),
                ExcitationGains {
                    codebook: mixed,
                    pitch: gain_pitch,
                    pitch_factor,
                    shift,
                },
            );
            self.vtrace("exc_enhanced", &enhanced);

            let energy = Self::excitation_energy(&mut ctx, &enhanced);
            self.trace1("excEnergy", i64::from(energy.0));
            self.run_excitation_control(&mut ctx, mode_index, &mut enhanced, energy, quality);
            self.update_energy_history(energy, quality);

            let target = &mut synthesis[subframe * L_SUBFR..(subframe + 1) * L_SUBFR];
            let a = &az[subframe * MP1..(subframe + 1) * MP1];
            self.synthesise(&mut ctx, a, &mut enhanced, sharpened, target);
            self.vtrace("syn", target);

            self.excitation.advance();
            self.old_lag = lag.integer;
        }

        self.detector.update(&mut ctx, &self.ltp_gains, &synthesis);

        // Before the post-filter: the noise description is derived from the
        // synthesis, not from what a listener eventually hears.
        let last_lsf = *self.lsf.last_lsf();
        self.dtx.observe_speech(&mut ctx, &last_lsf, &synthesis);

        self.previous = PreviousFrame { bad, degraded };
        self.started = true;
        self.lsf_average.update(&mut ctx, self.lsf.last_lsf());

        // `the_end: st->dtxDecoderState->dtxGlobalState = newDTXState`, which
        // the reference runs on every frame including this path's. Without it
        // the override above fires on every frame of a speech-only stream
        // rather than only on the first, because `dtx_dec_reset` leaves the
        // state at DTX and nothing else would ever move it.
        self.dtx.commit(DtxState::Speech);

        self.post_filter
            .process(&mut ctx, mode_index, &mut synthesis, &az);
        self.vtrace("postfilter", &synthesis);
        self.post_processor.process(&mut ctx, &mut synthesis);
        self.vtrace("postproc", &synthesis);

        // 13-bit linear output. A mask, not a shift pair and not a rounding
        // step — the reference writes `synth[i] & 0xfff8`.
        let mut out = [0i16; L_FRAME];
        for (slot, sample) in out.iter_mut().zip(synthesis.iter()) {
            *slot = sample.0 & !7;
        }
        out
    }

    /// Decode one frame of a stream that may use DTX — `Speech_Decode_Frame`.
    ///
    /// This is the whole receiver: it decides between speech and comfort noise,
    /// and every path through it leaves the state the next frame needs.
    /// [`decode`](Self::decode) and [`conceal_lost_frame`](Self::conceal_lost_frame)
    /// remain for streams where the caller has already made that decision.
    ///
    /// `params` is the frame's unpacked parameters. For a `SID_UPDATE` or
    /// `SID_BAD` those are the five *MRDTX* parameters, not the speech mode's —
    /// the reference overrides the bit allocation for exactly these two types
    /// in `Speech_Decode_Frame`, and unpacking a SID with the speech mode's
    /// widths yields five numbers that decode into a spectrum. For `SID_FIRST`,
    /// `NO_DATA` and `ONSET` the slice is unread and may be empty.
    ///
    /// `mode_index` is always a *speech* mode: on a SID it is the mode
    /// indication carried in the SID's own payload, which tells the decoder
    /// which level adjustment the encoder had been using.
    ///
    /// # Panics
    /// If `mode_index` is not 0..=7.
    pub fn decode_typed(
        &mut self,
        frame_type: RxFrameType,
        mode_index: u8,
        params: &[u16],
    ) -> [i16; L_FRAME] {
        assert!(mode_index < 8, "mode {mode_index} is not a speech mode");
        let mut ctx = DspContext::default();

        let new_state = self.dtx.receive(&mut ctx, frame_type);
        let out = if new_state == DtxState::Speech {
            let (state, params) =
                self.speech_frame_inputs(&mut ctx, frame_type, mode_index, params);
            self.decode_parameters(mode_index, &params, state)
        } else {
            self.comfort_noise_frame(&mut ctx, new_state, mode_index, params)
        };

        self.dtx.commit(new_state);
        out
    }

    /// What the speech path should be told about a frame, and what to decode.
    ///
    /// Three of the eight frame types reach the speech decoder without usable
    /// bits, and the reference does not treat them alike: `SPEECH_BAD` keeps
    /// the parameters it received and marks them untrusted, while `NO_DATA` and
    /// `ONSET` have none and manufacture a vector from `nodataSeed` first. The
    /// concealers ignore what that produces, but the fixed codebook does not.
    fn speech_frame_inputs(
        &mut self,
        ctx: &mut DspContext,
        frame_type: RxFrameType,
        mode_index: u8,
        params: &[u16],
    ) -> (FrameState, Vec<u16>) {
        match frame_type {
            RxFrameType::SpeechGood => (FrameState::Good, params.to_vec()),
            RxFrameType::SpeechDegraded => (FrameState::Degraded, params.to_vec()),
            RxFrameType::SpeechBad => (FrameState::Bad, params.to_vec()),
            RxFrameType::NoData | RxFrameType::Onset => (
                FrameState::Bad,
                conceal::build_cn_param(ctx, &mut self.nodata_seed, mode_index),
            ),
            // The SID types cannot reach the speech path: `receive` returns
            // DTX for all three unconditionally.
            other => unreachable!("{other:?} cannot be synthesised as speech"),
        }
    }

    /// Synthesise, post-filter and post-process one comfort-noise frame.
    fn comfort_noise_frame(
        &mut self,
        ctx: &mut DspContext,
        new_state: DtxState,
        mode_index: u8,
        params: &[u16],
    ) -> [i16; L_FRAME] {
        self.reset_for_comfort_noise();

        let cn = self.dtx.comfort_noise(
            ctx,
            new_state,
            usize::from(mode_index),
            params,
            &mut self.mem_syn,
            &mut self.lsf,
            &mut self.predictor,
        );

        // The rest of the decoder is told the spectrum was the comfort noise's,
        // so the first speech frame afterwards interpolates from it rather than
        // from whatever preceded the silence.
        self.lsp_old = super::lsp::lsf_to_lsp(ctx, self.lsf.last_lsf());
        self.lsf_average.update(ctx, self.lsf.last_lsf());

        let mut synthesis = cn.synth;
        self.post_filter
            .process(ctx, mode_index, &mut synthesis, &cn.a_t);
        self.post_processor.process(ctx, &mut synthesis);

        let mut out = [0i16; L_FRAME];
        for (slot, sample) in out.iter_mut().zip(synthesis.iter()) {
            *slot = sample.0 & !7;
        }
        out
    }

    /// `Decoder_amr_reset(st, MRDTX)`: clear the speech state, keep the rest.
    ///
    /// The five exemptions are what make comfort noise continuous with the
    /// speech around it, and each is load-bearing. `mem_syn` carries the
    /// synthesis filter across the boundary, so the noise does not start from
    /// an impulse. `lsp_old` and the LSP average keep the spectral history the
    /// backward analysis has just been computed from. The excitation energy
    /// history is what a later erasure's gain control reads. And the gain
    /// predictor is re-seeded from the SID's own level a moment later, which
    /// clearing it here would then undo.
    ///
    /// Everything else goes, `nodataSeed` included — so a lost frame after a
    /// silence draws from a re-seeded generator, not from wherever the stream
    /// had got to.
    fn reset_for_comfort_noise(&mut self) {
        self.excitation = Excitation::new();
        // `SHARPMIN` -- zero, not the mid-scale value the encoder starts from.
        self.sharp = Word16(0);
        self.old_lag = Word16(40);
        self.lag_buffer = Word16(40);
        self.erasure_state = 0;
        self.previous = PreviousFrame::default();
        self.ltp_gains = [Word16(0); LTP_HISTORY];
        self.smoother.reset_for_comfort_noise();
        self.lsf = LsfDecoder::at_reset();
        self.pitch_concealer = PitchGainConcealer::new();
        self.code_concealer = CodeGainConcealer::new();
        self.detector = SourceDetector::new();
        self.nodata_seed = conceal::NODATA_SEED_INIT;
        self.dispersion = PhaseDispersion::new();
    }

    /// Conceal a frame that never arrived -- RFC 4867 frame type 15, the
    /// reference's `RX_NO_DATA`.
    ///
    /// Not the same as decoding a damaged frame, though it ends in the same
    /// place. `dec_amr.c` manufactures the entire parameter vector from
    /// `nodataSeed` first, and only then runs the bad-frame path; the LSF and
    /// gain concealers ignore what it produced, but the fixed codebook does
    /// not, so the synthesised pulses become the excitation. Feeding a lost
    /// frame through [`Self::decode_parameters`] with [`FrameState::Bad`] and
    /// whatever bits happened to be in the buffer produces plausible speech
    /// that matches no reference.
    ///
    /// `mode_index` is the mode the stream was last using: a lost frame
    /// carries none of its own, and the reference substitutes `prev_mode`. It
    /// selects both how many parameters are synthesised and how wide each one
    /// is.
    ///
    /// # Two limits, both from DTX
    ///
    /// `dtx_dec_reset` leaves `dtxGlobalState` at `DTX`, not `SPEECH`, so the
    /// reference sends a frame lost *before any frame has been received* to
    /// comfort noise rather than here. Measured: it emits a non-zero frame
    /// where this path emits silence. That case is refused rather than
    /// approximated.
    ///
    /// `Decoder_amr_reset(st, MRDTX)` also re-seeds `nodataSeed` on every
    /// comfort-noise frame, so on a stream that has passed through DTX the
    /// sequence depends on that history. This is therefore exact for a stream
    /// that has never entered DTX -- and when DTX lands, re-seeding from the
    /// comfort-noise path is required behaviour, not an optimisation.
    ///
    /// # Errors
    /// When no frame has been decoded since the last reset, per above.
    ///
    /// # Panics
    /// If `mode_index` is not a speech mode, 0..=7.
    pub fn conceal_lost_frame(&mut self, mode_index: u8) -> Result<[i16; L_FRAME]> {
        if !self.started {
            return Err(CodecError::feature_not_enabled(
                "AMR-NB cannot conceal the first frame of a stream: the reference starts in \
                 the DTX state and routes that frame to comfort noise rather than to \
                 concealment, and concealment here extrapolates from decoder history that \
                 does not exist yet. Decode at least one frame first.",
            ));
        }
        let mut ctx = DspContext::default();
        let params = conceal::build_cn_param(&mut ctx, &mut self.nodata_seed, mode_index);
        Ok(self.decode_parameters(mode_index, &params, FrameState::Bad))
    }

    /// Return the decoder to its reset state, keeping nothing.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    // ----------------------------------------------------------------- parts --

    /// The consecutive-erasure counter, `dec_amr.c`'s `st->state`.
    ///
    /// It climbs on every bad frame and drops to zero on a good one — except
    /// from six, where it drops to five instead, so that a single good frame in
    /// a long erasure does not undo the whole attenuation.
    fn advance_erasure_state(&mut self, bad: bool) {
        self.erasure_state = if bad {
            (self.erasure_state + 1).min(6)
        } else if self.erasure_state == 6 {
            5
        } else {
            0
        };
    }

    /// LSF dequantisation and interpolation to four subframe filters.
    fn decode_spectrum(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        params: &[u16],
        bad: bool,
    ) -> [Word16; AZ_SIZE] {
        if mode_index == MR122 {
            let (mid, new) = self.lsf.decode_pair(&params[..5], bad);
            let az = interpolate_lsp_mid(ctx, &self.lsp_old, &mid, &new);
            self.lsp_old = new;
            az
        } else {
            let new = self.lsf.decode(mode_index, &params[..3], bad);
            let az = interpolate_lsp(ctx, &self.lsp_old, &new);
            self.lsp_old = new;
            az
        }
    }

    /// One subframe's pitch lag, including the bad-frame substitution.
    fn decode_lag(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        index: u16,
        subframe: usize,
        bad: bool,
    ) -> PitchLag {
        let resolution = LagResolution::for_mode(mode_index);
        let delta = is_delta_coded(mode_index, subframe);
        let index = Word16(i16::try_from(index).unwrap_or(i16::MAX));

        if mode_index == MR122 {
            let lag = if delta {
                delta_lag_1_6(ctx, index, self.old_lag)
            } else {
                absolute_lag(ctx, index, resolution)
            };
            // 12.2 has no graceful degradation: on a bad frame, or on one of
            // the three unused delta codewords, it repeats the previous lag
            // outright.
            if bad || (delta && index.0 >= 61) {
                self.lag_buffer = lag.integer;
                return PitchLag::integral(self.old_lag, resolution);
            }
            return lag;
        }

        // The window and the decoders read the lag *before* the bad-frame
        // increment below.
        let window = delta_window(ctx, mode_index, self.old_lag);
        let lag = if delta {
            delta_lag_1_3(ctx, index, window, delta_coding(mode_index, self.old_lag))
        } else {
            absolute_lag(ctx, index, resolution)
        };
        self.lag_buffer = lag.integer;

        if !bad {
            return lag;
        }

        // Graceful degradation: lengthen the repeated period by one sample per
        // erased frame, so a long erasure drifts flat rather than holding one
        // pitch. In background noise the received lag is preferred instead —
        // there is no pitch to hold, and the extrapolation would invent one.
        if self.old_lag.0 < PIT_MAX {
            self.old_lag = add(ctx, self.old_lag, Word16(1));
        }
        let noisy = self.detector.background_noise()
            && self.detector.voiced_hangover().0 > 4
            && mode_index <= 2;
        PitchLag::integral(
            if noisy { self.lag_buffer } else { self.old_lag },
            resolution,
        )
    }

    /// 12.2's pitch gain, which is read before the codevector.
    fn decode_pitch_gain_12k2(
        &mut self,
        ctx: &mut DspContext,
        index: u16,
        bad: bool,
        quality: FrameQuality,
    ) -> Word16 {
        let gain = if bad {
            self.pitch_concealer.conceal(ctx, self.erasure_state)
        } else {
            decode_pitch_gain(ctx, MR122, index)
        };
        self.pitch_concealer.update(ctx, quality.bad, gain)
    }

    /// The rate's algebraic codevector, advancing `cursor` past its parameters.
    fn decode_codebook(
        ctx: &mut DspContext,
        mode_index: u8,
        params: &[u16],
        cursor: &mut usize,
        subframe: usize,
    ) -> super::codebook::Codevector {
        let book = match mode_index {
            0 | 1 => {
                let positions = params[*cursor];
                let signs = params[*cursor + 1];
                *cursor += 2;
                FixedCodebook::TwoPulses9Bit {
                    subframe: u8::try_from(subframe).expect("0..=3"),
                    signs,
                    positions,
                }
            }
            2 => {
                let positions = params[*cursor];
                let signs = params[*cursor + 1];
                *cursor += 2;
                FixedCodebook::TwoPulses11Bit { signs, positions }
            }
            3 => {
                let positions = params[*cursor];
                let signs = params[*cursor + 1];
                *cursor += 2;
                FixedCodebook::ThreePulses14Bit { signs, positions }
            }
            4 | 5 => {
                let positions = params[*cursor];
                let signs = params[*cursor + 1];
                *cursor += 2;
                FixedCodebook::FourPulses17Bit { signs, positions }
            }
            MR102 => {
                let mut fields = [0u16; 7];
                fields.copy_from_slice(&params[*cursor..*cursor + 7]);
                *cursor += 7;
                FixedCodebook::EightPulses31Bit(fields)
            }
            _ => {
                let mut fields = [0u16; 10];
                fields.copy_from_slice(&params[*cursor..*cursor + 10]);
                *cursor += 10;
                FixedCodebook::TenPulses35Bit(fields)
            }
        };
        book.decode(ctx)
    }

    /// The subframe's gains, and the sharpening factor derived from the pitch
    /// gain, advancing `cursor` past the gain parameters.
    #[allow(clippy::too_many_arguments)]
    fn decode_gains(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        params: &[u16],
        cursor: &mut usize,
        code: &[Word16; L_SUBFR],
        even: bool,
        gain_pitch_12k2: Word16,
        quality: FrameQuality,
        shared_gain_index: &mut u16,
    ) -> (SubframeGains, Word16) {
        let bad = quality.bad.current;

        let gains = match mode_index {
            // 4.75 shares one index across two subframes: the even subframe
            // reads it, the odd one re-uses the same value. The odd subframe
            // still runs the whole decode, because the predictor advances.
            0 => {
                if even {
                    *shared_gain_index = params[*cursor];
                    *cursor += 1;
                }
                let index = *shared_gain_index;
                if bad {
                    self.conceal_gains(ctx, quality)
                } else {
                    let g = decode_joint(ctx, &mut self.predictor, 0, index, code, even);
                    self.commit_gains(ctx, quality, g)
                }
            }
            1..=4 | MR102 => {
                let index = params[*cursor];
                *cursor += 1;
                if bad {
                    self.conceal_gains(ctx, quality)
                } else {
                    let g = decode_joint(ctx, &mut self.predictor, mode_index, index, code, even);
                    self.commit_gains(ctx, quality, g)
                }
            }
            MR795 => {
                let pitch_index = params[*cursor];
                let code_index = params[*cursor + 1];
                *cursor += 2;
                let pitch = if bad {
                    self.pitch_concealer.conceal(ctx, self.erasure_state)
                } else {
                    decode_pitch_gain(ctx, MR795, pitch_index)
                };
                let pitch = self.pitch_concealer.update(ctx, quality.bad, pitch);
                let code_gain = if bad {
                    self.code_concealer
                        .conceal(ctx, &mut self.predictor, self.erasure_state)
                } else {
                    decode_code_gain(ctx, &mut self.predictor, MR795, code_index, code)
                };
                let code_gain = self.code_concealer.update(ctx, quality.bad, code_gain);
                SubframeGains {
                    pitch,
                    code: code_gain,
                }
            }
            _ => {
                // 12.2: the pitch gain was read before the codevector.
                let index = params[*cursor];
                *cursor += 1;
                let code_gain = if bad {
                    self.code_concealer
                        .conceal(ctx, &mut self.predictor, self.erasure_state)
                } else {
                    decode_code_gain(ctx, &mut self.predictor, MR122, index, code)
                };
                let code_gain = self.code_concealer.update(ctx, quality.bad, code_gain);
                SubframeGains {
                    pitch: gain_pitch_12k2,
                    code: code_gain,
                }
            }
        };

        // 12.2 sharpens with the pitch gain it read earlier and does not clamp
        // here; every other rate clamps to SHARPMAX.
        let sharpening = if mode_index == MR122 {
            gains.pitch
        } else if sub(ctx, gains.pitch, Word16(SHARPMAX)).0 > 0 {
            Word16(SHARPMAX)
        } else {
            gains.pitch
        };

        (gains, sharpening)
    }

    /// Both gains from their concealment generators.
    fn conceal_gains(&mut self, ctx: &mut DspContext, quality: FrameQuality) -> SubframeGains {
        let pitch = self.pitch_concealer.conceal(ctx, self.erasure_state);
        let code = self
            .code_concealer
            .conceal(ctx, &mut self.predictor, self.erasure_state);
        SubframeGains {
            pitch: self.pitch_concealer.update(ctx, quality.bad, pitch),
            code: self.code_concealer.update(ctx, quality.bad, code),
        }
    }

    /// Record decoded gains in both concealers, which may limit them.
    fn commit_gains(
        &mut self,
        ctx: &mut DspContext,
        quality: FrameQuality,
        gains: SubframeGains,
    ) -> SubframeGains {
        SubframeGains {
            pitch: self.pitch_concealer.update(ctx, quality.bad, gains.pitch),
            code: self.code_concealer.update(ctx, quality.bad, gains.code),
        }
    }

    /// A copy of the adaptive-codebook vector scaled by the sharpening factor.
    ///
    /// Only built when the factor exceeds 1.0 in Q15, which is the reference's
    /// own gate. It is later blended into the enhanced excitation and matched
    /// back to its energy.
    fn pitch_sharpened_copy(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        sharpening: Word16,
        gain_pitch: Word16,
    ) -> [Word16; L_SUBFR] {
        let mut out = [Word16(0); L_SUBFR];
        for (slot, &sample) in out.iter_mut().zip(self.excitation.subframe()) {
            let scaled = mult(ctx, sample, sharpening);
            let mut acc = l_mult(ctx, scaled, gain_pitch);
            if mode_index == MR122 {
                acc = l_shr(ctx, acc, 1);
            }
            *slot = round(ctx, acc);
        }
        out
    }

    /// Cap the pitch gain during an erasure in background noise.
    ///
    /// Only the three lowest rates, and only when the previous or current frame
    /// was bad: a repeated high pitch gain in noise turns the concealment into
    /// a tone.
    fn limit_gain_in_noise(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        gain_pitch: Word16,
        bad: bool,
    ) -> Word16 {
        if !((self.previous.bad || bad) && self.detector.background_noise() && mode_index <= 2) {
            return gain_pitch;
        }
        let mut gain = gain_pitch;
        if sub(ctx, gain, Word16(12288)).0 > 0 {
            let excess = sub(ctx, gain, Word16(12288));
            let halved = shr(ctx, excess, 1);
            gain = add(ctx, halved, Word16(12288));
        }
        if sub(ctx, gain, Word16(14745)).0 > 0 {
            gain = Word16(14745);
        }
        gain
    }

    /// Whether phase dispersion is forced to maximum this subframe.
    const fn locks_dispersion(&self, mode_index: u8, bad: bool) -> bool {
        mode_index <= 2
            && self.detector.voiced_hangover().0 > 3
            && self.detector.background_noise()
            && bad
    }

    /// Overwrite the excitation history's current subframe with the total.
    ///
    /// This is what the *next* subframe's adaptive codebook reads, and it is
    /// deliberately not what the synthesis filter consumes.
    fn form_total_excitation(
        &mut self,
        ctx: &mut DspContext,
        pitch_factor: Word16,
        gain_code: Word16,
        code: &[Word16; L_SUBFR],
        shift: i16,
    ) {
        let mut totals = [Word16(0); L_SUBFR];
        for (i, slot) in totals.iter_mut().enumerate() {
            let mut acc = l_mult(ctx, self.excitation.subframe()[i], pitch_factor);
            acc = l_mac(ctx, acc, code[i], gain_code);
            acc = crate::fixed_point::shift::l_shl(ctx, acc, shift);
            *slot = round(ctx, acc);
        }
        self.excitation.subframe_mut().copy_from_slice(&totals);
    }

    /// The excitation's RMS, scaled the way `Ex_ctrl` expects it.
    fn excitation_energy(ctx: &mut DspContext, excitation: &[Word16; L_SUBFR]) -> Word16 {
        let mut acc = Word32(0);
        for &sample in excitation {
            acc = l_mac(ctx, acc, sample, sample);
        }
        let acc = l_shr(ctx, acc, 1);
        let (root, exp) = sqrt_l_exp(ctx, acc);
        let denormalised = l_shr(ctx, root, (exp >> 1) + 15);
        extract_l(l_shr(ctx, denormalised, 2))
    }

    /// Run the excitation-control module, if this frame qualifies.
    fn run_excitation_control(
        &self,
        ctx: &mut DspContext,
        mode_index: u8,
        excitation: &mut [Word16; L_SUBFR],
        energy: Word16,
        quality: FrameQuality,
    ) {
        let armed = mode_index <= 2
            && self.detector.voiced_hangover().0 > 5
            && self.detector.background_noise()
            && self.erasure_state < 4
            && (quality.degraded.both() || quality.bad.either());
        if !armed {
            return;
        }
        // "Careful" means the frame is only degraded, not lost: the bits are
        // usable, so the rescale is capped rather than allowed its full range.
        let careful = quality.degraded.current && !quality.bad.current;
        control_excitation(
            ctx,
            excitation,
            energy,
            &self.energy_history,
            self.detector.voiced_hangover(),
            self.previous.bad,
            careful,
        );
    }

    /// Push this subframe's energy, unless the frame is being concealed in
    /// noise — in which case the history must not learn from it.
    fn update_energy_history(&mut self, energy: Word16, quality: FrameQuality) {
        let frozen =
            self.detector.background_noise() && quality.bad.either() && self.erasure_state < 4;
        if frozen {
            return;
        }
        self.energy_history.copy_within(1.., 0);
        self.energy_history[EXC_ENERGY_HIST - 1] = energy;
    }

    /// Synthesis filtering, with the reference's overflow-retry protocol.
    ///
    /// The flag is cleared immediately before the filter so the test sees only
    /// this call's overflow. On overflow the *whole* 194-sample excitation
    /// history is divided by four and the subframe is filtered again — the
    /// history and the present must share a scale, since the adaptive codebook
    /// reads across the boundary.
    fn synthesise(
        &mut self,
        ctx: &mut DspContext,
        a: &[Word16],
        excitation: &mut [Word16; L_SUBFR],
        sharpened: Option<[Word16; L_SUBFR]>,
        out: &mut [Word16],
    ) {
        // The sharpened copy, when there is one, is summed with the enhanced
        // excitation and then matched back to its energy — so the blend is what
        // the filter sees, while the history keeps the total excitation.
        let blended = sharpened.map(|mut copy| {
            for (slot, &enhanced) in copy.iter_mut().zip(excitation.iter()) {
                *slot = add(ctx, *slot, enhanced);
            }
            match_energy(ctx, excitation, &mut copy);
            copy
        });
        let input: &[Word16] = blended.as_ref().map_or(&excitation[..], |b| &b[..]);

        ctx.reset_flags();
        let memory = synthesis_filter(ctx, a, input, out, &self.mem_syn);

        if ctx.overflow {
            for sample in self.excitation.all_mut() {
                *sample = shr(ctx, *sample, 2);
            }
            for sample in excitation.iter_mut() {
                *sample = shr(ctx, *sample, 2);
            }
            // The retry updates the memory; the reference passes `update = 1`
            // here where the first attempt passed 0 and copied by hand.
            self.mem_syn = synthesis_filter(ctx, a, excitation, out, &self.mem_syn);
        } else {
            self.mem_syn = memory;
        }
    }

    #[cfg(test)]
    fn trace1(&mut self, name: &'static str, value: i64) {
        self.trace.push((name, value));
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)]
    const fn trace1(&self, _name: &'static str, _value: i64) {}

    #[cfg(test)]
    #[allow(clippy::needless_pass_by_ref_mut)]
    fn vtrace(&mut self, name: &'static str, values: &[Word16]) {
        self.vtrace
            .push((name, values.iter().map(|w| w.0).collect()));
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)]
    const fn vtrace(&self, _name: &'static str, _values: &[Word16]) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::{storage, AmrFrameType};

    /// The `.amr` fixture and the reference PCM for one rate.
    ///
    /// The bitstreams come from `opencore-amr`, which the qualification tests
    /// show reproduces TS 26.073 exactly at every rate; the PCM comes from the
    /// reference decoder itself.
    fn fixture(mode: u8) -> (&'static [u8], &'static [u8]) {
        match mode {
            0 => (
                include_bytes!("../testdata/amrnb_mode0.amr"),
                include_bytes!("../testdata/amrnb_mode0.pcm"),
            ),
            1 => (
                include_bytes!("../testdata/amrnb_mode1.amr"),
                include_bytes!("../testdata/amrnb_mode1.pcm"),
            ),
            2 => (
                include_bytes!("../testdata/amrnb_mode2.amr"),
                include_bytes!("../testdata/amrnb_mode2.pcm"),
            ),
            3 => (
                include_bytes!("../testdata/amrnb_mode3.amr"),
                include_bytes!("../testdata/amrnb_mode3.pcm"),
            ),
            4 => (
                include_bytes!("../testdata/amrnb_mode4.amr"),
                include_bytes!("../testdata/amrnb_mode4.pcm"),
            ),
            5 => (
                include_bytes!("../testdata/amrnb_mode5.amr"),
                include_bytes!("../testdata/amrnb_mode5.pcm"),
            ),
            6 => (
                include_bytes!("../testdata/amrnb_mode6.amr"),
                include_bytes!("../testdata/amrnb_mode6.pcm"),
            ),
            7 => (
                include_bytes!("../testdata/amrnb_mode7.amr"),
                include_bytes!("../testdata/amrnb_mode7.pcm"),
            ),
            other => panic!("no fixture for mode {other}"),
        }
    }

    fn reference(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    /// The DTX fixture and the reference decoder's PCM for one rate.
    ///
    /// Built by `tools/build-amr-dtx-fixtures.sh` from a signal that actually
    /// goes quiet, with the reference encoder driven `-dtx`. The all-speech
    /// fixtures above contain no SID frame at all, so nothing in this file
    /// before this point exercises a single line of the comfort-noise path.
    fn dtx_fixture(mode: u8) -> (&'static [u8], &'static [u8]) {
        match mode {
            0 => (
                include_bytes!("../testdata/amrnb_dtx_mode0.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode0.pcm"),
            ),
            1 => (
                include_bytes!("../testdata/amrnb_dtx_mode1.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode1.pcm"),
            ),
            2 => (
                include_bytes!("../testdata/amrnb_dtx_mode2.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode2.pcm"),
            ),
            3 => (
                include_bytes!("../testdata/amrnb_dtx_mode3.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode3.pcm"),
            ),
            4 => (
                include_bytes!("../testdata/amrnb_dtx_mode4.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode4.pcm"),
            ),
            5 => (
                include_bytes!("../testdata/amrnb_dtx_mode5.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode5.pcm"),
            ),
            6 => (
                include_bytes!("../testdata/amrnb_dtx_mode6.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode6.pcm"),
            ),
            7 => (
                include_bytes!("../testdata/amrnb_dtx_mode7.amr"),
                include_bytes!("../testdata/amrnb_dtx_mode7.pcm"),
            ),
            other => panic!("no DTX fixture for mode {other}"),
        }
    }

    /// What the receiver makes of one stored frame.
    ///
    /// The `.amr` storage format carries the frame type and the quality bit,
    /// which between them determine everything except `SID_FIRST` versus
    /// `SID_UPDATE` -- that distinction lives in the SID payload's own STI bit,
    /// and so does the speech mode the encoder had been using.
    fn classify(frame: &crate::codecs::amr::AmrPayloadFrame, last_mode: u8) -> (RxFrameType, u8) {
        match frame.frame_type {
            AmrFrameType::Speech(mode) => (
                if frame.quality_ok {
                    RxFrameType::SpeechGood
                } else {
                    RxFrameType::SpeechBad
                },
                mode.index(),
            ),
            AmrFrameType::Sid(_) => {
                let header = crate::codecs::amr::nb::bitstream::parse_sid_header(&frame.data)
                    .expect("a SID payload is five octets");
                let kind = if !frame.quality_ok {
                    RxFrameType::SidBad
                } else if header.update {
                    RxFrameType::SidUpdate
                } else {
                    RxFrameType::SidFirst
                };
                (kind, header.mode_index)
            }
            AmrFrameType::NoData => (RxFrameType::NoData, last_mode),
            AmrFrameType::SpeechLost => panic!("narrowband has no SPEECH_LOST"),
        }
    }

    /// Decode a DTX stream and report how many samples matched, and how the
    /// frame types were distributed.
    fn run_dtx(mode: u8) -> (usize, usize, [usize; 3]) {
        let (bits, pcm) = dtx_fixture(mode);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");

        let mut decoder = Decoder::new();
        let (mut exact, mut total) = (0usize, 0usize);
        // speech, SID, NO_DATA
        let mut kinds = [0usize; 3];
        let mut last_mode = mode;

        for (f, frame) in frames.iter().enumerate() {
            let (kind, frame_mode) = classify(frame, last_mode);
            last_mode = frame_mode;
            kinds[match kind {
                RxFrameType::SpeechGood | RxFrameType::SpeechDegraded | RxFrameType::SpeechBad => 0,
                RxFrameType::SidFirst | RxFrameType::SidUpdate | RxFrameType::SidBad => 1,
                RxFrameType::NoData | RxFrameType::Onset => 2,
            }] += 1;

            // A SID's parameters are read with the *MRDTX* layout, not the
            // speech mode's -- `Speech_Decode_Frame` overrides it for exactly
            // SID_UPDATE and SID_BAD.
            let params = match kind {
                RxFrameType::SidUpdate | RxFrameType::SidBad => {
                    crate::codecs::amr::nb::bitstream::parse(8, &frame.data).expect("a SID parses")
                }
                RxFrameType::NoData | RxFrameType::Onset | RxFrameType::SidFirst => Vec::new(),
                _ => crate::codecs::amr::nb::bitstream::parse(frame_mode, &frame.data)
                    .expect("speech parses"),
            };

            let got = decoder.decode_typed(kind, frame_mode, &params);
            for (i, &sample) in got.iter().enumerate() {
                let index = f * L_FRAME + i;
                if index >= want.len() {
                    break;
                }
                total += 1;
                exact += usize::from(sample == want[index]);
            }
        }
        (exact, total, kinds)
    }

    /// The comfort-noise path, sample for sample against the reference
    /// decoder, on all eight rates.
    ///
    /// The fixture's own shape is asserted first, and that is not decoration:
    /// a stream that turned out to be all speech would pass a sample
    /// comparison having exercised nothing this test exists for.
    #[test]
    fn comfort_noise_matches_the_reference_sample_for_sample() {
        for mode in 0..8u8 {
            let (exact, total, kinds) = run_dtx(mode);
            assert!(
                total >= 150 * L_FRAME,
                "mode {mode}: only {total} samples compared"
            );
            assert!(
                kinds[0] >= 40,
                "mode {mode}: only {} speech frames",
                kinds[0]
            );
            assert!(kinds[1] >= 8, "mode {mode}: only {} SID frames", kinds[1]);
            assert!(
                kinds[2] >= 40,
                "mode {mode}: only {} NO_DATA frames",
                kinds[2]
            );
            assert_eq!(
                exact,
                total,
                "mode {mode}: {} of {total} samples differ",
                total - exact
            );
        }
    }

    /// The `DTX_MUTE` fade, which no other fixture reaches.
    ///
    /// `DTX_MUTE` needs a stream that stays quiet for more than fifty frames
    /// *and* loses its SID updates -- a clean encoder sends one every eight, so
    /// the state is unreachable on any well-formed stream. The fixture builder
    /// makes one by dropping SIDs out of mode 4's DTX stream and re-decoding
    /// with the reference, and it fails the build if the result is identical
    /// to the intact original.
    ///
    /// Without this the fade is dead code that passes: removing the 0.75 dB
    /// step entirely leaves every other assertion in this file green.
    #[test]
    fn the_mute_fade_matches_the_reference() {
        let bits: &[u8] = include_bytes!("../testdata/amrnb_dtx_mute.amr");
        let pcm: &[u8] = include_bytes!("../testdata/amrnb_dtx_mute.pcm");
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");

        let mut decoder = Decoder::new();
        let (mut exact, mut total) = (0usize, 0usize);
        let mut muted = 0usize;
        let mut last_mode = 4u8;

        for (f, frame) in frames.iter().enumerate() {
            let (kind, frame_mode) = classify(frame, last_mode);
            last_mode = frame_mode;
            let params = match kind {
                RxFrameType::SidUpdate | RxFrameType::SidBad => {
                    super::super::bitstream::parse(8, &frame.data).expect("a SID parses")
                }
                RxFrameType::NoData | RxFrameType::Onset | RxFrameType::SidFirst => Vec::new(),
                _ => {
                    super::super::bitstream::parse(frame_mode, &frame.data).expect("speech parses")
                }
            };

            let got = decoder.decode_typed(kind, frame_mode, &params);
            muted += usize::from(decoder.dtx.global_state() == DtxState::DtxMute);
            for (i, &sample) in got.iter().enumerate() {
                let index = f * L_FRAME + i;
                if index >= want.len() {
                    break;
                }
                total += 1;
                exact += usize::from(sample == want[index]);
            }
        }

        // The state has to be *entered*, not merely made plausible by a long
        // gap. Asserting the gap length instead is what let the first version
        // of this fixture pass having never muted at all: it dropped the
        // silence's opening SID along with the updates, so the decoder read
        // every empty frame as a lost speech frame and stayed in SPEECH.
        assert!(muted >= 8, "only {muted} frames were synthesised muting");
        assert!(total >= 150 * L_FRAME, "only {total} samples compared");
        assert_eq!(exact, total, "{} of {total} samples differ", total - exact);
    }

    /// Decode a whole fixture and report, per mode, how many samples matched.
    fn run(mode: u8) -> (usize, usize, i32) {
        let (bits, pcm) = fixture(mode);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");

        let mut decoder = Decoder::new();
        let mut exact = 0usize;
        let mut total = 0usize;
        let mut worst = 0i32;

        for (f, frame) in frames.iter().enumerate() {
            let got = decoder.decode(mode, &frame.data).expect("frame decodes");
            for (i, &sample) in got.iter().enumerate() {
                let index = f * L_FRAME + i;
                if index >= want.len() {
                    break;
                }
                total += 1;
                let delta = i32::from(sample) - i32::from(want[index]);
                if delta == 0 {
                    exact += 1;
                }
                worst = worst.max(delta.abs());
            }
        }
        (exact, total, worst)
    }

    /// The claim the whole narrowband effort is for.
    ///
    /// Every sample of every frame of every rate identical to what TS 26.073's
    /// own decoder produces. Per-stage exactness cannot reach the state
    /// coupling *between* stages, which is where all six wideband defects
    /// lived; only this can.
    #[test]
    fn the_decoder_matches_the_reference_sample_for_sample() {
        for mode in 0..8u8 {
            let (exact, total, worst) = run(mode);
            assert!(total > 0, "mode {mode} compared nothing");
            assert_eq!(
                exact, total,
                "mode {mode}: {exact}/{total} samples exact, worst error {worst}"
            );
        }
    }

    /// The output is 13-bit linear, which is not a detail: comparing unmasked
    /// output against the reference PCM scores near zero for a perfectly
    /// correct decoder, and the wideband port lost a session to the 14-bit
    /// version of exactly this.
    #[test]
    fn every_output_sample_is_thirteen_bit() {
        let (bits, _) = fixture(4);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mut decoder = Decoder::new();
        for frame in &frames {
            for sample in decoder.decode(4, &frame.data).expect("decodes") {
                assert_eq!(sample & 7, 0, "sample {sample} has low bits set");
            }
        }
    }

    /// Decoding is stateful across frames, so a reset decoder must reproduce
    /// the start of the stream exactly.
    #[test]
    fn resetting_returns_the_decoder_to_the_start_of_the_stream() {
        let (bits, _) = fixture(2);
        let (_, frames) = storage::read(bits).expect("fixture parses");

        let mut decoder = Decoder::new();
        let first: Vec<i16> = frames[..3]
            .iter()
            .flat_map(|f| decoder.decode(2, &f.data).expect("decodes"))
            .collect();
        for f in &frames[3..10] {
            decoder.decode(2, &f.data).expect("decodes");
        }
        decoder.reset();
        let again: Vec<i16> = frames[..3]
            .iter()
            .flat_map(|f| decoder.decode(2, &f.data).expect("decodes"))
            .collect();
        assert_eq!(first, again, "reset left state behind");
    }

    /// A truncated payload is rejected rather than decoded from whatever bits
    /// happen to follow it in memory.
    #[test]
    fn a_short_payload_is_rejected() {
        let mut decoder = Decoder::new();
        assert!(decoder.decode(4, &[0u8; 3]).is_none());
        assert!(decoder.decode(4, &[]).is_none());
    }

    /// Concealment is where a decoder is least likely to be right by accident:
    /// it is pure state machine, it only runs once something has already gone
    /// wrong, and getting it wrong sounds like a bad network rather than like a
    /// bug. Every other fixture is a clean stream, so without this the
    /// `FrameState::Bad` path had never been taken end to end.
    ///
    /// The erasure pattern in `tools/build-amr-erasure-fixtures.sh` is chosen
    /// to move the state machine rather than to be representative: a single
    /// loss, a burst of three, the first good frame after the burst — where
    /// both gain concealers limit against the last known-good value — and then
    /// an alternating pair that keeps the machine from settling.
    #[test]
    fn concealment_matches_the_reference_frame_for_frame() {
        let bits: &[u8] = include_bytes!("../testdata/amrnb_erased.amr");
        let want = reference(include_bytes!("../testdata/amrnb_erased.pcm"));
        let (_, frames) = storage::read(bits).expect("fixture parses");

        let erased: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.quality_ok)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            erased,
            vec![5, 10, 11, 12, 20, 22],
            "the fixture's erasure pattern moved; the test below assumes it"
        );

        let mut decoder = Decoder::new();
        let mut compared = 0usize;
        for (f, frame) in frames.iter().enumerate() {
            let state = if frame.quality_ok {
                FrameState::Good
            } else {
                FrameState::Bad
            };
            // A bad frame still carries bits, and the reference decodes them:
            // the parameters are read normally and the concealment machinery
            // then overrides the parts it does not trust.
            let params = super::super::bitstream::parse(4, &frame.data).expect("parses");
            let got = decoder.decode_parameters(4, &params, state);
            for (i, &sample) in got.iter().enumerate() {
                let index = f * L_FRAME + i;
                assert_eq!(
                    sample, want[index],
                    "frame {f} ({state:?}) sample {i} differs"
                );
                compared += 1;
            }
        }
        assert_eq!(
            compared,
            want.len(),
            "compared fewer samples than the fixture holds"
        );
    }

    /// The same erasure pattern, but the frames are *lost* rather than damaged.
    ///
    /// This is the test that decides whether `build_CN_param` was read
    /// correctly: everything downstream of the parameter vector is already
    /// bit-exact, so a divergence can only be in the seed recurrence or the
    /// window walk, and it would appear at the first loss and nowhere earlier.
    #[test]
    fn lost_frames_match_the_reference_frame_for_frame() {
        let bits: &[u8] = include_bytes!("../testdata/amrnb_lost.amr");
        let want = reference(include_bytes!("../testdata/amrnb_lost.pcm"));
        let (_, frames) = storage::read(bits).expect("fixture parses");

        // Frame type 15 is NO_DATA: nothing arrived, so there is no payload to
        // parse and the mode comes from the stream's history instead.
        let lost: Vec<usize> = frames
            .iter()
            .enumerate()
            .filter(|(_, f)| f.frame_type == AmrFrameType::NoData)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            lost,
            vec![5, 10, 11, 12, 20, 22],
            "the fixture's loss pattern moved; the test below assumes it"
        );

        let mut decoder = Decoder::new();
        let mut compared = 0usize;
        let mut concealed = 0usize;
        for (f, frame) in frames.iter().enumerate() {
            let got = if frame.frame_type == AmrFrameType::NoData {
                concealed += 1;
                decoder.conceal_lost_frame(4).expect("not the first frame")
            } else {
                let params = super::super::bitstream::parse(4, &frame.data).expect("parses");
                decoder.decode_parameters(4, &params, FrameState::Good)
            };
            for (i, &sample) in got.iter().enumerate() {
                let index = f * L_FRAME + i;
                assert_eq!(sample, want[index], "frame {f} sample {i} differs");
                compared += 1;
            }
        }
        assert_eq!(concealed, 6, "the concealment path was not taken six times");
        assert_eq!(
            compared,
            want.len(),
            "compared fewer samples than the fixture holds"
        );
    }

    /// The first frame of a stream cannot be concealed, and says so.
    ///
    /// The reference's `dtxGlobalState` starts at `DTX`, so a frame lost
    /// before any has been received goes to comfort noise rather than to
    /// `build_CN_param`. Measured against the reference decoder: feeding it a
    /// stream whose frame 0 is frame type 15 yields
    /// `[0, 0, 0, 0, -8, -16, 0, 0]`, where this path yields silence. The
    /// committed fixture cannot catch that -- its first loss is frame 5 --
    /// which is exactly why the boundary is asserted here instead.
    #[test]
    fn the_first_frame_of_a_stream_is_refused_rather_than_approximated() {
        let mut decoder = Decoder::new();
        assert!(decoder.conceal_lost_frame(4).is_err());

        // One good frame is enough to leave the DTX state behind.
        let (bits, _) = fixture(4);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let params = super::super::bitstream::parse(4, &frames[0].data).expect("parses");
        let _ = decoder.decode_parameters(4, &params, FrameState::Good);
        assert!(
            decoder.conceal_lost_frame(4).is_ok(),
            "the refusal outlived the first frame"
        );

        // And it comes back after a reset, because the reference's state does.
        decoder.reset();
        assert!(decoder.conceal_lost_frame(4).is_err());
    }

    /// A lost frame and a damaged one must not decode alike.
    ///
    /// The two fixtures erase the same frames of the same stream and differ
    /// only in how: one clears the quality bit, the other replaces the frame
    /// with `NO_DATA`. If concealment ignored the manufactured parameters --
    /// the mistake this module exists to avoid -- both would produce identical
    /// audio and every other assertion here would still pass.
    #[test]
    fn a_lost_frame_does_not_decode_like_a_damaged_one() {
        let lost = reference(include_bytes!("../testdata/amrnb_lost.pcm"));
        let damaged = reference(include_bytes!("../testdata/amrnb_erased.pcm"));
        let clean = reference(include_bytes!("../testdata/amrnb_mode4.pcm"));
        assert_eq!(lost.len(), damaged.len());
        assert_ne!(lost, damaged, "lost and damaged decoded identically");
        assert_ne!(lost, clean, "the losses had no effect at all");
        // And the difference is confined to the frames that were lost and what
        // follows them -- frames 0..5 arrived intact in both streams.
        let head = 5 * L_FRAME;
        assert_eq!(
            lost[..head],
            damaged[..head],
            "the streams differ before the first erasure"
        );
    }

    /// Prints the first stage at which the Rust decoder leaves the reference,
    /// for diffing against `tools/trace-amrnb-reference.sh`.
    ///
    /// Ignored because it is a debugging instrument rather than an assertion:
    /// it reports, it does not judge.
    #[test]
    #[ignore = "debugging instrument; run with --nocapture against a reference trace"]
    fn nb_where_does_it_diverge() {
        // As in `nb_dump_trace`: the first line would otherwise be appended to
        // the still-open test-name line, and mode 0 — the only rate that has
        // ever been wrong here — would be the one silently swallowed.
        println!();
        for mode in 0..8u8 {
            let (exact, total, worst) = run(mode);
            let pct = 100.0 * f64::from(u32::try_from(exact).expect("sample count"))
                / f64::from(u32::try_from(total).expect("sample count"));
            println!("mode {mode}: {exact}/{total} exact ({pct:.1}%), worst |delta| {worst}");
        }
    }

    /// Emit the Rust decoder's trace in the reference's own format, so the two
    /// can be diffed line for line.
    ///
    /// ```text
    /// tools/trace-amrnb-reference.sh 4
    /// AMR_TRACE_MODE=4 cargo test -p rvoip-codec-core --all-features \
    ///     nb_dump_trace -- --ignored --nocapture > /tmp/rust-trace.txt
    /// ```
    ///
    /// Ignored because it prints rather than asserts. Reasoning about output
    /// PCM found none of the six wideband defects; this found all of them.
    #[test]
    #[ignore = "debugging instrument; prints a trace for diffing"]
    fn nb_dump_trace() {
        let mode: u8 = std::env::var("AMR_TRACE_MODE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let (bits, _) = fixture(mode);
        let (_, frames) = storage::read(bits).expect("fixture parses");

        // Start on a fresh line. `cargo test --nocapture` leaves the test-name
        // line open, so the first `println!` is appended to it and a filter on
        // `^T ` drops that row without a word. The C instrumentation hit the
        // identical trap earlier — the reference's progress output merging into
        // the first trace line — and it cost fifty rows there before it was
        // noticed. A missing row reads as agreement.
        println!();

        let mut decoder = Decoder::new();
        for (f, frame) in frames.iter().enumerate() {
            decoder.decode(mode, &frame.data).expect("decodes");
            for (name, value) in &decoder.trace {
                println!("T {f} -1 {name} {value}");
            }
            for (name, values) in &decoder.vtrace {
                let body: Vec<String> = values.iter().map(ToString::to_string).collect();
                println!("T {f} -1 {name} {}", body.join(" "));
            }
        }
    }
}
