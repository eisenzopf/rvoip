//! The AMR-WB encoder proper: one frame of speech in, one packed frame out.
//!
//! Everything else under `enc/` is a stage that was made bit-exact on its own
//! against TS 26.173's instrumented encoder. This module is the sequence those
//! stages run in — TS 26.173 `coder()` — plus the three things that belong to
//! no stage and therefore had nowhere else to live:
//!
//! * the **local synthesis** loop, which is what makes an ACELP encoder closed
//!   loop at all: the target for subframe *n+1* is built from the excitation
//!   subframe *n* actually chose, so a divergence compounds rather than staying
//!   local;
//! * the **voice activity detector** (`wb_vad.c`), whose single output bit is
//!   the first bit of every speech frame and whose state is advanced twice more
//!   per frame by the open-loop pitch gain;
//! * the **high-band analysis** of `cod_main.c`'s static `synthesis()`, which
//!   exists only at 23.85 kbit/s and contributes four bits per subframe.
//!
//! # Two excitations, and why they are not one buffer
//!
//! At 23.85 kbit/s the reference builds a *second* excitation from a high-pass
//! filtered codeword and a noise-smoothed gain, and feeds only that to the high
//! band. The adaptive codebook history — the thing every later subframe
//! predicts from — is the *first* excitation. Sharing one buffer gives an
//! encoder that sounds right and whose bitstream is wrong from the second
//! subframe of the first 23.85 frame onward. [`highband::HighBand::
//! enhanced_excitation`] returns a fresh vector for exactly that reason.
//!
//! # Q-formats
//!
//! The frame's speech, its excitation and the pitch target share `Q_new` from
//! the pre-emphasis; the pitch target and impulse response carry a further
//! frame-level `shift` in `-3..=0`. Predictor coefficients are Q12, gains Q14
//! (pitch) and Q16 (code, as a `Word32`), the algebraic codeword Q9, and the
//! impulse response Q15 (`h1`) or Q12 (`h2`). Each function states what it
//! wants; nothing here re-derives a scaling a stage already chose.
//!
//! # What validated it
//!
//! `testdata/wb_enc_trace.txt` — every traced intermediate of three frames at
//! 12.65 kbit/s, with the comparison count asserted — and
//! `testdata/amrwb_enc_mode*.amr`, the reference encoder's own output for fifty
//! frames at each of the nine rates, compared byte for byte.

// This module transcribes reference fixed-point arithmetic and a bit layout.
// The lints below fight the transcription rather than the code.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unreadable_literal,
    clippy::similar_names,
    clippy::too_many_lines
)]

use super::super::isf_noise;
use super::super::lp::isf::{interpolate_isp, isf_to_isp};
use super::super::lp::isf_dequant::{IsfQuantizer, ISF_INIT};
use super::super::lp::isp_to_lp::isp_to_lp;
use super::super::math::{dot_product12, isqrt_n, scale_sig};
use super::super::sort_tables::{
    SORT_1265, SORT_1425, SORT_1585, SORT_1825, SORT_1985, SORT_2305, SORT_2385, SORT_660,
    SORT_885, SORT_SID,
};
use super::analysis::{deemph2, residu, weight_a, FrontEnd, GAMMA1, L_SUBFR, TILT_FAC};
use super::codebook::{
    correlate_target, pitch_sharpen, search_multi_pulse, search_two_pulse, PulseBudget, PITCH_SHARP,
};
use super::dtx::{DtxEncoder, TxDecision};
use super::gain_quant::{scale_code_gain, GainBits, GainInputs, GainPredictor, PitchCorrelations};
use super::isf_quant::IsfEncoder;
use super::pitch::{
    choose_ltp, closed_loop_lag, predict_adaptive, update_target, GainClipping, LagResolution,
    LagWindow, OpenLoopPitch, PitchMode, SearchLimits, WeightedSpeechHighPass, PIT_MAX,
};
use super::preproc::{restrict_to_14_bit, L_FRAME, L_FRAME16K, L_TOTAL, NEW_SPEECH};

use crate::fixed_point::arith::{add, extract_h, mult, negate, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu, l_mult, l_negate};
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::{l_shl, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

use highband::HighBand;
use vad::VoiceActivityDetector;

/// LP order.
pub const M: usize = 16;

/// Subframes per frame.
pub const NB_SUBFR: usize = 4;

/// Offset of `speech[0]` inside the 384-word speech buffer.
///
/// `L_TOTAL - L_FRAME - L_NEXT`: the frame the subframe loop codes begins 64
/// words in, and the 64 words in front of it are the history a `Residu` reaches
/// back over.
const SPEECH: usize = 64;

/// Excitation history carried between frames, `PIT_MAX + L_INTERPOL`.
const EXC_HISTORY: usize = PIT_MAX as usize + 17;

/// The whole working excitation buffer, `(L_FRAME + 1) + PIT_MAX + L_INTERPOL`.
const EXC_LEN: usize = L_FRAME + 1 + EXC_HISTORY;

/// One subframe at 16 kHz.
const L_SUBFR16K: usize = 80;

/// Frame sizes in bits, indexed by mode. Every mode test in `coder()` keys on
/// the budget rather than on the mode number.
const NB_OF_BITS: [usize; 10] = [132, 177, 253, 285, 317, 365, 397, 461, 477, 35];

/// Shortest pitch lag, in 12.8 kHz samples.
const PIT_MIN: i16 = 34;
/// The 9-bit index drops to half-sample resolution here.
const PIT_FR2: i16 = 128;
/// The 9-bit index drops to whole-sample resolution here.
const PIT_FR1_9B: i16 = 160;
/// The 8-bit index drops to whole-sample resolution here.
const PIT_FR1_8B: i16 = 92;

/// Frame budget at and below which the 12-bit two-pulse codebook is used.
const NBBITS_7K: usize = 132;
/// Frame budget from which the high band is analysed and transmitted.
const NBBITS_24K: usize = 477;

/// The rate a frame is encoded at.
///
/// A newtype over the mode index rather than a bare integer, because almost
/// every branch in `coder()` is on the *bit budget* and not on the index, and
/// the two are easy to confuse where both are small integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rate(u8);

impl Rate {
    /// The comfort-noise pseudo-rate, frame type 9.
    ///
    /// Not reachable through [`Self::from_index`]: it is a decision the DTX
    /// handler makes about a frame, not a rate a caller can ask for, and the
    /// ISF quantiser and index widths below are meaningless for it.
    pub const SID: Self = Self(9);

    /// The rate for mode index 0..=8, or `None` for anything else.
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        if index <= 8 {
            Some(Self(index))
        } else {
            None
        }
    }

    /// Whether this is the comfort-noise pseudo-rate.
    #[must_use]
    pub const fn is_sid(self) -> bool {
        self.0 == 9
    }

    /// The mode index, 0..=8.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Speech bits per frame.
    #[must_use]
    pub const fn bits(self) -> usize {
        NB_OF_BITS[self.0 as usize]
    }

    /// Bytes the MIME/storage payload occupies, excluding the table-of-contents
    /// byte.
    #[must_use]
    pub const fn packed_bytes(self) -> usize {
        self.bits().div_ceil(8)
    }

    /// The table-of-contents byte RFC 4867 storage format puts in front of the
    /// payload: mode in bits 6..3, quality bit set.
    #[must_use]
    pub const fn toc_byte(self) -> u8 {
        (self.0 << 3) | 0x04
    }

    /// Payload bit order, TS 26.201: `codec_bits[table[i]]` is payload bit `i`.
    const fn sort_table(self) -> &'static [u16] {
        match self.0 {
            0 => &SORT_660,
            1 => &SORT_885,
            2 => &SORT_1265,
            3 => &SORT_1425,
            4 => &SORT_1585,
            5 => &SORT_1825,
            6 => &SORT_1985,
            7 => &SORT_2305,
            9 => &SORT_SID,
            _ => &SORT_2385,
        }
    }

    /// The ISF quantiser this rate can afford.
    ///
    /// # Panics
    /// On [`Self::SID`], which quantises its spectrum with the comfort-noise
    /// codebooks instead and never reaches here.
    const fn isf_quantiser(self) -> IsfQuantizer {
        assert!(!self.is_sid(), "a SID frame has no speech ISF quantiser");
        if self.bits() <= NBBITS_7K {
            IsfQuantizer::Bits36
        } else {
            IsfQuantizer::Bits46
        }
    }

    /// Widths of the transmitted ISF indices, in order.
    const fn isf_index_widths(self) -> &'static [usize] {
        if self.bits() <= NBBITS_7K {
            &[8, 8, 7, 7, 6]
        } else {
            &[8, 8, 6, 7, 7, 5, 5]
        }
    }

    /// Widths of the transmitted algebraic codebook indices, in order.
    ///
    /// Transcribed from the seven-way ladder of `cod_main.c` 1138–1212. The
    /// splits are not derivable from the pulse count: 44 bits is 13/13/9/9 and
    /// 72 bits is 10/10/2/2/10/10/14/14, both of which depend on which tracks
    /// the extra pulses were forced onto.
    const fn pulse_index_widths(self) -> &'static [usize] {
        match self.bits() {
            0..=132 => &[12],
            133..=177 => &[5, 5, 5, 5],
            178..=253 => &[9, 9, 9, 9],
            254..=285 => &[13, 13, 9, 9],
            286..=317 => &[13, 13, 13, 13],
            318..=365 => &[2, 2, 2, 2, 14, 14, 14, 14],
            366..=397 => &[10, 10, 2, 2, 10, 10, 14, 14],
            _ => &[11, 11, 11, 11, 11, 11, 11, 11],
        }
    }

    /// Whether the frame carries a high-band correction gain per subframe.
    const fn has_high_band(self) -> bool {
        self.bits() >= NBBITS_24K
    }
}

// ---------------------------------------------------------------------------
// Bit assembly
// ---------------------------------------------------------------------------

/// The reference's `prms[]`: one entry per codec bit, in codec order.
///
/// One byte per bit rather than packed, because the payload order is a
/// permutation of this and a permutation wants random access. Packing happens
/// once, in [`Self::pack`].
struct Parameters {
    bits: [u8; 477],
    len: usize,
}

impl Parameters {
    const fn new() -> Self {
        Self {
            bits: [0; 477],
            len: 0,
        }
    }

    /// Append `width` bits of `value`, most significant first (`Parm_serial`).
    fn push(&mut self, value: u16, width: usize) {
        for i in (0..width).rev() {
            self.bits[self.len] = ((value >> i) & 1) as u8;
            self.len += 1;
        }
    }

    /// Sort and pack into the MIME/storage payload of `rate` (`bits.c`
    /// 176–212).
    ///
    /// The trailing bits of the final octet are zero: the reference shifts the
    /// partial byte up rather than down, so the pad sits at the bottom.
    fn pack(&self, rate: Rate) -> Vec<u8> {
        let sort = rate.sort_table();
        debug_assert_eq!(sort.len(), self.len, "every codec bit must be assigned");
        let mut out = vec![0u8; rate.packed_bytes()];
        for (i, &source) in sort.iter().enumerate() {
            out[i / 8] |= self.bits[source as usize] << (7 - (i % 8));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Per-stage trace
// ---------------------------------------------------------------------------

/// A recording of every intermediate the instrumented reference dumps.
///
/// Off by default. The row names are exactly those of
/// `testdata/wb_enc_trace.txt`, so a divergence is found by comparing rows
/// rather than by reasoning about output — which is the only method that has
/// ever worked on this codec.
#[derive(Clone, Debug, Default)]
pub struct EncoderTrace {
    rows: Vec<(usize, i32, &'static str, Vec<i32>)>,
}

impl EncoderTrace {
    /// Every recorded row: `(frame, subframe, name, values)`, subframe `-1` for
    /// frame-level rows.
    #[must_use]
    pub fn rows(&self) -> &[(usize, i32, &'static str, Vec<i32>)] {
        &self.rows
    }

    /// One row's values, or `None` if it was never recorded.
    #[must_use]
    pub fn row(&self, frame: usize, subframe: i32, name: &str) -> Option<&[i32]> {
        self.rows
            .iter()
            .find(|r| r.0 == frame && r.1 == subframe && r.2 == name)
            .map(|r| r.3.as_slice())
    }
}

// ---------------------------------------------------------------------------
// The encoder
// ---------------------------------------------------------------------------

/// The AMR-WB encoder, TS 26.190 / TS 26.173 `coder()`.
///
/// Owns every field of the reference's `Coder_State`. One instance encodes one
/// stream; the rate may change from frame to frame, and nothing is reset when
/// it does — which is the reference's behaviour and is why a mode switch is
/// audible for a few frames rather than instantaneous.
///
/// Discontinuous transmission is implemented and off by default, as the
/// reference's own encoder is without `-dtx`; enabled, the `dtx` field below
/// produces SID and `NO_DATA` frames on the reference's schedule, pinned against
/// the normative `tst_md` sequence.
pub struct WbEncoder {
    front: FrontEnd,
    isf_quantiser: IsfEncoder,
    open_loop: OpenLoopPitch,
    /// The open-loop *gain* the tone detector needs, measured alongside
    /// [`Self::open_loop`] because that stage does not expose it.
    tone_probe: OpenLoopGainProbe,
    clipping: GainClipping,
    gains: GainPredictor,
    vad: VoiceActivityDetector,
    /// Discontinuous transmission: the frame classifier, the eight-frame
    /// background history and the comfort-noise generator.
    ///
    /// Present whether or not DTX is enabled for a session, because
    /// 23.85 kbit/s reads its hangover counter on ordinary speech frames.
    dtx: DtxEncoder,
    /// Whether the DTX handler may turn a quiet frame into comfort noise.
    ///
    /// Off by default: DTX changes the *frame types* on the wire, so a caller
    /// that has not negotiated it must never see one. The hangover counter
    /// runs either way, because 23.85 kbit/s reads it.
    allow_dtx: bool,
    high_band: HighBand,

    /// `st->old_exc`: excitation history, in the previous frame's `Q_new`.
    old_exc: [Word16; EXC_HISTORY],
    /// `st->mem_syn`: the local synthesis filter's memory.
    mem_syn: [Word16; M],
    /// `st->mem_w0`: the weighting filter's memory, i.e. the target's.
    mem_w0: Word16,
    /// `st->tilt_code`: the previous subframe's voicing, Q15.
    tilt_code: Word16,
    /// `st->ispold_q`: the previous frame's *quantised* ISPs.
    isp_old_q: [Word16; M],
    /// `st->isfold`: the previous frame's quantised ISFs, for the stability
    /// measure.
    isf_old: [Word16; M],
    /// `st->first_frame`: whether `ispold_q` still holds its reset value.
    first_frame: bool,
    /// `st->vad_hist`: consecutive frames the detector has called non-speech.
    vad_hist: i16,

    trace: Option<EncoderTrace>,
    frame_index: usize,
}

impl Default for WbEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl WbEncoder {
    /// A cold-start encoder, matching `Init_coder` / `Reset_encoder(st, 1)`.
    #[must_use]
    pub fn new() -> Self {
        let mut isf_old = [Word16(0); M];
        for (slot, &v) in isf_old.iter_mut().zip(ISF_INIT.iter()) {
            *slot = Word16(v);
        }
        Self {
            front: FrontEnd::new(),
            isf_quantiser: IsfEncoder::new(),
            open_loop: OpenLoopPitch::new(),
            tone_probe: OpenLoopGainProbe::new(),
            clipping: GainClipping::new(),
            gains: GainPredictor::new(),
            vad: VoiceActivityDetector::new(),
            dtx: DtxEncoder::new(),
            allow_dtx: false,
            high_band: HighBand::new(),
            old_exc: [Word16(0); EXC_HISTORY],
            mem_syn: [Word16(0); M],
            mem_w0: Word16(0),
            tilt_code: Word16(0),
            isp_old_q: super::analysis::ISP_INIT,
            isf_old,
            first_frame: true,
            vad_hist: 0,
            trace: None,
            frame_index: 0,
        }
    }

    /// `Reset_encoder(st, 0)`: what a comfort-noise frame clears, and what it
    /// deliberately does not.
    ///
    /// Cleared: the excitation history, the synthesis and weighting memories,
    /// the ISF quantiser's predictor, the voicing tilt, the gain-clipping
    /// state and the high band's slow gain threshold.
    ///
    /// Kept, and this is the half that matters: the *gain predictor* -- the
    /// reference resets it only under `reset_all`, so a talk spurt resuming
    /// after silence inherits its history -- along with `gain_alpha`, every
    /// high-band filter memory, the noise seed, both ISP memories and the DTX
    /// state itself. `first_frame` going back to true is how `isp_old_q`
    /// recovers: the next speech frame copies rather than interpolating from a
    /// stale value.
    fn reset_after_comfort_noise(&mut self) {
        self.old_exc = [Word16(0); EXC_HISTORY];
        self.mem_syn = [Word16(0); M];
        self.mem_w0 = Word16(0);
        self.tilt_code = Word16(0);
        self.first_frame = true;
        self.isf_quantiser.reset();
        self.clipping = GainClipping::new();
        self.high_band.reset_gain_threshold();
    }

    /// Enable discontinuous transmission.
    ///
    /// With it on, a frame the detector calls quiet may be coded as comfort
    /// noise instead of speech, and [`Self::encode_frame_typed`] reports that.
    /// [`Self::encode_frame`] always codes speech regardless, because it has
    /// nowhere to say otherwise.
    #[allow(clippy::missing_const_for_fn)]
    pub fn set_allow_dtx(&mut self, allow: bool) {
        self.allow_dtx = allow;
    }

    /// Walk the DTX hangover counter down, without running the detector.
    ///
    /// Test-only. 23.85 kbit/s reads this counter on ordinary speech frames,
    /// and with DTX off it never moves -- so nothing else can demonstrate that
    /// the value actually reaches the transmitted gain index.
    #[cfg(test)]
    pub(crate) fn force_dtx_hangover(&mut self, quiet_frames: usize) {
        let mut ctx = DspContext::default();
        for _ in 0..quiet_frames {
            self.dtx.classify(&mut ctx, false);
        }
    }

    /// Start recording per-stage intermediates.
    ///
    /// A diagnostic rather than part of encoding: the recording is what a
    /// divergence against `testdata/wb_enc_trace.txt` is found with.
    pub fn record_trace(&mut self) {
        self.trace = Some(EncoderTrace::default());
    }

    /// Take the recording so far, leaving recording enabled but empty.
    #[must_use]
    pub fn take_trace(&mut self) -> Option<EncoderTrace> {
        self.trace.replace(EncoderTrace::default())
    }

    fn trc(&mut self, subframe: i32, name: &'static str, values: &[Word16]) {
        let frame = self.frame_index;
        if let Some(trace) = self.trace.as_mut() {
            let row = values.iter().map(|v| i32::from(v.0)).collect();
            trace.rows.push((frame, subframe, name, row));
        }
    }

    fn trc1(&mut self, subframe: i32, name: &'static str, value: i32) {
        let frame = self.frame_index;
        if let Some(trace) = self.trace.as_mut() {
            trace.rows.push((frame, subframe, name, vec![value]));
        }
    }

    /// Encode one 20 ms frame of 16 kHz mono PCM into its MIME/storage payload.
    ///
    /// The returned bytes are the frame's speech bits only —
    /// [`Rate::packed_bytes`] of them — with no table-of-contents byte in
    /// front; [`Rate::toc_byte`] supplies that.
    pub fn encode_frame(&mut self, pcm: &[i16; L_FRAME16K], rate: Rate) -> Vec<u8> {
        self.encode_frame_typed(pcm, rate).1
    }

    /// One frame, reporting whether it came out as speech or comfort noise.
    ///
    /// The payload for a comfort-noise frame is the 35-bit SID, five bytes.
    /// Whether the *transmitter* sends it is a separate decision -- see
    /// [`SidCadence`](crate::codecs::amr::sid_cadence::SidCadence) -- because
    /// the encoder builds a SID on every comfort-noise frame and most are
    /// discarded.
    pub fn encode_frame_typed(&mut self, pcm: &[i16; L_FRAME16K], rate: Rate) -> (bool, Vec<u8>) {
        let mut prms = Parameters::new();
        let comfort_noise = self.encode_into(pcm, rate, &mut prms);
        let payload = prms.pack(if comfort_noise { Rate::SID } else { rate });
        self.frame_index += 1;
        (comfort_noise, payload)
    }

    /// One frame of `coder()`, writing parameters in transmission order.
    fn encode_into(&mut self, pcm: &[i16; L_FRAME16K], rate: Rate, prms: &mut Parameters) -> bool {
        let ser_size = rate.bits();
        let mut mode = PitchMode::from_frame_bits(ser_size as u16);
        let mut ctx = DspContext::default();

        // --- input conditioning, LP analysis, weighted speech ---------------
        let mut input = [Word16(0); L_FRAME16K];
        for (slot, &sample) in input.iter_mut().zip(pcm.iter()) {
            *slot = Word16(sample);
        }
        // TS 26.173 masks the two low bits in its *driver*, not in `coder()`.
        // It is nonetheless part of the codec: without it the decimator output
        // moves within the first twenty samples and never recovers.
        restrict_to_14_bit(&mut input);
        self.trc(-1, "speech16k", &input);

        let frame = self.front.process_frame(&input);
        let q_new = frame.scaling.q_new;
        let q_exp = frame.scaling.exp;
        let shift = frame.wsp_shift;

        // The buffers the front end deliberately does not own follow the
        // frame's change of scaling here. `mem_preemph` is excluded — it lives
        // in the unscaled domain — and so is `tilt_code`, which is a ratio.
        scale_sig(&mut ctx, &mut self.old_exc, q_exp);
        scale_sig(&mut ctx, &mut self.mem_syn, q_exp);
        scale_sig(&mut ctx, core::slice::from_mut(&mut self.mem_w0), q_exp);

        self.trc1(-1, "Q_new", i32::from(q_new));
        self.trc(-1, "window", &frame.window);
        self.trc(-1, "r_h_pre", &frame.autocorr_raw.high);
        self.trc(-1, "r_l_pre", &frame.autocorr_raw.low);
        self.trc(-1, "r_h", &frame.autocorr.high);
        self.trc(-1, "r_l", &frame.autocorr.low);
        self.trc(-1, "A", &frame.a);
        self.trc(-1, "rc", &frame.rc);
        self.trc(-1, "ispnew", &frame.isp);
        self.trc(-1, "A_interp", &flatten(&frame.a_interp));
        self.trc(-1, "wsp", &frame.wsp);
        self.trc1(-1, "wsp_shift", i32::from(shift));

        // --- voice activity -------------------------------------------------
        // The detector sees the pre-emphasised frame brought back to Q1, which
        // is a different signal from anything the ACELP path uses.
        let mut vad_input = [Word16(0); L_FRAME];
        vad_input.copy_from_slice(&frame.window[NEW_SPEECH..NEW_SPEECH + L_FRAME]);
        scale_sig(&mut ctx, &mut vad_input, 1 - q_new);
        let vad_flag = self.vad.process(&mut ctx, &vad_input);
        self.vad_hist = if vad_flag {
            0
        } else {
            add(&mut ctx, Word16(self.vad_hist), Word16(1)).0
        };
        // --- discontinuous transmission -------------------------------------
        // Runs before the VAD bit and may replace the whole frame with comfort
        // noise. Guarded, because the reference guards it: with DTX off
        // `tx_dtx_handler` is not called at all, so the hangover counter never
        // leaves its reset value and 23.85 kbit/s's `gain_alpha` stays pinned
        // at unity. Calling it anyway and discarding the decision would move
        // mode 8's bitstream on quiet input.
        let comfort_noise =
            self.allow_dtx && self.dtx.classify(&mut ctx, vad_flag) == TxDecision::ComfortNoise;

        // A SID frame carries no VAD bit: the frame type says what it is.
        if comfort_noise {
            // Every predicate downstream is on the bit budget, and a SID frame
            // has 35 of them. `PitchMode` already documents and tests that
            // case -- it takes the two-half open-loop path, like 8.85 upward.
            mode = PitchMode::from_frame_bits(Rate::SID.bits() as u16);
        } else {
            prms.push(u16::from(vad_flag), 1);
        }

        // --- pitch-clipping resonance measure, on the *unquantised* ISFs -----
        // With the SID budget in hand, deliberately: the reference re-reads
        // `ser_size` before this call, so a comfort-noise frame observes
        // clipping against a different threshold than a speech frame would.
        self.clipping.observe_isf(&mut ctx, mode, &frame.isf);

        // --- open loop pitch --------------------------------------------------
        self.open_loop.rescale(&mut ctx, q_exp, shift);
        self.tone_probe.rescale(&mut ctx, q_exp, shift);
        let lags = self.open_loop.analyse(&mut ctx, &frame.wsp, mode);
        if let Some(median) = lags.smoothed_after_first_half {
            self.trc1(-1, "T_op_med", i32::from(median));
        }
        // The tone detector wants each half's normalised correlation, in the
        // order the halves were analysed.
        let half = if mode.open_loop_spans_frame() {
            frame.wsp.len()
        } else {
            frame.wsp.len() / 2
        };
        let tone_gain = self
            .tone_probe
            .measure(&mut ctx, &frame.wsp[..half], lags.first_half / 2);
        self.vad.tone_detection(&mut ctx, tone_gain);
        if !mode.open_loop_spans_frame() {
            let tone_gain =
                self.tone_probe
                    .measure(&mut ctx, &frame.wsp[half..], lags.second_half / 2);
            self.vad.tone_detection(&mut ctx, tone_gain);
        }

        // --- comfort noise, and nothing after it -----------------------------
        // Everything above ran on both paths: the front end, the VAD, the
        // clipping observation and both open-loop halves with their tone
        // detection. What a SID frame skips is the ISF quantiser, the gain
        // predictor and the whole subframe loop.
        if comfort_noise {
            // One `Residu` across all 256 samples, through the *unquantised*
            // interpolated predictor of the fourth subframe. Not per-subframe
            // and not the quantised one -- the other `dtx_buffer` call site
            // downstream uses both of those, and confusing them yields a
            // plausible SID several frames later.
            let mut residual = [Word16(0); L_FRAME];
            residu(
                &frame.a_interp[NB_SUBFR - 1],
                &frame.window[SPEECH - M..SPEECH + L_FRAME],
                &mut residual,
            );

            let mut excitation = [Word16(0); L_FRAME];
            let mut energy = Word32(0);
            for (slot, &sample) in excitation.iter_mut().zip(residual.iter()) {
                *slot = shr(&mut ctx, sample, q_new);
            }
            for &sample in &excitation {
                energy = l_mac(&mut ctx, energy, sample, sample);
            }
            let energy = crate::fixed_point::shift::l_shr(&mut ctx, energy, 1);

            self.dtx
                .buffer(&mut ctx, &frame.isf, energy, rate.index() as usize);
            let sid = self.dtx.build_sid(&mut ctx);
            for (&index, &width) in sid.isf_indices.iter().zip(&isf_noise::SPLIT_BITS) {
                prms.push(index, usize::from(width));
            }
            #[allow(clippy::cast_sign_loss)]
            prms.push(sid.energy_index.0 as u16, 6);
            prms.push(u16::from(sid.dither), 1);

            // The encoder synthesises the comfort noise it just described, so
            // the high band, de-emphasis and output filters carry the right
            // memories into the next speech frame. One predictor for all four
            // subframes -- the CN spectrum does not move within a frame -- and
            // the gain indices it produces are computed and discarded.
            let cn_excitation = self.dtx.excitation(&mut ctx, sid.energy_index);
            let isp = isf_to_isp(&sid.spectrum);
            let aq = isp_to_lp(&isp);
            if rate.has_high_band() {
                let hangover = self.dtx.hangover_count();
                for k in 0..NB_SUBFR {
                    let sub: [Word16; L_SUBFR] = cn_excitation[k * L_SUBFR..(k + 1) * L_SUBFR]
                        .try_into()
                        .expect("one subframe");
                    let reference: [Word16; L_SUBFR16K] = input
                        [k * L_SUBFR16K..(k + 1) * L_SUBFR16K]
                        .try_into()
                        .expect("one subframe at 16 kHz");
                    let _ = self.high_band.analyse(
                        &mut ctx,
                        &aq,
                        &sub,
                        0,
                        &reference,
                        self.vad_hist,
                        hangover,
                    );
                }
            }
            self.isf_old = sid.spectrum;
            self.reset_after_comfort_noise();
            return true;
        }

        // --- ISF quantisation ------------------------------------------------
        let narrow = rate.bits() <= NBBITS_7K;
        self.trc(
            -1,
            if narrow { "isf_unq36" } else { "isf_unq46" },
            &frame.isf,
        );
        let quantised = self
            .isf_quantiser
            .quantize(&frame.isf, rate.isf_quantiser());
        for (&index, &width) in quantised
            .indices()
            .iter()
            .zip(rate.isf_index_widths().iter())
        {
            prms.push(index, width);
        }
        let indice_words: Vec<Word16> = quantised
            .indices()
            .iter()
            .map(|&v| Word16(v as i16))
            .collect();
        self.trc(
            -1,
            if narrow {
                "isf_indice36"
            } else {
                "isf_indice46"
            },
            &indice_words,
        );

        let isf_q = *quantised.isf();
        let stab_fac = self.stability(&mut ctx, &isf_q);
        self.trc(-1, "isf_q", &isf_q);
        self.trc1(-1, "stab_fac", i32::from(stab_fac.0));
        self.isf_old = isf_q;

        // --- quantised LP, interpolated over the four subframes ---------------
        let isp_q = isf_to_isp(&isf_q);
        if self.first_frame {
            self.first_frame = false;
            self.isp_old_q = isp_q;
        }
        let aq = interpolate_isp(&self.isp_old_q, &isp_q);
        self.isp_old_q = isp_q;
        self.trc(-1, "ispnew_q", &isp_q);
        self.trc(-1, "Aq", &flatten(&aq));

        // --- the excitation buffer, history first -----------------------------
        let mut exc = [Word16(0); EXC_LEN];
        exc[..EXC_HISTORY].copy_from_slice(&self.old_exc);

        // The reference fills the whole frame's residual once before the
        // subframe loop and then again per subframe, because `Pred_lt4`
        // overwrites each subframe as it goes. Only the second pass is read on
        // this path; the first is kept where the reference puts it.
        for (k, a) in aq.iter().enumerate() {
            let base = EXC_HISTORY + k * L_SUBFR;
            let at = SPEECH + k * L_SUBFR;
            residu(
                a,
                &frame.window[at - M..at + L_SUBFR],
                &mut exc[base..base + L_SUBFR],
            );
        }

        // --- background history, on a speech frame the detector called quiet ---
        // The second of the two `dtx_buffer` call sites, and it disagrees with
        // the first on both inputs: this one measures the residual through the
        // *quantised* per-subframe predictors, and buffers the *quantised*
        // ISFs. The reference has no `allow_dtx` guard here -- the history
        // fills whether or not DTX is enabled, which is harmless because
        // nothing reads it until a SID is built.
        if !vad_flag {
            let mut scaled = [Word16(0); L_FRAME];
            let mut energy = Word32(0);
            for (slot, &sample) in scaled.iter_mut().zip(&exc[EXC_HISTORY..]) {
                *slot = shr(&mut ctx, sample, q_new);
            }
            for &sample in &scaled {
                energy = l_mac(&mut ctx, energy, sample, sample);
            }
            let energy = crate::fixed_point::shift::l_shr(&mut ctx, energy, 1);
            self.dtx
                .buffer(&mut ctx, &isf_q, energy, rate.index() as usize);
        }

        // --- the subframe loop -------------------------------------------------
        let mut window = LagWindow::around(&mut ctx, lags.first_half);
        for (k, (a, a_quantised)) in frame.a_interp.iter().zip(aq.iter()).enumerate() {
            let absolute = k == 0 || (k == 2 && mode.third_subframe_is_absolute());
            if k == 2 && mode.third_subframe_is_absolute() {
                window = LagWindow::around(&mut ctx, lags.second_half);
            }
            window = self.subframe(
                &mut ctx,
                &SubframeInputs {
                    rate,
                    mode,
                    index: k,
                    window: &frame.window,
                    a,
                    aq: a_quantised,
                    q_new,
                    shift,
                    stab_fac,
                    lag_window: window,
                    absolute,
                    pcm: &input,
                },
                &mut exc,
                prms,
            );
        }

        self.old_exc
            .copy_from_slice(&exc[L_FRAME..L_FRAME + EXC_HISTORY]);
        false
    }

    /// `stab_fac`: how far this frame's quantised ISFs moved from the last,
    /// Q15, clamped at zero.
    ///
    /// Only the low fifteen enter the sum; the sixteenth ISF carries half scale
    /// and is not comparable with the rest. Both the `L_shl` and the final
    /// `shl` can saturate, and the reference says so in a comment rather than
    /// guarding against it.
    fn stability(&self, ctx: &mut DspContext, isf: &[Word16; M]) -> Word16 {
        let mut acc = Word32(0);
        for (&now, &before) in isf.iter().zip(self.isf_old.iter()).take(M - 1) {
            let moved = sub(ctx, now, before);
            acc = l_mac(ctx, acc, moved, moved);
        }
        let tmp = extract_h(l_shl(ctx, acc, 8));
        let tmp = mult(ctx, tmp, Word16(26214));
        let tmp = sub(ctx, Word16(20480), tmp);
        let stab = shl(ctx, tmp, 1);
        if stab.0 < 0 {
            Word16(0)
        } else {
            stab
        }
    }

    /// One subframe: target, closed-loop pitch, codebook, gains, and the local
    /// synthesis that makes the next subframe's target.
    ///
    /// Returns the lag window the next subframe searches, which an absolute
    /// subframe narrows around its own choice.
    fn subframe(
        &mut self,
        ctx: &mut DspContext,
        inputs: &SubframeInputs<'_>,
        exc: &mut [Word16; EXC_LEN],
        prms: &mut Parameters,
    ) -> LagWindow {
        let SubframeInputs {
            rate,
            mode,
            index,
            window,
            a,
            aq,
            q_new,
            shift,
            stab_fac,
            lag_window,
            absolute,
            pcm,
        } = *inputs;
        let sf = index as i32;
        let base = EXC_HISTORY + index * L_SUBFR;
        let at = SPEECH + index * L_SUBFR;
        let ser_size = rate.bits();

        // --- target for the pitch search --------------------------------------
        // The zero-input response is removed by filtering this subframe's
        // residual through 1/A(z) started from `speech - mem_syn`, rather than
        // by computing the ringing separately and subtracting it.
        let mut error = [Word16(0); M + L_SUBFR];
        for i in 0..M {
            error[i] = sub(ctx, window[at - M + i], self.mem_syn[i]);
        }
        residu(
            aq,
            &window[at - M..at + L_SUBFR],
            &mut exc[base..base + L_SUBFR],
        );
        let residual: [Word16; L_SUBFR] = exc[base..base + L_SUBFR]
            .try_into()
            .expect("one subframe of residual");
        {
            let mut memory: [Word16; M] = error[..M].try_into().expect("filter memory");
            let mut filtered = [Word16(0); L_SUBFR];
            syn_filt(ctx, aq, &residual, &mut filtered, &mut memory, false);
            error[M..].copy_from_slice(&filtered);
        }
        let ap = weight_a(a, GAMMA1);
        let mut xn = [Word16(0); L_SUBFR];
        residu(&ap, &error, &mut xn);
        deemph2(&mut xn, TILT_FAC, &mut self.mem_w0);

        // --- the same target in the residual domain, for the sign decision -----
        // First half exactly, second half approximated by the residual itself.
        // That asymmetry is the reference's and it is visible in `cn`.
        let mut cn = [Word16(0); L_SUBFR];
        {
            const HALF: usize = L_SUBFR / 2;
            let mut scratch = [Word16(0); M + HALF];
            scratch[M..].copy_from_slice(&xn[..HALF]);
            let mut memory = Word16(0);
            preemph2(ctx, &mut scratch[M..], TILT_FAC, &mut memory);
            let block: [Word16; HALF] = scratch[M..].try_into().expect("half a subframe");
            let mut filtered = [Word16(0); HALF];
            let mut filter_memory = [Word16(0); M];
            syn_filt(ctx, &ap, &block, &mut filtered, &mut filter_memory, false);
            scratch[M..].copy_from_slice(&filtered);
            residu(aq, &scratch, &mut cn[..HALF]);
        }
        cn[L_SUBFR / 2..].copy_from_slice(&residual[L_SUBFR / 2..]);

        // --- impulse response of the weighted synthesis filter ------------------
        let mut h1 = [Word16(0); L_SUBFR];
        {
            // The weighted numerator is laid into the same buffer the recursion
            // reads, so the first seventeen taps are the numerator and every
            // later one is a sample the loop has already written.
            let mut chain = [Word16(0); M + L_SUBFR];
            chain[M..=(M + M)].copy_from_slice(&ap);
            for i in 0..L_SUBFR {
                let mut acc = l_mult(ctx, chain[i + M], Word16(16384));
                for j in 1..=M {
                    acc = l_msu(ctx, acc, aq[j], chain[i + M - j]);
                }
                let acc = l_shl(ctx, acc, 3);
                let v = round(ctx, acc);
                chain[i + M] = v;
                h1[i] = v;
            }
            let mut memory = Word16(0);
            deemph2(&mut h1, TILT_FAC, &mut memory);
        }
        // Q12 for the codebook search; Q15 with the frame's scaling for the
        // pitch search. Two different vectors from here on.
        let mut h2 = h1;
        scale_sig(ctx, &mut h2, -2);
        scale_sig(ctx, &mut xn, shift);
        scale_sig(ctx, &mut h1, 1 + shift);
        self.trc(sf, "xn", &xn);
        self.trc(sf, "h1", &h1);
        self.trc1(sf, "shift", i32::from(shift));

        // --- closed-loop pitch ---------------------------------------------------
        let resolution = mode.lag_resolution();
        let (t0, t0_frac) = closed_loop_lag(
            ctx,
            &exc[..],
            base,
            &xn,
            &h1,
            SearchLimits {
                window: lag_window,
                absolute,
                resolution,
            },
        );
        let next_window = if absolute {
            LagWindow::around(ctx, t0)
        } else {
            lag_window
        };
        let quarter = resolution == LagResolution::NINE_BIT;
        prms.push(
            encode_lag(ctx, t0, t0_frac, lag_window.min, absolute, quarter),
            lag_index_width(absolute, quarter),
        );

        let clip = self.clipping.clips(ctx, mode);

        predict_adaptive(&mut exc[..], base, t0, t0_frac, L_SUBFR + 1);
        self.trc1(sf, "T0", i32::from(t0));
        self.trc1(sf, "T0_frac", i32::from(t0_frac));
        // The reference samples `T0_min`/`T0_max` after an absolute subframe
        // has already narrowed them around its own choice, so what is traced is
        // the window the *next* subframe will search.
        self.trc1(sf, "T0_min", i32::from(next_window.min));
        self.trc1(sf, "T0_max", i32::from(next_window.max));
        let adaptive: [Word16; L_SUBFR] = exc[base..base + L_SUBFR]
            .try_into()
            .expect("one subframe of adaptive codebook");

        let decision = choose_ltp(ctx, &mut exc[..], base, &xn, &h1, clip, mode);
        self.trc1(sf, "gain1", i32::from(decision.sharp_gain.0));
        self.trc1(sf, "gain2", i32::from(decision.smooth_gain.0));
        self.trc(sf, "adapt", &adaptive);
        self.trc(sf, "y1", &decision.sharp_response);
        if mode.has_sharp_candidate() {
            prms.push(u16::from(decision.prefer_sharp), 1);
        }
        let gain_pit = decision.gain();

        // --- the codebook target, and its residual-domain reference -------------
        let xn2 = decision.codebook_target;
        let chosen: [Word16; L_SUBFR] = exc[base..base + L_SUBFR]
            .try_into()
            .expect("one subframe of chosen adaptive vector");
        let mut cn = update_target(ctx, &cn, &chosen, gain_pit);
        scale_sig(ctx, &mut cn, shift);

        // --- the impulse response the codebook actually searches ----------------
        // Pre-emphasis by the *previous* subframe's voicing, then pitch
        // sharpening: both fold decisions already taken into the search, so the
        // pulses are placed against the excitation as it will finally be.
        let mut memory = Word16(0);
        preemph(ctx, &mut h2, self.tilt_code, &mut memory);
        let sharp_lag = usize::try_from(if t0_frac > 2 { t0 + 1 } else { t0 })
            .expect("a pitch lag is positive");
        pitch_sharpen(ctx, &mut h2, sharp_lag, PITCH_SHARP);

        let mut dn = correlate_target(ctx, &h2, &xn2);
        self.trc(sf, "xn2", &xn2);
        self.trc(sf, "cn", &cn);
        self.trc(sf, "h2", &h2);
        self.trc(sf, "dn", &dn);

        // --- algebraic codebook ---------------------------------------------------
        // `y2` enters the search holding the filtered LTP candidate and is only
        // overwritten when an iteration is accepted. That is not a quirk to
        // normalise away: it is what the reference's `y2` contains.
        let mut y2 = decision.smooth_response;
        let mut code;
        match PulseBudget::from_frame_bits(ser_size) {
            None => {
                let innovation = search_two_pulse(ctx, &mut dn, &cn, &h2);
                code = innovation.code;
                y2 = innovation.filtered;
                prms.push(innovation.index, 12);
            }
            Some(budget) => {
                let innovation =
                    search_multi_pulse(ctx, &mut dn, &cn, &h2, &mut y2, budget, ser_size);
                code = innovation.code;
                for (&index, &width) in innovation
                    .indices
                    .iter()
                    .zip(rate.pulse_index_widths().iter())
                {
                    prms.push(index, width);
                }
            }
        }

        // The codeword gets the same two filters the impulse response got, so
        // that the gain quantiser sees a codeword and a filtered codeword that
        // correspond. `y2` is deliberately *not* refiltered.
        let mut memory = Word16(0);
        preemph(ctx, &mut code, self.tilt_code, &mut memory);
        pitch_sharpen(ctx, &mut code, sharp_lag, PITCH_SHARP);
        self.trc1(sf, "select", i32::from(decision.prefer_sharp));
        self.trc(sf, "code", &code);
        self.trc(sf, "y2", &y2);

        // --- joint gain quantisation ------------------------------------------------
        let correlations = PitchCorrelations {
            energy: decision.correlations.energy,
            energy_exp: decision.correlations.energy_exp,
            correlation: decision.correlations.correlation,
            correlation_exp: decision.correlations.correlation_exp,
        };
        let bits = GainBits::from_frame_bits(ser_size);
        let target_q = add(ctx, Word16(q_new), Word16(shift)).0;
        let quantised = self.gains.quantise(
            ctx,
            &GainInputs {
                target: &xn,
                filtered_adaptive: decision.response(),
                target_q,
                filtered_code: &y2,
                code: &code,
                correlations,
                bits,
                pitch_gain: gain_pit,
                clip_pitch_gain: clip,
            },
        );
        prms.push(quantised.index, bits.bits());
        let gain_pit = quantised.pitch_gain;
        let l_gain_code = quantised.code_gain;
        self.clipping.observe_gain(ctx, mode, gain_pit);
        let gain_code = scale_code_gain(ctx, l_gain_code, q_new);

        // --- voicing, for the next subframe's impulse response ---------------------
        let mut scaled_adaptive: [Word16; L_SUBFR] = chosen;
        scale_sig(ctx, &mut scaled_adaptive, shift);
        let voice_fac = voice_factor(ctx, &scaled_adaptive, shift, gain_pit, &code, gain_code);
        let quarter = shr(ctx, voice_fac, 2);
        self.tilt_code = add(ctx, quarter, Word16(8192));

        // --- the weighting filter's memory, for the next subframe's target ---------
        {
            let acc = l_mult(ctx, gain_code, y2[L_SUBFR - 1]);
            let acc = l_shl(ctx, acc, 5 + shift);
            let acc = l_negate(ctx, acc);
            let acc = l_mac(ctx, acc, xn[L_SUBFR - 1], Word16(16384));
            let acc = l_msu(ctx, acc, decision.response()[L_SUBFR - 1], gain_pit);
            let acc = l_shl(ctx, acc, 1 - shift);
            self.mem_w0 = round(ctx, acc);
        }

        // --- the two excitations -----------------------------------------------------
        // The adaptive codebook history is `exc`, which the loop below writes.
        // The high band's input is a *different* vector, built from the same
        // adaptive contribution but an enhanced codeword and gain.
        let adaptive_contribution: [Word16; L_SUBFR] = chosen;
        for i in 0..L_SUBFR {
            let acc = l_mult(ctx, gain_code, code[i]);
            let acc = l_shl(ctx, acc, 5);
            let acc = l_mac(ctx, acc, exc[base + i], gain_pit);
            // Saturation can occur here and is the reference's behaviour.
            let acc = l_shl(ctx, acc, 1);
            exc[base + i] = round(ctx, acc);
        }

        let total: [Word16; L_SUBFR] = exc[base..base + L_SUBFR]
            .try_into()
            .expect("one subframe of excitation");
        let mut synth = [Word16(0); L_SUBFR];
        let mut mem = self.mem_syn;
        syn_filt(ctx, aq, &total, &mut synth, &mut mem, true);
        self.mem_syn = mem;

        self.trc1(sf, "gain_pit", i32::from(gain_pit.0));
        self.trc1(sf, "gain_code", i32::from(gain_code.0));
        self.trc1(sf, "L_gain_code", l_gain_code.0);
        self.trc1(sf, "voice_fac", i32::from(voice_fac.0));
        self.trc(sf, "exc_total", &total);
        self.trc(sf, "synth", &synth);

        if rate.has_high_band() {
            // Read before the borrow of `self.high_band` below.
            let hangover = self.dtx.hangover_count();
            let enhanced = self.high_band.enhanced_excitation(
                ctx,
                &EnhancementInputs {
                    adaptive: &adaptive_contribution,
                    code: &code,
                    l_gain_code,
                    gain_pit,
                    voice_fac,
                    stab_fac,
                    q_new,
                },
            );
            let reference: [Word16; L_SUBFR16K] = pcm[index * L_SUBFR16K..(index + 1) * L_SUBFR16K]
                .try_into()
                .expect("one subframe at 16 kHz");
            let gain_index = self.high_band.analyse(
                ctx,
                aq,
                &enhanced,
                q_new,
                &reference,
                self.vad_hist,
                hangover,
            );
            prms.push(gain_index, 4);
        }

        next_window
    }
}

/// Flatten four per-subframe predictors into the layout the trace uses.
fn flatten(a: &[[Word16; M + 1]; NB_SUBFR]) -> [Word16; NB_SUBFR * (M + 1)] {
    let mut flat = [Word16(0); NB_SUBFR * (M + 1)];
    for (k, row) in a.iter().enumerate() {
        flat[k * (M + 1)..(k + 1) * (M + 1)].copy_from_slice(row);
    }
    flat
}

/// Everything one subframe reads that it does not own.
struct SubframeInputs<'a> {
    rate: Rate,
    mode: PitchMode,
    index: usize,
    window: &'a [Word16; L_TOTAL],
    a: &'a [Word16; M + 1],
    aq: &'a [Word16; M + 1],
    q_new: i16,
    shift: i16,
    stab_fac: Word16,
    lag_window: LagWindow,
    absolute: bool,
    pcm: &'a [Word16; L_FRAME16K],
}

/// Everything the 23.85 kbit/s excitation enhancement reads.
struct EnhancementInputs<'a> {
    adaptive: &'a [Word16; L_SUBFR],
    code: &'a [Word16; L_SUBFR],
    l_gain_code: Word32,
    gain_pit: Word16,
    voice_fac: Word16,
    stab_fac: Word16,
    q_new: i16,
}

// ---------------------------------------------------------------------------
// Pitch index encoding
// ---------------------------------------------------------------------------

/// How many bits this subframe's pitch index occupies.
const fn lag_index_width(absolute: bool, quarter_sample: bool) -> usize {
    match (absolute, quarter_sample) {
        (true, true) => 9,
        (true, false) => 8,
        (false, true) => 6,
        (false, false) => 5,
    }
}

/// Encode the closed-loop lag, `cod_main.c` 902–1011.
///
/// An absolute subframe codes the whole `[PIT_MIN, PIT_MAX]` range at a
/// resolution that coarsens with the lag — quarter, then half, then whole
/// samples — so long lags, where a quarter sample is inaudible, do not consume
/// index space. A relative subframe codes an offset into the sixteen integer
/// lags around the last absolute choice, which is what makes five or six bits
/// enough.
fn encode_lag(
    ctx: &mut DspContext,
    t0: i16,
    t0_frac: i16,
    t0_min: i16,
    absolute: bool,
    quarter_sample: bool,
) -> u16 {
    // `T0_frac >> 1` is the half-sample fraction; the shift floors, which is
    // what maps fractions 0 and 1 onto index step 0.
    let half_frac = shr(ctx, Word16(t0_frac), 1);
    let index = if quarter_sample {
        if absolute {
            if t0 < PIT_FR2 {
                let quarters = shl(ctx, Word16(t0), 2);
                let with_frac = add(ctx, quarters, Word16(t0_frac));
                sub(ctx, with_frac, Word16(PIT_MIN * 4))
            } else if t0 < PIT_FR1_9B {
                let halves = shl(ctx, Word16(t0), 1);
                let with_frac = add(ctx, halves, half_frac);
                let rebased = sub(ctx, with_frac, Word16(PIT_FR2 * 2));
                add(ctx, rebased, Word16((PIT_FR2 - PIT_MIN) * 4))
            } else {
                let coarse = sub(ctx, Word16(t0), Word16(PIT_FR1_9B));
                let offset = add(ctx, coarse, Word16((PIT_FR2 - PIT_MIN) * 4));
                add(ctx, offset, Word16((PIT_FR1_9B - PIT_FR2) * 2))
            }
        } else {
            let i = sub(ctx, Word16(t0), Word16(t0_min));
            let quarters = shl(ctx, i, 2);
            add(ctx, quarters, Word16(t0_frac))
        }
    } else if absolute {
        if t0 < PIT_FR1_8B {
            let halves = shl(ctx, Word16(t0), 1);
            let with_frac = add(ctx, halves, half_frac);
            sub(ctx, with_frac, Word16(PIT_MIN * 2))
        } else {
            let coarse = sub(ctx, Word16(t0), Word16(PIT_FR1_8B));
            add(ctx, coarse, Word16((PIT_FR1_8B - PIT_MIN) * 2))
        }
    } else {
        let i = sub(ctx, Word16(t0), Word16(t0_min));
        let halves = shl(ctx, i, 1);
        add(ctx, halves, half_frac)
    };
    index.0 as u16
}

// ---------------------------------------------------------------------------
// Filters `coder()` uses directly
// ---------------------------------------------------------------------------

/// `Syn_filt`: the 16-bit synthesis filter, `1/A(z)`.
///
/// `memory` supplies `M` samples of history and is written back only when
/// `update` is set — the target computation runs it with `update` clear, from a
/// memory it prepared itself, and must not disturb the encoder's.
fn syn_filt(
    ctx: &mut DspContext,
    a: &[Word16; M + 1],
    input: &[Word16],
    output: &mut [Word16],
    memory: &mut [Word16; M],
    update: bool,
) {
    let mut buffer = [Word16(0); M + L_SUBFR];
    buffer[..M].copy_from_slice(memory);

    let headroom = norm_s(a[0]) - 2;
    // The input is halved rather than the coefficients scaled down, so one
    // shift puts the result back where it belongs.
    let a0 = shr(ctx, a[0], 1);

    for (i, &sample) in input.iter().enumerate() {
        let mut acc = l_mult(ctx, sample, a0);
        for j in 1..=M {
            acc = l_msu(ctx, acc, a[j], buffer[M + i - j]);
        }
        let acc = l_shl(ctx, acc, 3 + headroom);
        let filtered = round(ctx, acc);
        buffer[M + i] = filtered;
        output[i] = filtered;
    }

    if update {
        let n = input.len();
        memory.copy_from_slice(&buffer[n..n + M]);
    }
}

/// `Preemph`: filtering through `1 - μz⁻¹`, in place.
///
/// Descending, because the filter is in place and reads `x[i-1]`: the order is
/// what keeps that neighbour unfiltered.
fn preemph(ctx: &mut DspContext, x: &mut [Word16], mu: Word16, memory: &mut Word16) {
    let last = x[x.len() - 1];
    for i in (1..x.len()).rev() {
        let acc = l_deposit_h(x[i]);
        let acc = l_msu(ctx, acc, x[i - 1], mu);
        x[i] = round(ctx, acc);
    }
    let acc = l_deposit_h(x[0]);
    let acc = l_msu(ctx, acc, *memory, mu);
    x[0] = round(ctx, acc);
    *memory = last;
}

/// `Preemph2`: as [`preemph`], with the output doubled.
fn preemph2(ctx: &mut DspContext, x: &mut [Word16], mu: Word16, memory: &mut Word16) {
    let last = x[x.len() - 1];
    for i in (1..x.len()).rev() {
        let acc = l_deposit_h(x[i]);
        let acc = l_msu(ctx, acc, x[i - 1], mu);
        let acc = l_shl(ctx, acc, 1);
        x[i] = round(ctx, acc);
    }
    let acc = l_deposit_h(x[0]);
    let acc = l_msu(ctx, acc, *memory, mu);
    let acc = l_shl(ctx, acc, 1);
    x[0] = round(ctx, acc);
    *memory = last;
}

/// `voice_factor`: −1 for a wholly unvoiced subframe, +1 for a wholly voiced
/// one, Q15.
///
/// The ratio of the two excitation energies once both are on a common exponent,
/// which is why every intermediate here carries its own.
fn voice_factor(
    ctx: &mut DspContext,
    excitation: &[Word16; L_SUBFR],
    q_exc: i16,
    gain_pit: Word16,
    code: &[Word16; L_SUBFR],
    gain_code: Word16,
) -> Word16 {
    let (energy, exp1) = dot_product12(ctx, excitation, excitation);
    let mut ener1 = extract_h(energy);
    let mut exp1 = exp1 - 2 * q_exc;
    let product = l_mult(ctx, gain_pit, gain_pit);
    let exp = norm_l(product);
    let tmp = extract_h(l_shl(ctx, product, exp));
    ener1 = mult(ctx, ener1, tmp);
    // 10 brings the Q14 pitch gain down to the codeword's Q9.
    exp1 = exp1 - exp - 10;

    let (energy, exp2) = dot_product12(ctx, code, code);
    let mut ener2 = extract_h(energy);
    let exp = norm_s(gain_code);
    let tmp = shl(ctx, gain_code, exp);
    let tmp = mult(ctx, tmp, tmp);
    ener2 = mult(ctx, ener2, tmp);
    let exp2 = exp2 - 2 * exp;

    let i = exp1 - exp2;
    if i >= 0 {
        ener1 = shr(ctx, ener1, 1);
        ener2 = shr(ctx, ener2, i + 1);
    } else {
        ener1 = shr(ctx, ener1, 1 - i);
        ener2 = shr(ctx, ener2, 1);
    }

    let difference = sub(ctx, ener1, ener2);
    let sum = add(ctx, ener1, ener2);
    // The `+1` keeps the denominator non-zero for a wholly silent subframe.
    let total = add(ctx, sum, Word16(1));
    if difference.0 >= 0 {
        div_s(difference, total)
    } else {
        let magnitude = negate(ctx, difference);
        negate(ctx, div_s(magnitude, total))
    }
}

// ---------------------------------------------------------------------------
// The open-loop pitch gain, for the tone detector
// ---------------------------------------------------------------------------

/// Decimated weighted-speech history the open-loop search reaches over.
const WSP_HISTORY: usize = 115;
/// Decimated weighted speech per frame.
const WSP_FRAME: usize = 128;

/// The `ol_gain` half of `Pitch_med_ol`, measured a second time.
///
/// [`OpenLoopPitch`] computes exactly this and keeps it private; the tone
/// detector needs it, and this module may not change that stage. Running the
/// same filter over the same signal gives the same numbers, so the duplicate is
/// exact — but it *is* a duplicate, and it should be deleted the moment
/// `OpenLoopLags` carries the two gains.
#[derive(Clone, Debug)]
struct OpenLoopGainProbe {
    high_pass: WeightedSpeechHighPass,
    high_passed: [Word16; WSP_HISTORY + WSP_FRAME],
    history_shift: i16,
}

impl OpenLoopGainProbe {
    fn new() -> Self {
        Self {
            high_pass: WeightedSpeechHighPass::default(),
            high_passed: [Word16(0); WSP_HISTORY + WSP_FRAME],
            history_shift: 0,
        }
    }

    /// Follow the frame's change of scaling, as `Pitch_med_ol`'s caller does.
    fn rescale(&mut self, ctx: &mut DspContext, q_exp: i16, shift: i16) {
        let regained = sub(ctx, Word16(shift), Word16(self.history_shift));
        let exp = add(ctx, Word16(q_exp), regained).0;
        self.history_shift = shift;
        scale_sig(ctx, &mut self.high_passed[..WSP_HISTORY], exp);
        self.high_pass.rescale(ctx, exp);
    }

    /// The normalised correlation of the high-passed weighted speech at `lag`,
    /// Q15, advancing the filter over `input`.
    fn measure(&mut self, ctx: &mut DspContext, input: &[Word16], lag: i16) -> Word16 {
        let span = input.len();
        let lag = usize::try_from(lag).expect("an open-loop lag is positive");
        self.high_pass
            .filter(ctx, input, &mut self.high_passed[WSP_HISTORY..]);

        // The energies start at 1 so that silence still normalises.
        let mut cross = Word32(0);
        let mut lagged_energy = Word32(1);
        let mut energy = Word32(1);
        for j in 0..span {
            let here = self.high_passed[WSP_HISTORY + j];
            let back = self.high_passed[WSP_HISTORY + j - lag];
            cross = l_mac(ctx, cross, here, back);
            lagged_energy = l_mac(ctx, lagged_energy, back, back);
            energy = l_mac(ctx, energy, here, here);
        }

        let cross_exp = norm_l(cross);
        let cross = l_shl(ctx, cross, cross_exp);
        let lagged_exp = norm_l(lagged_energy);
        let lagged_energy = l_shl(ctx, lagged_energy, lagged_exp);
        let energy_exp = norm_l(energy);
        let energy = l_shl(ctx, energy, energy_exp);

        let lagged_rounded = round(ctx, lagged_energy);
        let energy_rounded = round(ctx, energy);
        let mut product = l_mult(ctx, lagged_rounded, energy_rounded);
        let renorm = norm_l(product);
        product = l_shl(ctx, product, renorm);
        let mut exp = add(ctx, Word16(lagged_exp), Word16(energy_exp));
        exp = add(ctx, exp, Word16(renorm));
        exp = sub(ctx, Word16(62), exp);

        // `Isqrt_n` rewrites its exponent; what follows uses the one it wrote.
        let (inverse_root, exp) = isqrt_n(ctx, (product, exp.0));

        let cross_rounded = round(ctx, cross);
        let root_rounded = round(ctx, inverse_root);
        let scaled = l_mult(ctx, cross_rounded, root_rounded);
        let headroom = sub(ctx, Word16(31), Word16(cross_exp));
        let total = add(ctx, headroom, Word16(exp)).0;
        let shifted = l_shl(ctx, scaled, total);
        let gain = round(ctx, shifted);

        // Overlapping forward copy: the ranges intersect when the span is half
        // a frame, so this must behave as a memmove.
        self.high_passed.copy_within(span..span + WSP_HISTORY, 0);
        gain
    }
}

// ---------------------------------------------------------------------------
// Voice activity detection
// ---------------------------------------------------------------------------

/// The voice activity detector of TS 26.173 `wb_vad.c`.
///
/// One bit per frame reaches the bitstream, and with discontinuous transmission
/// off that bit is *all* the detector contributes to the payload — but it is a
/// transmitted bit like any other, so a wrong decision costs byte-exactness on
/// the frame it is wrong on.
///
/// # Why it lives here and not in a stage module
///
/// It shares nothing with the ACELP path: a different signal (the frame in Q1
/// rather than `Q_new`), a different decomposition (a twelve-band tree of
/// all-pass pairs, not linear prediction) and a different notion of time (a
/// fifteen-frame decision register). The one thing it takes from the rest of
/// the encoder is the open-loop pitch gain, twice a frame, through
/// [`VoiceActivityDetector::tone_detection`].
mod vad {
    use super::{
        add, div_s, extract_h, l_mac, l_mult, l_shl, mult, norm_l, norm_s, shl, shr, sub,
        DspContext, Word16, Word32,
    };
    use crate::fixed_point::arith::{abs_s, mult_r};
    use crate::fixed_point::arith32::{l_add, l_sub};

    /// Samples the detector sees per frame, at 12.8 kHz.
    const FRAME_LEN: usize = 256;
    /// Sub-bands the filter bank produces.
    const COMPLEN: usize = 12;
    /// `log2(32767 / 256)`: the shift that brings a band level onto the SNR's
    /// own scale.
    const UNIRSHFT: i16 = 7;

    /// Open-loop pitch gain above which a half-frame counts as tonal, 0.65 Q15.
    const TONE_THR: i16 = 21298;

    const SP_EST_COUNT: i16 = 80;
    const SP_ACTIVITY_COUNT: i16 = 25;
    const ALPHA_SP_UP: i16 = 4915;
    const ALPHA_SP_DOWN: i16 = 4915;
    const SPEECH_LEVEL_INIT: i16 = 2050;
    const MIN_SPEECH_LEVEL1: i16 = 129;
    const MIN_SPEECH_LEVEL2: i16 = 410;
    /// 0 dB in Q12: the SNR below which the speech-level estimate is taken to
    /// be contaminated by the noise estimate.
    const MIN_SPEECH_SNR: i16 = 4096;

    const ALPHA_UP1: i16 = 1638;
    const ALPHA_DOWN1: i16 = 2097;
    const ALPHA_UP2: i16 = 491;
    const ALPHA_DOWN2: i16 = 1867;
    const ALPHA3: i16 = 1638;
    const ALPHA4: i16 = 3276;
    const ALPHA5: i16 = 16383;

    const THR_MIN: i16 = 204;
    const THR_HIGH: i16 = 768;

    // The break-point constants below are literal integers in `wb_vad_c.h`.
    // Three of the four do *not* equal `ilog2()` of the value their comment
    // names — `NO_P1` is 31744 where `ilog2(1)` is 31743, and `SP_P2`, which
    // survives only inside `SP_SLOPE`, is 17832 where `ilog2(8200)` is 18430 —
    // so recomputing them at run time would move the whole threshold curve.
    // They are transcribed, not derived.
    const NO_P1: i16 = 31744;
    const NO_SLOPE: i16 = 1509;
    const SP_CH_MIN: i16 = -96;
    const SP_CH_MAX: i16 = 96;
    const SP_P1: i16 = 22527;
    const SP_SLOPE: i16 = -1339;

    const HANG_HIGH: i16 = 12;
    const HANG_LOW: i16 = 2;
    const HANG_P1: i16 = 217;
    const HANG_SLOPE: i16 = -1110;
    const BURST_HIGH: i16 = 8;
    const BURST_P1: i16 = 768;
    const BURST_SLOPE: i16 = 297;

    const STAT_COUNT: i16 = 20;
    const STAT_THR_LEVEL: i16 = 184;
    const STAT_THR: i16 = 1000;

    const NOISE_MIN: i16 = 40;
    const NOISE_MAX: i16 = 20000;
    const NOISE_INIT: i16 = 150;

    const VAD_POW_LOW: i32 = 30000;
    const POW_TONE_THR: i32 = 686_080;

    const COEFF3: i16 = 13363;
    const COEFF5_1: i16 = 21955;
    const COEFF5_2: i16 = 6390;

    /// Detector state, `VadVars`.
    ///
    /// `stat_count` is the one field `wb_vad_reset` does not touch: the
    /// reference leaves whatever `malloc` returned. That is safe only because
    /// `vadreg` is zero on the first frame, which forces `update_stationarity`
    /// to assign it before anything reads it. Zero here, for the same reason it
    /// does not matter there.
    #[derive(Clone, Debug)]
    pub struct VoiceActivityDetector {
        /// Background noise estimate per band.
        bckr_est: [Word16; COMPLEN],
        /// Long-term average level per band, for the stationarity test.
        ave_level: [Word16; COMPLEN],
        /// The previous frame's band levels — what the noise estimate tracks.
        old_level: [Word16; COMPLEN],
        /// Level of the tail of the frame, carried into the next one.
        sub_level: [Word16; COMPLEN],
        /// Fifth-order all-pass memories of the filter bank.
        a_data5: [[Word16; 2]; 5],
        /// Third-order all-pass memories of the filter bank.
        a_data3: [Word16; 6],
        burst_count: i16,
        hang_count: i16,
        stat_count: i16,
        /// Fifteen intermediate decisions, newest in bit 14.
        vadreg: i16,
        /// Fifteen tone flags, newest in bit 14, two written per frame.
        tone_flag: i16,
        sp_est_cnt: i16,
        sp_max: Word16,
        sp_max_cnt: i16,
        speech_level: Word16,
        prev_pow_sum: Word32,
    }

    impl Default for VoiceActivityDetector {
        fn default() -> Self {
            Self::new()
        }
    }

    impl VoiceActivityDetector {
        /// The reset state, `wb_vad_reset`.
        #[must_use]
        pub const fn new() -> Self {
            Self {
                bckr_est: [Word16(NOISE_INIT); COMPLEN],
                ave_level: [Word16(NOISE_INIT); COMPLEN],
                old_level: [Word16(NOISE_INIT); COMPLEN],
                sub_level: [Word16(0); COMPLEN],
                a_data5: [[Word16(0); 2]; 5],
                a_data3: [Word16(0); 6],
                burst_count: 0,
                hang_count: 0,
                stat_count: 0,
                vadreg: 0,
                tone_flag: 0,
                sp_est_cnt: 0,
                sp_max: Word16(0),
                sp_max_cnt: 0,
                speech_level: Word16(SPEECH_LEVEL_INIT),
                prev_pow_sum: Word32(0),
            }
        }

        /// Fold one half-frame's open-loop pitch gain into the tone history.
        ///
        /// A strongly periodic half is evidence of a signalling tone rather
        /// than of speech; five consecutive tonal halves freeze the noise
        /// estimate through `stat_count`.
        pub fn tone_detection(&mut self, ctx: &mut DspContext, pitch_gain: Word16) {
            self.tone_flag = shr(ctx, Word16(self.tone_flag), 1).0;
            if sub(ctx, pitch_gain, Word16(TONE_THR)).0 > 0 {
                self.tone_flag |= 0x4000;
            }
        }

        /// Decide whether one frame carries speech (`wb_vad`).
        ///
        /// `frame` is the pre-emphasised 12.8 kHz frame brought to Q1.
        pub fn process(&mut self, ctx: &mut DspContext, frame: &[Word16; FRAME_LEN]) -> bool {
            let mut power = Word32(0);
            for &sample in frame {
                power = l_mac(ctx, power, sample, sample);
            }
            // Two frames of power, so no decision rests on one frame's worth of
            // evidence alone.
            let pow_sum = l_add(ctx, power, self.prev_pow_sum);
            self.prev_pow_sum = power;

            if l_sub(ctx, pow_sum, Word32(POW_TONE_THR)).0 < 0 {
                self.tone_flag &= 0x1fff;
            }

            let level = self.filter_bank(ctx, frame);
            let flag = self.decide(ctx, &level, pow_sum);

            // The lowest band is excluded here as everywhere: it is where hum
            // and rumble live, and they are not speech.
            let mut total = Word32(0);
            for &l in &level[1..] {
                total = l_add(ctx, total, Word32(i32::from(l.0)));
            }
            let input_level = extract_h(l_shl(ctx, total, 12));
            self.estimate_speech(ctx, input_level);

            flag
        }

        /// Split the frame into twelve bands and measure each one's level.
        ///
        /// A tree of half-band all-pass pairs: each stage splits what it is
        /// given into a low and a high half at half the rate, so twelve bands
        /// of unequal width fall out of five stages without a transform.
        fn filter_bank(
            &mut self,
            ctx: &mut DspContext,
            input: &[Word16; FRAME_LEN],
        ) -> [Word16; COMPLEN] {
            let mut buf = [Word16(0); FRAME_LEN];
            for (slot, &sample) in buf.iter_mut().zip(input.iter()) {
                *slot = shr(ctx, sample, 1);
            }

            for i in 0..FRAME_LEN / 2 {
                filter5(ctx, &mut buf, 2 * i, 2 * i + 1, 0, &mut self.a_data5);
            }
            for i in 0..FRAME_LEN / 4 {
                filter5(ctx, &mut buf, 4 * i, 4 * i + 2, 1, &mut self.a_data5);
                filter5(ctx, &mut buf, 4 * i + 1, 4 * i + 3, 2, &mut self.a_data5);
            }
            for i in 0..FRAME_LEN / 8 {
                filter5(ctx, &mut buf, 8 * i, 8 * i + 4, 3, &mut self.a_data5);
                filter5(ctx, &mut buf, 8 * i + 2, 8 * i + 6, 4, &mut self.a_data5);
                filter3(ctx, &mut buf, 8 * i + 3, 8 * i + 7, 0, &mut self.a_data3);
            }
            for i in 0..FRAME_LEN / 16 {
                filter3(ctx, &mut buf, 16 * i, 16 * i + 8, 1, &mut self.a_data3);
                filter3(ctx, &mut buf, 16 * i + 4, 16 * i + 12, 2, &mut self.a_data3);
                filter3(ctx, &mut buf, 16 * i + 6, 16 * i + 14, 3, &mut self.a_data3);
            }
            for i in 0..FRAME_LEN / 32 {
                filter3(ctx, &mut buf, 32 * i, 32 * i + 16, 4, &mut self.a_data3);
                filter3(ctx, &mut buf, 32 * i + 8, 32 * i + 24, 5, &mut self.a_data3);
            }

            let mut level = [Word16(0); COMPLEN];
            for &(band, span) in &BANDS {
                level[band] = level_calculation(ctx, &buf, &mut self.sub_level[band], span);
            }
            level
        }

        /// The decision itself: SNR against an adaptive threshold, then
        /// hangover.
        fn decide(
            &mut self,
            ctx: &mut DspContext,
            level: &[Word16; COMPLEN],
            pow_sum: Word32,
        ) -> bool {
            // Sum of squared per-band signal-to-noise ratios.
            let mut snr_sum = Word32(0);
            for (&band, &noise) in level.iter().zip(self.bckr_est.iter()) {
                let exp = norm_s(noise);
                let denominator = shl(ctx, noise, exp);
                let halved = shr(ctx, band, 1);
                let ratio = div_s(halved, denominator);
                let ratio = shl(ctx, ratio, exp - (UNIRSHFT - 1));
                snr_sum = l_mac(ctx, snr_sum, ratio, ratio);
            }

            // Average noise level, lowest band excluded.
            let mut total = Word32(0);
            for &noise in &self.bckr_est[1..] {
                total = l_add(ctx, total, Word32(i32::from(noise.0)));
            }
            let noise_level = extract_h(l_shl(ctx, total, 12));

            // A speech level below the noise floor times the minimum SNR is not
            // believable, so it is lifted back onto it.
            let scaled_noise = mult(ctx, noise_level, Word16(MIN_SPEECH_SNR));
            let floor = shl(ctx, scaled_noise, 3);
            if sub(ctx, self.speech_level, floor).0 < 0 {
                self.speech_level = floor;
            }
            let noise_log = ilog2(ctx, noise_level);
            // Subtracting the floor is what stops a noise-dominated speech
            // level from raising the threshold twice over.
            let above_floor = sub(ctx, self.speech_level, floor);
            let speech_log = ilog2(ctx, above_floor);

            let noise_offset = sub(ctx, noise_log, Word16(NO_P1));
            let noise_term = mult(ctx, Word16(NO_SLOPE), noise_offset);
            let from_noise = add(ctx, noise_term, Word16(THR_HIGH));

            let speech_offset = sub(ctx, speech_log, Word16(SP_P1));
            let speech_term = mult(ctx, Word16(SP_SLOPE), speech_offset);
            let from_speech = add(ctx, Word16(SP_CH_MIN), speech_term);
            let from_speech = Word16(from_speech.0.clamp(SP_CH_MIN, SP_CH_MAX));
            let mut vad_thr = add(ctx, from_noise, from_speech);
            if sub(ctx, vad_thr, Word16(THR_MIN)).0 < 0 {
                vad_thr = Word16(THR_MIN);
            }

            self.vadreg = shr(ctx, Word16(self.vadreg), 1).0;
            let scaled = l_mult(ctx, vad_thr, Word16(512 * COMPLEN as i16));
            if l_sub(ctx, snr_sum, scaled).0 > 0 {
                self.vadreg |= 0x4000;
            }
            let low_power = l_sub(ctx, pow_sum, Word32(VAD_POW_LOW)).0 < 0;

            self.update_noise_estimate(ctx, level);

            // A quieter frame needs a longer hangover and a shorter burst to
            // earn one, so both are read off the same threshold.
            let hang_offset = sub(ctx, vad_thr, Word16(HANG_P1));
            let hang_term = mult(ctx, Word16(HANG_SLOPE), hang_offset);
            let mut hang_len = add(ctx, hang_term, Word16(HANG_HIGH));
            if sub(ctx, hang_len, Word16(HANG_LOW)).0 < 0 {
                hang_len = Word16(HANG_LOW);
            }
            let burst_offset = sub(ctx, vad_thr, Word16(BURST_P1));
            let burst_term = mult(ctx, Word16(BURST_SLOPE), burst_offset);
            let burst_len = add(ctx, burst_term, Word16(BURST_HIGH));

            self.hangover(ctx, low_power, hang_len.0, burst_len.0)
        }

        /// Extend a decision past the end of a burst, so a trailing fricative
        /// is not cut off.
        fn hangover(
            &mut self,
            ctx: &mut DspContext,
            low_power: bool,
            hang_len: i16,
            burst_len: i16,
        ) -> bool {
            if low_power {
                self.burst_count = 0;
                self.hang_count = 0;
                return false;
            }
            if (self.vadreg & 0x4000) != 0 {
                self.burst_count = add(ctx, Word16(self.burst_count), Word16(1)).0;
                if sub(ctx, Word16(self.burst_count), Word16(burst_len)).0 >= 0 {
                    self.hang_count = hang_len;
                }
                return true;
            }
            self.burst_count = 0;
            if self.hang_count > 0 {
                self.hang_count = sub(ctx, Word16(self.hang_count), Word16(1)).0;
                return true;
            }
            false
        }

        /// Track the background noise, at a speed the stationarity test picks.
        fn update_noise_estimate(&mut self, ctx: &mut DspContext, level: &[Word16; COMPLEN]) {
            self.update_stationarity(ctx, level);

            // A small additive term, because at very low levels the
            // multiplicative update rounds to no change at all.
            let mut bckr_add = Word16(2);
            let (alpha_up, alpha_down) = if (self.vadreg & 0x7800) == 0 {
                (Word16(ALPHA_UP1), Word16(ALPHA_DOWN1))
            } else if self.stat_count == 0 {
                (Word16(ALPHA_UP2), Word16(ALPHA_DOWN2))
            } else {
                bckr_add = Word16(0);
                (Word16(0), Word16(ALPHA3))
            };

            for i in 0..COMPLEN {
                let delta = sub(ctx, self.old_level[i], self.bckr_est[i]);
                self.bckr_est[i] = if delta.0 < 0 {
                    // Downwards with a bias, so an estimate that was inflated
                    // by speech recovers rather than sticking.
                    let step = mult_r(ctx, alpha_down, delta);
                    let moved = add(ctx, self.bckr_est[i], step);
                    let v = add(ctx, Word16(-2), moved);
                    if sub(ctx, v, Word16(NOISE_MIN)).0 < 0 {
                        Word16(NOISE_MIN)
                    } else {
                        v
                    }
                } else {
                    let step = mult_r(ctx, alpha_up, delta);
                    let moved = add(ctx, self.bckr_est[i], step);
                    let v = add(ctx, bckr_add, moved);
                    if sub(ctx, v, Word16(NOISE_MAX)).0 > 0 {
                        Word16(NOISE_MAX)
                    } else {
                        v
                    }
                };
            }

            self.old_level.copy_from_slice(level);
        }

        /// `update_cntrl`: how stationary the spectrum is, which gates the
        /// noise estimate's update speed.
        fn update_stationarity(&mut self, ctx: &mut DspContext, level: &[Word16; COMPLEN]) {
            if (self.tone_flag & 0x7c00) == 0x7c00 {
                // A sustained tone must never be learned as background noise.
                self.stat_count = STAT_COUNT;
            } else if (self.vadreg & 0x7f80) == 0 {
                self.stat_count = STAT_COUNT;
            } else {
                let mut stat_rat = Word16(0);
                for (&band, &average) in level.iter().zip(self.ave_level.iter()) {
                    // Always the larger over the smaller, so the ratio measures
                    // change in either direction.
                    let (mut num, mut denom) = if sub(ctx, band, average).0 > 0 {
                        (band, average)
                    } else {
                        (average, band)
                    };
                    if sub(ctx, num, Word16(STAT_THR_LEVEL)).0 < 0 {
                        num = Word16(STAT_THR_LEVEL);
                    }
                    if sub(ctx, denom, Word16(STAT_THR_LEVEL)).0 < 0 {
                        denom = Word16(STAT_THR_LEVEL);
                    }
                    let exp = norm_s(denom);
                    let denom = shl(ctx, denom, exp);
                    let halved = shr(ctx, num, 1);
                    let ratio = div_s(halved, denom);
                    let contribution = shr(ctx, ratio, 8 - exp);
                    stat_rat = add(ctx, stat_rat, contribution);
                }

                if sub(ctx, stat_rat, Word16(STAT_THR)).0 > 0 {
                    self.stat_count = STAT_COUNT;
                } else if (self.vadreg & 0x4000) != 0 && self.stat_count != 0 {
                    self.stat_count = sub(ctx, Word16(self.stat_count), Word16(1)).0;
                }
            }

            // A frame just declared stationary replaces the average outright
            // rather than smoothing into it.
            let alpha = if self.stat_count == STAT_COUNT {
                Word16(32767)
            } else if (self.vadreg & 0x4000) == 0 {
                Word16(ALPHA5)
            } else {
                Word16(ALPHA4)
            };
            for (average, &band) in self.ave_level.iter_mut().zip(level.iter()) {
                let delta = sub(ctx, band, *average);
                let step = mult_r(ctx, alpha, delta);
                *average = add(ctx, *average, step);
            }
        }

        /// Track the level of speech, from the peak of recent active frames.
        fn estimate_speech(&mut self, ctx: &mut DspContext, in_level: Word16) {
            // If the required number of active frames has not turned up inside
            // the observation window, start the window again — otherwise a
            // noisy channel's occasional active frame would drag the estimate.
            let elapsed = sub(ctx, Word16(self.sp_est_cnt), Word16(self.sp_max_cnt));
            if sub(ctx, elapsed, Word16(SP_EST_COUNT - SP_ACTIVITY_COUNT)).0 > 0 {
                self.sp_est_cnt = 0;
                self.sp_max = Word16(0);
                self.sp_max_cnt = 0;
            }
            self.sp_est_cnt = add(ctx, Word16(self.sp_est_cnt), Word16(1)).0;

            let active = (self.vadreg & 0x4000) != 0 || sub(ctx, in_level, self.speech_level).0 > 0;
            if !active || sub(ctx, in_level, Word16(MIN_SPEECH_LEVEL1)).0 <= 0 {
                return;
            }

            if sub(ctx, in_level, self.sp_max).0 > 0 {
                self.sp_max = in_level;
            }
            self.sp_max_cnt = add(ctx, Word16(self.sp_max_cnt), Word16(1)).0;
            if sub(ctx, Word16(self.sp_max_cnt), Word16(SP_ACTIVITY_COUNT)).0 < 0 {
                return;
            }

            // Half the peak, as a stand-in for the average level of speech.
            let target = shr(ctx, self.sp_max, 1);
            let alpha = if sub(ctx, target, self.speech_level).0 > 0 {
                Word16(ALPHA_SP_UP)
            } else {
                Word16(ALPHA_SP_DOWN)
            };
            if sub(ctx, target, Word16(MIN_SPEECH_LEVEL2)).0 > 0 {
                let delta = sub(ctx, target, self.speech_level);
                let step = mult_r(ctx, alpha, delta);
                self.speech_level = add(ctx, self.speech_level, step);
            }
            self.sp_max = Word16(0);
            self.sp_max_cnt = 0;
            self.sp_est_cnt = 0;
        }
    }

    /// One fifth-order half-band all-pass pair, in place.
    ///
    /// `lo` becomes the low half and `hi` the high half, both at half the input
    /// rate — the decimation is implicit in the caller's stride. The memory is
    /// passed as the whole array plus an index because two of the five
    /// instances are driven from one loop.
    fn filter5(
        ctx: &mut DspContext,
        x: &mut [Word16],
        lo: usize,
        hi: usize,
        slot: usize,
        data: &mut [[Word16; 2]; 5],
    ) {
        let feedback = mult(ctx, Word16(COEFF5_1), data[slot][0]);
        let t0 = sub(ctx, x[lo], feedback);
        let forward = mult(ctx, Word16(COEFF5_1), t0);
        let t1 = add(ctx, data[slot][0], forward);
        data[slot][0] = t0;

        let feedback = mult(ctx, Word16(COEFF5_2), data[slot][1]);
        let t0 = sub(ctx, x[hi], feedback);
        let forward = mult(ctx, Word16(COEFF5_2), t0);
        let t2 = add(ctx, data[slot][1], forward);
        data[slot][1] = t0;

        // Sum and difference are formed in 32 bits and shifted back, which is
        // where the bit a 16-bit add would have lost to saturation goes.
        let sum = l_add(ctx, Word32(i32::from(t1.0)), Word32(i32::from(t2.0)));
        let difference = l_sub(ctx, Word32(i32::from(t1.0)), Word32(i32::from(t2.0)));
        x[lo] = extract_h(l_shl(ctx, sum, 15));
        x[hi] = extract_h(l_shl(ctx, difference, 15));
    }

    /// One third-order half-band all-pass pair, in place.
    fn filter3(
        ctx: &mut DspContext,
        x: &mut [Word16],
        lo: usize,
        hi: usize,
        slot: usize,
        data: &mut [Word16; 6],
    ) {
        let feedback = mult(ctx, Word16(COEFF3), data[slot]);
        let t1 = sub(ctx, x[hi], feedback);
        let forward = mult(ctx, Word16(COEFF3), t1);
        let t2 = add(ctx, data[slot], forward);
        data[slot] = t1;

        let difference = l_sub(ctx, Word32(i32::from(x[lo].0)), Word32(i32::from(t2.0)));
        let sum = l_add(ctx, Word32(i32::from(x[lo].0)), Word32(i32::from(t2.0)));
        x[hi] = extract_h(l_shl(ctx, difference, 15));
        x[lo] = extract_h(l_shl(ctx, sum, 15));
    }

    /// Where one band's samples sit in the filter bank's output buffer.
    #[derive(Clone, Copy)]
    struct LevelSpan {
        /// Samples of this frame that belong to the *previous* frame's window.
        count1: usize,
        /// Total samples the band contributes.
        count2: usize,
        /// Distance between consecutive samples of the band.
        stride: usize,
        /// Index of the band's first sample.
        offset: usize,
        /// Left shift that brings the summed magnitudes into Q0.
        scale: i16,
    }

    /// Which samples of the filter bank's output each band occupies, highest
    /// band first — exactly the order `filter_bank` writes them in, since each
    /// call also advances that band's carried tail.
    const BANDS: [(usize, LevelSpan); COMPLEN] = {
        const fn span(
            count1: usize,
            count2: usize,
            stride: usize,
            offset: usize,
            scale: i16,
        ) -> LevelSpan {
            LevelSpan {
                count1,
                count2,
                stride,
                offset,
                scale,
            }
        }
        [
            // 4800–6400 Hz
            (11, span(FRAME_LEN / 4 - 48, FRAME_LEN / 4, 4, 1, 14)),
            // 4000–4800, 3200–4000, 2400–3200 Hz
            (10, span(FRAME_LEN / 8 - 24, FRAME_LEN / 8, 8, 7, 15)),
            (9, span(FRAME_LEN / 8 - 24, FRAME_LEN / 8, 8, 3, 15)),
            (8, span(FRAME_LEN / 8 - 24, FRAME_LEN / 8, 8, 2, 15)),
            // 2000–2400, 1600–2000, 1200–1600, 800–1200 Hz
            (7, span(FRAME_LEN / 16 - 12, FRAME_LEN / 16, 16, 14, 16)),
            (6, span(FRAME_LEN / 16 - 12, FRAME_LEN / 16, 16, 6, 16)),
            (5, span(FRAME_LEN / 16 - 12, FRAME_LEN / 16, 16, 4, 16)),
            (4, span(FRAME_LEN / 16 - 12, FRAME_LEN / 16, 16, 12, 16)),
            // 600–800, 400–600, 200–400, 0–200 Hz
            (3, span(FRAME_LEN / 32 - 6, FRAME_LEN / 32, 32, 8, 17)),
            (2, span(FRAME_LEN / 32 - 6, FRAME_LEN / 32, 32, 24, 17)),
            (1, span(FRAME_LEN / 32 - 6, FRAME_LEN / 32, 32, 16, 17)),
            (0, span(FRAME_LEN / 32 - 6, FRAME_LEN / 32, 32, 0, 17)),
        ]
    };

    /// One band's level: the sum of absolute values, plus what the previous
    /// frame's tail left over.
    ///
    /// The tail is measured separately and carried, so every band's level spans
    /// the same amount of *signal* even though the bands are decimated by
    /// different factors and therefore hold different numbers of samples.
    fn level_calculation(
        ctx: &mut DspContext,
        data: &[Word16],
        sub_level: &mut Word16,
        span: LevelSpan,
    ) -> Word16 {
        let LevelSpan {
            count1,
            count2,
            stride,
            offset,
            scale,
        } = span;

        let mut tail = Word32(0);
        for i in count1..count2 {
            let magnitude = abs_s(ctx, data[stride * i + offset]);
            tail = l_mac(ctx, tail, Word16(1), magnitude);
        }

        let carried = l_shl(ctx, Word32(i32::from(sub_level.0)), 16 - scale);
        let mut total = l_add(ctx, tail, carried);
        *sub_level = extract_h(l_shl(ctx, tail, scale));

        for i in 0..count1 {
            let magnitude = abs_s(ctx, data[stride * i + offset]);
            total = l_mac(ctx, total, Word16(1), magnitude);
        }
        extract_h(l_shl(ctx, total, scale))
    }

    /// A cheap *decreasing* pseudo-logarithm: bigger input, smaller output.
    ///
    /// Not `log2`. It is the reference's own approximation, and the threshold
    /// curve's break points were fitted to *it*, so substituting a real
    /// logarithm moves every threshold in the detector.
    fn ilog2(ctx: &mut DspContext, value: Word16) -> Word16 {
        let mut mant = if value.0 <= 0 { Word16(1) } else { value };
        let ex = norm_s(mant);
        mant = shl(ctx, mant, ex);

        // Three squarings in 16 bits and one more in 32: raising the mantissa
        // to the sixteenth power is what turns a normalisation count into a
        // fractional exponent.
        for _ in 0..3 {
            mant = mult(ctx, mant, mant);
        }
        let squared = l_mult(ctx, mant, mant);
        let ex2 = norm_l(squared);
        let mant = extract_h(l_shl(ctx, squared, ex2));

        let binade = add(ctx, Word16(ex), Word16(16));
        let res = shl(ctx, binade, 10);
        let refinement = shl(ctx, Word16(ex2), 6);
        let res = add(ctx, res, refinement);
        let res = add(ctx, res, Word16(127));
        let fraction = shr(ctx, mant, 8);
        sub(ctx, res, fraction)
    }
}

// ---------------------------------------------------------------------------
// The high band, 23.85 kbit/s only
// ---------------------------------------------------------------------------

/// The 6–7 kHz band the top rate spends four bits per subframe on.
///
/// Nothing is transmitted about the high band except a gain correction: the
/// decoder synthesises the band from noise shaped by the *low* band's own
/// predictor, and this analysis measures how far that synthesis would land from
/// the original and codes the difference. So the encoder has to run the
/// decoder's whole high-band chain — noise, shaping, band-pass — before it has
/// anything to compare against.
mod highband {
    use super::super::super::highband::{
        gain_from_tilt, match_energy, spectral_tilt, transmitted_gain, BandFilter, NoiseGenerator,
        NoiseShaper, TiltFilter,
    };
    use super::super::super::math::{dot_product12, isqrt_n, scale_sig};
    use super::super::super::synthesis::{deemphasis, HighPass50, SynthesisFilter, PREEMPH_FAC};
    use super::{
        add, div_s, extract_h, l_mac, l_mult, l_shl, mult, round, shr, sub, DspContext,
        EnhancementInputs, Word16, Word32, L_SUBFR, L_SUBFR16K, M,
    };
    use crate::fixed_point::arith32::{l_add, l_deposit_h, l_msu, l_sub};
    use crate::fixed_point::oper32::{l_extract, mpy_32_16};

    /// The high band's own filter memories.
    #[derive(Clone, Debug)]
    pub struct HighBand {
        /// `mem_syn_hi` / `mem_syn_lo`: the 32-bit synthesis filter.
        synthesis: SynthesisFilter,
        /// `mem_deemph`.
        deemph_memory: Word16,
        /// `mem_sig_out`: the 50 Hz high-pass on the synthesised low band.
        output_highpass: HighPass50,
        /// `seed2`.
        noise: NoiseGenerator,
        /// `mem_syn_hf`: the noise-shaping filter.
        shaper: NoiseShaper,
        /// `mem_hf`: the band-pass on the synthesised noise.
        band_noise: BandFilter,
        /// `mem_hf2`: the same band-pass on the *original* signal. A second
        /// instance, not a second call — the two signals have independent
        /// histories and sharing one memory couples them.
        band_reference: BandFilter,
        /// `mem_hp400`: the 400 Hz high-pass the tilt is measured through.
        tilt: TiltFilter,
        /// `L_gc_thres`: the slow threshold the noise enhancer moves toward.
        gc_threshold: Word32,
        /// `gain_alpha`: how far to trust the measured gain over the estimated
        /// one.
        gain_alpha: Word16,
    }

    impl HighBand {
        /// The reset state.
        pub const fn new() -> Self {
            Self {
                synthesis: SynthesisFilter::new(),
                deemph_memory: Word16(0),
                output_highpass: HighPass50::new(),
                noise: NoiseGenerator::new(),
                shaper: NoiseShaper::new(),
                band_noise: BandFilter::band_pass(),
                band_reference: BandFilter::band_pass(),
                tilt: TiltFilter::new(),
                gc_threshold: Word32(0),
                gain_alpha: Word16(32767),
            }
        }

        /// Clear `L_gc_thres`, the slow gain threshold.
        ///
        /// The only high-band state a comfort-noise frame's partial reset
        /// touches: every filter memory and the noise seed survive, because
        /// the next speech frame has to continue the same noise.
        #[allow(clippy::missing_const_for_fn)]
        pub fn reset_gain_threshold(&mut self) {
            self.gc_threshold = Word32(0);
        }

        /// The excitation the high band is driven from, `cod_main.c`
        /// 1294–1377.
        ///
        /// Two enhancements the transmitted excitation does not get: the code
        /// gain is pulled toward a slow threshold so that noise energy varies
        /// less between frames, and the codeword is high-pass filtered in
        /// proportion to how voiced the subframe is. Both improve the *sound*
        /// of the synthesised high band and would corrupt the adaptive codebook
        /// if they reached it, which is why this returns a new vector rather
        /// than editing one.
        pub fn enhanced_excitation(
            &mut self,
            ctx: &mut DspContext,
            inputs: &EnhancementInputs<'_>,
        ) -> [Word16; L_SUBFR] {
            let EnhancementInputs {
                adaptive,
                code,
                l_gain_code,
                gain_pit,
                voice_fac,
                stab_fac,
                q_new,
            } = *inputs;

            let (gain_hi, gain_lo) = l_extract(l_gain_code);

            // Unvoiced *and* spectrally stable is what earns the smoothing;
            // either alone leaves `fac` small.
            let half_voicing = shr(ctx, voice_fac, 1);
            let unvoiced = sub(ctx, Word16(16384), half_voicing);
            let fac = mult(ctx, stab_fac, unvoiced);

            // Move the threshold 1.5 dB toward this subframe's gain, from
            // whichever side it is on.
            let threshold = if l_sub(ctx, l_gain_code, self.gc_threshold).0 < 0 {
                let step = mpy_32_16(gain_hi, gain_lo, Word16(6226));
                let raised = l_add(ctx, l_gain_code, step);
                if l_sub(ctx, raised, self.gc_threshold).0 > 0 {
                    self.gc_threshold
                } else {
                    raised
                }
            } else {
                let lowered = mpy_32_16(gain_hi, gain_lo, Word16(27536));
                if l_sub(ctx, lowered, self.gc_threshold).0 < 0 {
                    self.gc_threshold
                } else {
                    lowered
                }
            };
            self.gc_threshold = threshold;

            let complement = sub(ctx, Word16(32767), fac);
            let kept = mpy_32_16(gain_hi, gain_lo, complement);
            let (threshold_hi, threshold_lo) = l_extract(threshold);
            let pulled = mpy_32_16(threshold_hi, threshold_lo, fac);
            let smoothed = l_add(ctx, kept, pulled);

            // A gentle high-pass whose strength is the voicing: a quarter of
            // each neighbour when fully voiced, none at all when unvoiced.
            let eighth_voicing = shr(ctx, voice_fac, 3);
            let tap = add(ctx, eighth_voicing, Word16(4096));
            let mut shaped = [Word16(0); L_SUBFR];
            {
                let acc = l_deposit_h(code[0]);
                let acc = l_msu(ctx, acc, code[1], tap);
                shaped[0] = round(ctx, acc);
            }
            for i in 1..L_SUBFR - 1 {
                let acc = l_deposit_h(code[i]);
                let acc = l_msu(ctx, acc, code[i + 1], tap);
                let acc = l_msu(ctx, acc, code[i - 1], tap);
                shaped[i] = round(ctx, acc);
            }
            {
                let acc = l_deposit_h(code[L_SUBFR - 1]);
                let acc = l_msu(ctx, acc, code[L_SUBFR - 2], tap);
                shaped[L_SUBFR - 1] = round(ctx, acc);
            }

            let scaled = l_shl(ctx, smoothed, q_new);
            let gain_code = round(ctx, scaled);
            let mut out = [Word16(0); L_SUBFR];
            for i in 0..L_SUBFR {
                let acc = l_mult(ctx, shaped[i], gain_code);
                let acc = l_shl(ctx, acc, 5);
                let acc = l_mac(ctx, acc, adaptive[i], gain_pit);
                // Saturation can occur here and is the reference's behaviour.
                let acc = l_shl(ctx, acc, 1);
                out[i] = round(ctx, acc);
            }
            out
        }

        /// Synthesise the high band the decoder will build and code the gain
        /// correction that brings it onto the original, `cod_main.c`
        /// 1408–1619.
        ///
        /// `reference` is the *unprocessed* 16 kHz input for this subframe:
        /// what the far end should end up hearing, which is what the correction
        /// is measured against.
        #[allow(clippy::too_many_arguments)]
        pub fn analyse(
            &mut self,
            ctx: &mut DspContext,
            aq: &[Word16; M + 1],
            excitation: &[Word16; L_SUBFR],
            q_new: i16,
            reference: &[Word16; L_SUBFR16K],
            vad_hist: i16,
            dtx_hangover: Word16,
        ) -> u16 {
            // --- the low band, synthesised exactly as the decoder will -----
            let (high, low) = self.synthesis.filter(aq, excitation, q_new);
            let mut synth = deemphasis(&high, &low, PREEMPH_FAC, &mut self.deemph_memory);
            self.output_highpass.filter(&mut synth);

            let mut original = *reference;
            let mut noise = self.noise.fill(ctx);

            // --- noise at the excitation's energy ---------------------------
            let mut scaled_excitation = *excitation;
            scale_sig(ctx, &mut scaled_excitation, -3);
            match_energy(ctx, &scaled_excitation, &mut noise, q_new - 3);

            // --- the gain the decoder would guess from the tilt alone -------
            let tilt = spectral_tilt(ctx, &mut self.tilt, &mut synth);
            let estimated = gain_from_tilt(ctx, tilt, vad_hist);

            // --- shape and band-limit the noise, and band-limit the original -
            self.shaper.shape(ctx, aq, &mut noise);
            self.band_noise.filter(ctx, &mut noise);
            self.band_reference.filter(ctx, &mut original);
            scale_sig(ctx, &mut original, -1);

            // --- the gain that would actually be right ----------------------
            let measured = energy_ratio(ctx, &original, &noise);

            // The one place the DTX hangover reaches the transmitted
            // bitstream. With DTX off the counter never leaves its reset value
            // of 7, `7 > 6` holds, and this collapses to unity — which is why
            // eight of the nine normative encoder vectors reproduce without
            // DTX and 23.85 kbit/s does not. With DTX on the counter walks
            // down through a talk spurt's hangover and the transmitted gain
            // index moves with it.
            let fraction = l_mult(ctx, dtx_hangover, Word16(4681));
            let hangover = l_shl(ctx, fraction, 15);
            let decayed = extract_h(hangover);
            self.gain_alpha = mult(ctx, self.gain_alpha, decayed);
            if dtx_hangover.0 > 6 {
                self.gain_alpha = Word16(32767);
            }

            // Q15 to Q14: the transmitted table is Q14.
            let estimated = shr(ctx, estimated, 1);
            let from_measurement = mult(ctx, measured, self.gain_alpha);
            let complement = sub(ctx, Word16(32767), self.gain_alpha);
            let from_estimate = mult(ctx, complement, estimated);
            let corrected = add(ctx, from_measurement, from_estimate);

            // Nearest table entry, strictly closer to displace the incumbent,
            // so a tie keeps the lower index.
            let mut best = 0u16;
            let mut best_distance = Word16(32767);
            for index in 0..16u16 {
                let delta = sub(ctx, corrected, transmitted_gain(index));
                let distance = mult(ctx, delta, delta);
                if best_distance.0 > distance.0 {
                    best_distance = distance;
                    best = index;
                }
            }
            best
        }
    }

    /// `sqrt(energy(target) / energy(source))`, Q15 — see below.
    ///
    /// The same computation [`match_energy`] performs, but the scale is
    /// returned rather than applied, and the final shift is one bit smaller —
    /// that bit is the factor of two `match_energy` folds in on purpose.
    fn energy_ratio(
        ctx: &mut DspContext,
        target: &[Word16; L_SUBFR16K],
        source: &[Word16; L_SUBFR16K],
    ) -> Word16 {
        let (energy, target_exp) = dot_product12(ctx, target, target);
        let target_energy = extract_h(energy);

        let (energy, mut exp) = dot_product12(ctx, source, source);
        let mut source_energy = extract_h(energy);
        // `div_s` wants a proper fraction, so make sure the numerator is the
        // smaller of the two.
        if source_energy.0 > target_energy.0 {
            source_energy = shr(ctx, source_energy, 1);
            exp += 1;
        }

        let ratio = l_deposit_h(div_s(source_energy, target_energy));
        let (frac, exp) = isqrt_n(ctx, (ratio, exp - target_exp));
        extract_h(l_shl(ctx, frac, exp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference encoder's own per-stage trace, three frames at 12.65
    /// kbit/s.
    const TRACE: &str = include_str!("../../testdata/wb_enc_trace.txt");

    /// The 16 kHz input both the trace and the bitstreams were produced from.
    const INPUT: &[u8] = include_bytes!("../../testdata/amrwb_enc_input.pcm");

    /// The reference encoder's output at each of the nine rates.
    const BITSTREAMS: [&[u8]; 9] = [
        include_bytes!("../../testdata/amrwb_enc_mode0.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode1.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode2.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode3.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode4.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode5.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode6.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode7.amr"),
        include_bytes!("../../testdata/amrwb_enc_mode8.amr"),
    ];

    /// Frames in the committed bitstreams.
    const FRAMES: usize = 50;
    /// Frames the committed trace covers.
    const TRACED_FRAMES: usize = 3;
    /// The rate the trace was produced at.
    const TRACE_MODE: u8 = 2;
    /// The storage format's magic number, `#!AMR-WB\n`.
    const MAGIC: usize = 9;

    fn input_frame(frame: usize) -> [i16; L_FRAME16K] {
        let mut samples = [0i16; L_FRAME16K];
        let base = frame * L_FRAME16K * 2;
        for (n, slot) in samples.iter_mut().enumerate() {
            let lo = u16::from(INPUT[base + n * 2]);
            let hi = u16::from(INPUT[base + n * 2 + 1]);
            *slot = (lo | (hi << 8)) as i16;
        }
        samples
    }

    /// Every row of the committed trace, in file order.
    fn reference_rows() -> Vec<(usize, i32, String, Vec<i32>)> {
        let mut rows = Vec::new();
        for line in TRACE.lines() {
            let mut field = line.split_whitespace();
            if field.next() != Some("T") {
                continue;
            }
            let frame: usize = field.next().expect("frame").parse().expect("frame");
            let subframe: i32 = field.next().expect("subframe").parse().expect("subframe");
            let name = field.next().expect("name").to_owned();
            let values = field.map(|v| v.parse().expect("value")).collect();
            rows.push((frame, subframe, name, values));
        }
        assert!(!rows.is_empty(), "the committed trace parsed to nothing");
        rows
    }

    /// Encode `frames` frames at `mode`, recording the trace.
    fn run(mode: u8, frames: usize) -> (Vec<Vec<u8>>, EncoderTrace) {
        let rate = Rate::from_index(mode).expect("a speech mode");
        let mut encoder = WbEncoder::new();
        encoder.record_trace();
        let mut payloads = Vec::with_capacity(frames);
        for frame in 0..frames {
            payloads.push(encoder.encode_frame(&input_frame(frame), rate));
        }
        let trace = encoder.take_trace().expect("recording was enabled");
        (payloads, trace)
    }

    /// With DTX on, the encoder reproduces the reference's frame-type sequence.
    ///
    /// The committed fixture is what TS 26.173's own encoder made of
    /// `amrwb_dtx_input.pcm` with `-dtx`, so its table-of-contents bytes are
    /// the reference's own speech / SID / `NO_DATA` decisions over 150 frames.
    /// Reproducing them exercises the VAD, `tx_dtx_handler`'s asymmetric
    /// hangover and the transmit cadence together -- none of which any other
    /// test reaches, because comfort noise is off by default.
    #[test]
    fn the_frame_type_sequence_matches_the_reference_with_dtx_on() {
        use crate::codecs::amr::mode::{AmrFrameType, AmrMode, AmrVariant};
        use crate::codecs::amr::sid_cadence::SidCadence;
        use crate::codecs::amr::storage;

        let bits: &[u8] = include_bytes!("../../testdata/amrwb_dtx_mode2.amr");
        let pcm: &[u8] = include_bytes!("../../testdata/amrwb_dtx_input.pcm");
        let (_, want) = storage::read(bits).expect("fixture parses");

        let rate = Rate::from_index(2).expect("12.65 kbit/s");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("12.65 kbit/s");
        let mut encoder = WbEncoder::new();
        encoder.set_allow_dtx(true);
        let mut cadence = SidCadence::new(AmrVariant::WideBand);

        let mut got = Vec::with_capacity(want.len());
        let mut sid_payloads = Vec::new();
        for frame in 0..want.len() {
            let mut samples = [0i16; L_FRAME16K];
            for (slot, chunk) in samples
                .iter_mut()
                .zip(pcm[frame * 2 * L_FRAME16K..].chunks_exact(2))
            {
                *slot = i16::from_le_bytes([chunk[0], chunk[1]]);
            }
            let (comfort_noise, mut payload) = encoder.encode_frame_typed(&samples, rate);
            let frame_type = cadence.next(comfort_noise, mode);
            if frame_type == AmrFrameType::Sid(AmrVariant::WideBand) {
                let update = cadence.last_sid_was_an_update();
                crate::codecs::amr::wb::bitstream::finish_sid_payload(&mut payload, update, 2);
                if !update {
                    crate::codecs::amr::wb::bitstream::blank_sid_first(&mut payload);
                }
                sid_payloads.push(payload);
            }
            got.push(frame_type);
        }

        assert_eq!(
            got.len(),
            want.len(),
            "the fixture and the run disagree on length"
        );
        let want_types: Vec<AmrFrameType> = want.iter().map(|f| f.frame_type).collect();
        // Report the first divergence rather than a wall of frame types.
        for (frame, (&mine, &theirs)) in got.iter().zip(&want_types).enumerate() {
            assert_eq!(mine, theirs, "frame {frame} differs");
        }

        // The SID payloads themselves, not just where they fall. This is what
        // ties the DTX kernel -- already exact against `dtx_enc` in isolation
        // -- to the wiring that feeds it: a right kernel fed the wrong
        // residual or the wrong history produces a well-formed SID with wrong
        // bits, and the frame-type sequence above would not notice.
        let want_sids: Vec<Vec<u8>> = want
            .iter()
            .filter(|f| f.frame_type == AmrFrameType::Sid(AmrVariant::WideBand))
            .map(|f| f.data.clone())
            .collect();
        assert_eq!(sid_payloads.len(), want_sids.len(), "SID count differs");
        assert!(
            sid_payloads.len() >= 8,
            "too few SIDs to be worth comparing"
        );
        for (n, (mine, theirs)) in sid_payloads.iter().zip(&want_sids).enumerate() {
            assert_eq!(mine, theirs, "SID {n} payload differs");
        }

        // And the sequence has to be worth comparing: all three kinds present,
        // in both directions.
        let speech = got
            .iter()
            .filter(|t| matches!(t, AmrFrameType::Speech(_)))
            .count();
        let sid = got
            .iter()
            .filter(|t| matches!(t, AmrFrameType::Sid(_)))
            .count();
        let quiet = got.iter().filter(|t| **t == AmrFrameType::NoData).count();
        assert!(
            speech > 50 && sid >= 8 && quiet > 40,
            "{speech} / {sid} / {quiet}"
        );
    }

    /// The DTX hangover counter reaches the wire, and only at 23.85 kbit/s.
    ///
    /// Eight of the nine normative encoder vectors reproduce with DTX off and
    /// mode 8 does not, because mode 8 is the only rate whose high-band
    /// correction gain is scaled by `gain_alpha`, which is scaled by this
    /// counter. With the counter pinned at its reset value the expression
    /// collapses to unity -- so a port that hardcoded it would pass every
    /// existing test here and fail the one vector that matters.
    #[test]
    fn the_dtx_hangover_moves_mode_8_and_leaves_the_other_rates_alone() {
        let frames = 6;
        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");

            let mut plain = WbEncoder::new();
            let baseline: Vec<Vec<u8>> = (0..frames)
                .map(|f| plain.encode_frame(&input_frame(f), rate))
                .collect();

            let mut hungover = WbEncoder::new();
            // Four quiet classifications leave the counter at 3, inside the
            // range where `gain_alpha` decays rather than saturating.
            hungover.force_dtx_hangover(4);
            let moved: Vec<Vec<u8>> = (0..frames)
                .map(|f| hungover.encode_frame(&input_frame(f), rate))
                .collect();

            if mode == 8 {
                assert_ne!(
                    baseline, moved,
                    "23.85 kbit/s ignored the hangover counter; gain_alpha is not wired"
                );
            } else {
                assert_eq!(
                    baseline, moved,
                    "mode {mode} changed with the hangover counter, and only mode 8 should"
                );
            }
        }
    }

    #[test]
    fn every_traced_intermediate_is_bit_exact_against_ts26173() {
        // The whole point of the fixture: a divergence is located by the first
        // row that differs, not by staring at the bitstream. The comparison
        // count is asserted because a harness that silently compares nothing
        // reads exactly like one that agrees.
        // `decimated` is the band-limited frame *before* pre-emphasis, which
        // lives inside `preproc` and never reaches this module —
        // `FrontEndFrame` carries the pre-emphasised `window` instead. It is
        // covered by
        // `preproc::tests::decimation_and_highpass_are_bit_exact_against_ts26173`
        // over the same three frames, so it is skipped here by name rather than
        // quietly missing.
        const NOT_SURFACED: &str = "decimated";

        let (_, got) = run(TRACE_MODE, TRACED_FRAMES);
        let want = reference_rows();

        let mut compared_rows = 0usize;
        let mut compared_values = 0usize;
        let mut skipped = 0usize;
        for (frame, subframe, name, expected) in &want {
            if name == NOT_SURFACED {
                assert!(
                    got.row(*frame, *subframe, name).is_none(),
                    "{name} is now surfaced; compare it instead of skipping it"
                );
                skipped += 1;
                continue;
            }
            let actual = got.row(*frame, *subframe, name).unwrap_or_else(|| {
                panic!("frame {frame} subframe {subframe}: this encoder never produced {name}")
            });
            assert_eq!(
                actual.len(),
                expected.len(),
                "frame {frame} subframe {subframe}: {name} length"
            );
            for (i, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
                assert_eq!(
                    a, e,
                    "frame {frame} subframe {subframe}: {name}[{i}] = {a} but TS 26.173 gives {e}"
                );
                compared_values += 1;
            }
            compared_rows += 1;
        }

        assert_eq!(skipped, 3, "only the three `decimated` rows may be skipped");
        assert_eq!(
            compared_rows + skipped,
            350,
            "the committed trace has 350 rows"
        );
        assert_eq!(
            compared_values, 12_791,
            "every traced value outside `decimated` must have been compared"
        );
    }

    #[test]
    fn the_bitstream_is_byte_identical_to_ts26173_at_every_rate() {
        // The deliverable. Nine rates, fifty frames each, against the output of
        // the normative encoder driven from the same PCM.
        let mut exact = Vec::new();
        let mut failures = Vec::new();

        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let file = BITSTREAMS[mode as usize];
            assert_eq!(&file[..MAGIC], b"#!AMR-WB\n", "mode {mode}: magic number");

            let (payloads, _) = run(mode, FRAMES);
            let stride = 1 + rate.packed_bytes();
            assert_eq!(
                file.len(),
                MAGIC + FRAMES * stride,
                "mode {mode}: fixture is not fifty frames"
            );

            let mut wrong = 0usize;
            let mut first: Option<(usize, usize)> = None;
            for (frame, payload) in payloads.iter().enumerate() {
                let at = MAGIC + frame * stride;
                assert_eq!(
                    file[at],
                    rate.toc_byte(),
                    "mode {mode} frame {frame}: table-of-contents byte"
                );
                let expected = &file[at + 1..at + 1 + rate.packed_bytes()];
                if payload.as_slice() != expected {
                    wrong += 1;
                    if first.is_none() {
                        let byte = payload
                            .iter()
                            .zip(expected.iter())
                            .position(|(a, b)| a != b)
                            .expect("the frames differ somewhere");
                        first = Some((frame, byte));
                    }
                }
            }

            if wrong == 0 {
                exact.push(mode);
            } else {
                failures.push(format!(
                    "mode {mode}: {wrong}/{FRAMES} frames differ, first at frame {} byte {}",
                    first.expect("a first difference").0,
                    first.expect("a first difference").1
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "byte-exact at modes {exact:?}; {}",
            failures.join("; ")
        );
    }

    #[test]
    fn the_voice_activity_bit_is_not_hard_wired() {
        // Every frame of the committed input is speech, at every rate, so the
        // byte-exactness above proves only that the bit is *carried* — it would
        // pass with the detector replaced by a constant. Silence is the cheapest
        // input whose answer is known independently: `pow_sum` cannot reach
        // `VAD_POW_LOW`, so `hangover_addition` returns zero on the first frame
        // and every frame after it.
        let rate = Rate::from_index(TRACE_MODE).expect("a speech mode");
        let mut encoder = WbEncoder::new();
        let mut frames = 0usize;
        for _ in 0..5 {
            let payload = encoder.encode_frame(&[0i16; L_FRAME16K], rate);
            // The permutation puts codec bit 0 at payload bit 0 for every rate.
            assert_eq!(payload[0] >> 7, 0, "silence was called speech");
            frames += 1;
        }
        assert_eq!(frames, 5, "the silent run produced no frames");
    }

    #[test]
    fn extreme_input_produces_a_frame_rather_than_a_panic() {
        // Fixed-point division here carries a precondition — `div_s` wants a
        // proper fraction — and several of its call sites reach it through a
        // chain of energies and normalisations. Full-scale square waves and
        // alternating extremes drive every one of those chains to its limit at
        // every rate, including the high band, which only 23.85 kbit/s runs.
        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = WbEncoder::new();
            for frame in 0..6 {
                let mut pcm = [0i16; L_FRAME16K];
                for (i, slot) in pcm.iter_mut().enumerate() {
                    *slot = match frame % 3 {
                        0 => i16::MIN,
                        1 => {
                            if i % 2 == 0 {
                                i16::MAX
                            } else {
                                i16::MIN
                            }
                        }
                        _ => i16::MAX,
                    };
                }
                let payload = encoder.encode_frame(&pcm, rate);
                assert_eq!(payload.len(), rate.packed_bytes(), "mode {mode}");
            }
        }
    }

    #[test]
    fn the_parameter_writer_fills_the_frame_exactly() {
        // A layout that overruns or underruns its frame does not fail loudly:
        // it shifts every later field and yields plausible parameters. The
        // defence is conservation, and it is the same one the decoder's own
        // field walk uses.
        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = WbEncoder::new();
            let mut prms = Parameters::new();
            encoder.encode_into(&input_frame(0), rate, &mut prms);
            assert_eq!(
                prms.len,
                rate.bits(),
                "mode {mode}: wrote {} of {} codec bits",
                prms.len,
                rate.bits()
            );
        }
    }

    #[test]
    fn the_payload_is_a_permutation_of_the_codec_bits() {
        // Packing is the inverse of the decoder's unsorting, and the cheapest
        // way to say so is to unsort what was just sorted.
        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut prms = Parameters::new();
            for i in 0..rate.bits() {
                // A pattern with no period that divides eight, so a byte-order
                // slip cannot survive it.
                prms.push(u16::from(i % 3 == 0), 1);
            }
            let packed = prms.pack(rate);
            let sort = rate.sort_table();
            let mut recovered = vec![0u8; rate.bits()];
            for (i, &target) in sort.iter().enumerate() {
                recovered[target as usize] = (packed[i / 8] >> (7 - (i % 8))) & 1;
            }
            assert_eq!(
                recovered,
                prms.bits[..rate.bits()].to_vec(),
                "mode {mode}: packing is not the inverse of unsorting"
            );
        }
    }
    /// Independent end-to-end check, written outside the assembly's own tests:
    /// encode the committed PCM and compare the *file* against the reference
    /// `.amr`, byte for byte, including magic and table of contents.
    #[test]
    #[ignore = "independent verification; writes to a temp dir"]
    fn independently_reproduce_the_reference_files() {
        let pcm: Vec<i16> = include_bytes!("../../testdata/amrwb_enc_input.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(pcm.len(), 50 * 320);
        // `cargo test --nocapture` leaves the test-name line open, so the first
        // `println!` is appended to it. Mode 0's line has been eaten by this
        // three times in this branch already.
        println!();

        let refs: [&[u8]; 9] = [
            include_bytes!("../../testdata/amrwb_enc_mode0.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode1.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode2.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode3.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode4.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode5.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode6.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode7.amr"),
            include_bytes!("../../testdata/amrwb_enc_mode8.amr"),
        ];

        for mode in 0..9u8 {
            let rate = Rate::from_index(mode).expect("rate");
            let mut enc = WbEncoder::new();
            let mut out: Vec<u8> = b"#!AMR-WB\n".to_vec();
            for frame in pcm.chunks_exact(320) {
                let mut block = [0i16; 320];
                block.copy_from_slice(frame);
                let payload = enc.encode_frame(&block, rate);
                // Storage ToC: FT in bits 6..3, quality bit set.
                out.push((mode << 3) | 0x04);
                out.extend_from_slice(&payload);
            }
            let want = refs[usize::from(mode)];
            assert_eq!(out.len(), want.len(), "mode {mode}: file length");
            let first = out.iter().zip(want).position(|(a, b)| a != b);
            assert!(
                first.is_none(),
                "mode {mode}: first differs at byte {}",
                first.unwrap()
            );
            println!("mode {mode}: {} bytes byte-identical", out.len());

            // The comparison must not be vacuous. Nudging one input sample by
            // a single LSB has to move the bitstream — if it does not, this
            // test is comparing something other than what it thinks.
            let mut nudged = pcm.clone();
            nudged[1000] = nudged[1000].wrapping_add(4);
            let mut enc = WbEncoder::new();
            let mut other: Vec<u8> = b"#!AMR-WB\n".to_vec();
            for frame in nudged.chunks_exact(320) {
                let mut block = [0i16; 320];
                block.copy_from_slice(frame);
                other.push((mode << 3) | 0x04);
                other.extend_from_slice(&enc.encode_frame(&block, rate));
            }
            assert_ne!(
                other, out,
                "mode {mode}: a one-LSB change to the input left the bitstream \
                 unchanged, so this test proves nothing"
            );
        }
    }
}
