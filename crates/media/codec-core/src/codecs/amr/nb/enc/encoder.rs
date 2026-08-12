//! The AMR-NB encoder proper: one frame of speech in, one packed frame out.
//!
//! Every other module under `enc/` is a stage that was made bit-exact on its
//! own against TS 26.073's instrumented encoder. This module is the sequence
//! those stages run in — `cod_amr()` wrapped in `Speech_Encode_Frame()` — plus
//! the two things that belong to no stage and therefore had nowhere else to
//! live:
//!
//! * the **local synthesis** (`spstproc.c`), which is what makes an ACELP
//!   encoder closed loop at all: the target for subframe *n+1* is built from
//!   the excitation subframe *n* actually chose, so a divergence compounds
//!   rather than staying local;
//! * the **frame assembly** — which parameter word goes where, and the TS
//!   26.101 permutation that turns codec bits into a payload.
//!
//! Discontinuous transmission is implemented and off by default, exactly as
//! `coder.c` is: without `-dtx` every frame is speech, and with it the VAD1
//! decision drives `dtx_buffer` and the `MRDTX` branch of `cod_amr`. Both
//! shapes are pinned — the speech-only vectors byte-for-byte, and the DTX
//! fixtures (SID cadence included) in `super::super::super::tests`.
//!
//! # 4.75 kbit/s codes two subframes jointly
//!
//! Seven of the eight rates are a plain loop: pre-process, search, quantise,
//! synthesise, next. 4.75 is not. It saves `mem_syn`, `mem_w0`, `mem_err` and
//! `sharp` on the even subframe, synthesises that subframe *provisionally* with
//! unquantised gains into the saved copies, runs the odd subframe, and only
//! then — once the four-dimensional quantiser has chosen gains for the pair —
//! rebuilds the even subframe's adaptive excitation, re-convolves it with the
//! impulse response saved before the codebook search sharpened it, and redoes
//! the even subframe's post-processing into the *real* memories. The odd
//! subframe's own pre-processing then has to run a second time, because the
//! memories it originally saw were the provisional ones, and its adaptive
//! excitation has to be rebuilt because the history it interpolates from
//! changed underneath it.
//!
//! Two consequences that are invisible in the output waveform:
//!
//! * `st->sharp` is restored after the even subframe, so **both** subframes of
//!   a 4.75 pair search their codebook with the same sharpening constant;
//! * `update_gp_clipping` still runs once per subframe — four times a frame,
//!   as in every other rate — and on the even subframe it is handed the
//!   *unquantised* pitch gain. Firing it once per pair desynchronises the
//!   seven-tap history for the rest of the stream.
//!
//! # Q-formats
//!
//! Speech, excitation, the targets `xn`/`xn2`, the residuals and `y1` are Q0.
//! Predictor coefficients are Q12, and so are the impulse response `h1` and the
//! filtered innovation `y2` (Q10 at 12.2 kbit/s). LSPs are Q15 cosines, LSFs
//! Q15 on a 0..16384 scale. The pitch gain is Q14, the code gain Q1, and the
//! algebraic codevector Q13 (Q12 at 12.2 kbit/s).
//!
//! # What validated it
//!
//! `testdata/nb_enc_trace.txt` — every traced intermediate of three frames at
//! 7.40 kbit/s, with the comparison count asserted — and
//! `testdata/amrnb_enc_mode*.amr`, the reference encoder's own output for fifty
//! frames at each of the eight rates, compared byte for byte.

// This module transcribes reference fixed-point arithmetic and a bit layout.
// The lints below fight the transcription rather than the code.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::similar_names,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use super::super::bitstream::parameter_widths;
use super::super::codebook::FixedCodebook;
use super::super::lag::{Excitation, PitchLag};
use super::super::lsp::{initial_lsp, interpolate_lsp, interpolate_lsp_mid, AZ_SIZE, M, MP1};
use super::super::synthesis::synthesis_filter;
use super::super::tables::{
    SORT_102, SORT_122, SORT_475, SORT_515, SORT_59, SORT_67, SORT_74, SORT_795,
};
use super::super::{L_FRAME, L_INTERPOL, L_SUBFR, PIT_MAX, SHARPMAX};

use super::analysis::{
    az_lsp, subframe_targets, LpAnalysis, SpeechBuffer, SubframeTargets, WeightedSpeech, MR122,
    MR475,
};
use super::codebook::{search as codebook_search, CodebookInputs};
use super::gain_quant::{GainParams, GainQuantiser, SubframeSignals};
use super::dtx::{DtxEncoder, TxDecision};
use super::lsp_quant::LsfQuantiser;
use super::vad::VoiceActivityDetector;
use super::pitch::{
    closed_loop_ltp, convolve, open_loop_lags, ClosedLoopPitch, ToneStability, WeightedOpenLoop,
    EXC_ORIGIN,
};
use super::preproc::Preprocessor;

use crate::fixed_point::arith::{add, extract_h, round, sub};
use crate::fixed_point::arith32::{l_mac, l_mult};
use crate::fixed_point::shift::{l_shl, shr};
use crate::fixed_point::types::{DspContext, Word16};

/// Subframes per frame.
const NB_SUBFR: usize = 4;

/// The whole working excitation buffer, `PIT_MAX + L_INTERPOL + L_FRAME`.
///
/// The reference's `old_exc`; its `st->exc` is this offset by [`EXC_ORIGIN`],
/// so subframe `k` occupies `EXC_ORIGIN + k·L_SUBFR ..`.
const EXC_TOTAL: usize = EXC_ORIGIN + L_FRAME;

/// The window one call into the pitch stage sees: the history it may reach back
/// over, followed by the subframe being built.
const EXC_VIEW: usize = EXC_ORIGIN + L_SUBFR;

/// `old_wsp`: the weighted speech, with a whole pitch period of history in
/// front so the open-loop search can reach back across the frame boundary.
const WSP_TOTAL: usize = PIT_MAX as usize + L_FRAME;

/// Frame sizes in bits, indexed by mode.
const NB_OF_BITS: [usize; 8] = [95, 103, 118, 134, 148, 159, 204, 244];

// ---------------------------------------------------------------------------
// Rate
// ---------------------------------------------------------------------------

/// The rate a frame is encoded at.
///
/// A newtype rather than a bare integer, because a mode index and a subframe
/// number are both small integers and both plausible in the same argument slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rate(u8);

impl Rate {
    /// The rate for mode index 0..=7, or `None` for anything else.
    ///
    /// Eight speech rates only: comfort noise is a frame *type* rather than a
    /// rate this encoder can be asked for.
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        if index <= 7 {
            Some(Self(index))
        } else {
            None
        }
    }

    /// The mode index, 0..=7, in TS 26.073's own numeric order.
    #[must_use]
    pub const fn index(self) -> u8 {
        self.0
    }

    /// Speech bits per frame.
    #[must_use]
    pub const fn bits(self) -> usize {
        NB_OF_BITS[self.0 as usize]
    }

    /// Bytes the RFC 4867 storage payload occupies, **excluding** the
    /// table-of-contents byte.
    ///
    /// TS 26.073's own `packed_size` table counts that byte; this does not,
    /// matching the wideband convention used elsewhere in this crate.
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

    /// Payload bit order, TS 26.101: `codec_bits[table[i]]` is payload bit `i`.
    const fn sort_table(self) -> &'static [u16] {
        match self.0 {
            0 => &SORT_475,
            1 => &SORT_515,
            2 => &SORT_59,
            3 => &SORT_67,
            4 => &SORT_74,
            5 => &SORT_795,
            6 => &SORT_102,
            _ => &SORT_122,
        }
    }

    /// Width of every transmitted parameter, in the order they are written.
    ///
    /// Shared with the decoder's field walk rather than restated: the two must
    /// agree, and a second copy is a second thing to get wrong.
    const fn widths(self) -> &'static [usize] {
        parameter_widths(self.0)
    }
}

// ---------------------------------------------------------------------------
// Frame assembly
// ---------------------------------------------------------------------------

/// The reference's `prm[]`: one entry per transmitted parameter, in codec
/// order, written as the subframe loop reaches each one.
///
/// 4.75 kbit/s is why this is a list with a back-fill rather than a bit writer:
/// its even subframe *reserves* a slot whose value is only known once the odd
/// subframe has run.
#[derive(Debug, Default)]
struct Parameters {
    words: Vec<u16>,
}

impl Parameters {
    const fn new() -> Self {
        Self { words: Vec::new() }
    }

    fn push(&mut self, value: u16) {
        self.words.push(value);
    }

    /// Claim a slot to be filled later, returning its position.
    fn reserve(&mut self) -> usize {
        self.words.push(0);
        self.words.len() - 1
    }

    /// Fill a slot claimed by [`Self::reserve`].
    fn fill(&mut self, at: usize, value: u16) {
        self.words[at] = value;
    }

    /// Widen each parameter to its field, then permute into the payload.
    ///
    /// The two steps are the exact inverse of
    /// [`super::super::bitstream::parse`], which the decoder next door uses, so
    /// a round trip through the pair checks both.
    ///
    /// # Panics
    ///
    /// If the parameter count does not match the rate's layout. A frame that
    /// wrote the wrong number of words would otherwise shift every later field
    /// and still produce a plausible payload.
    fn pack(&self, rate: Rate) -> Vec<u8> {
        let widths = rate.widths();
        assert_eq!(
            self.words.len(),
            widths.len(),
            "mode {}: wrote {} parameters, the layout has {}",
            rate.index(),
            self.words.len(),
            widths.len()
        );

        let mut bits = Vec::with_capacity(rate.bits());
        for (&value, &width) in self.words.iter().zip(widths.iter()) {
            debug_assert!(
                u32::from(value) < (1u32 << width),
                "mode {}: parameter {value} does not fit in {width} bits",
                rate.index()
            );
            for i in (0..width).rev() {
                bits.push(((value >> i) & 1) as u8);
            }
        }
        assert_eq!(bits.len(), rate.bits(), "the layout must fill the frame");

        let sort = rate.sort_table();
        let mut out = vec![0u8; rate.packed_bytes()];
        for (i, &source) in sort.iter().enumerate() {
            out[i / 8] |= bits[source as usize] << (7 - (i % 8));
        }
        out
    }

    /// The same, for a SID frame's own five-parameter layout.
    ///
    /// Five octets, of which the first 35 bits are the description and the
    /// remaining five are left clear for the caller to finish — the STI bit,
    /// the three-bit mode indication, and one spare.
    ///
    /// # Panics
    /// If five parameters were not written.
    fn pack_sid(&self) -> Vec<u8> {
        let widths = parameter_widths(8);
        assert_eq!(self.words.len(), widths.len(), "a SID is five parameters");

        let mut bits = Vec::with_capacity(35);
        for (&value, &width) in self.words.iter().zip(widths.iter()) {
            debug_assert!(
                u32::from(value) < (1u32 << width),
                "SID parameter {value} does not fit in {width} bits"
            );
            for i in (0..width).rev() {
                bits.push(u8::try_from((value >> i) & 1).expect("one bit"));
            }
        }
        assert_eq!(bits.len(), 35, "the SID layout must fill 35 bits");

        let sort = super::super::tables::SORT_SID;
        let mut out = vec![0u8; 5];
        for (i, &source) in sort.iter().enumerate() {
            out[i / 8] |= bits[source as usize] << (7 - (i % 8));
        }
        out
    }
}

/// The transmitted words for one subframe's algebraic codebook, in `ana` order.
///
/// The narrow rates send a position word then a sign word; the two widest send
/// the vector of sub-indices their search already assembled.
fn codebook_words(book: FixedCodebook, out: &mut Parameters) {
    match book {
        FixedCodebook::TwoPulses9Bit {
            signs, positions, ..
        }
        | FixedCodebook::TwoPulses11Bit { signs, positions }
        | FixedCodebook::ThreePulses14Bit { signs, positions }
        | FixedCodebook::FourPulses17Bit { signs, positions } => {
            out.push(positions);
            out.push(signs);
        }
        FixedCodebook::EightPulses31Bit(fields) => {
            for field in fields {
                out.push(field);
            }
        }
        FixedCodebook::TenPulses35Bit(fields) => {
            for field in fields {
                out.push(field);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-stage trace
// ---------------------------------------------------------------------------

/// A recording of every intermediate the instrumented reference dumps.
///
/// Off by default. The row names are exactly those of
/// `testdata/nb_enc_trace.txt`, so a divergence is found by comparing rows
/// rather than by reasoning about the output bitstream — which is the only
/// method that has ever worked on this codec.
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

/// The AMR-NB encoder, 3GPP TS 26.090 / TS 26.073 `Speech_Encode_Frame`.
///
/// Owns every field of the reference's state that a non-DTX encoder reaches.
/// One instance encodes one stream; the rate may change from frame to frame and
/// nothing is reset when it does, which is the reference's behaviour and is why
/// a mode switch takes a few frames to settle.
#[derive(Debug, Clone)]
pub struct NbEncoder {
    // --- input conditioning ------------------------------------------------
    /// `Pre_ProcessState`: the 80 Hz high-pass that also halves the level.
    preprocessor: Preprocessor,
    /// `old_speech`: two frames of conditioned speech, with the analysis
    /// windows and the coded frame at fixed offsets inside it.
    speech: SpeechBuffer,

    // --- spectral path -----------------------------------------------------
    /// `lpcState`: the Levinson recursion's carried `old_A`.
    lp: LpAnalysis,
    /// `lspSt->lsp_old`: the previous frame's **unquantised** 4th-subframe
    /// LSPs, Q15, which is also the fallback `Az_lsp` reverts to.
    lsp_old: [Word16; M],
    /// `lspSt->lsp_old_q`: the previous frame's **quantised** LSPs, Q15.
    lsp_old_q: [Word16; M],
    /// `Q_plsfState`: the LSF quantiser's MA prediction memory.
    lsf: LsfQuantiser,

    // --- open-loop pitch ---------------------------------------------------
    /// `pre_big`'s weighting filter memory, updated four times a frame.
    weighting: WeightedSpeech,
    /// `old_wsp`: the weighted speech and a pitch period of history.
    old_wsp: [Word16; WSP_TOTAL],
    /// `pitchOLWghtState`: 10.2 kbit/s' weighted open-loop search.
    weighted_open_loop: WeightedOpenLoop,
    /// `st->old_lags`: the five-entry lag history the weighted search takes its
    /// median over. Reset to 40, and only 10.2 kbit/s ever moves it.
    old_lags: [Word16; 5],
    /// `st->ol_gain_flg`: whether each half-frame's open-loop lag was voiced
    /// enough to join that history. Cleared by `ol_ltp` for every rate but 10.2.
    ol_gain_flg: [bool; 2],

    // --- closed-loop path --------------------------------------------------
    /// `Pitch_frState`: the previous subframe's lag, for the delta windows.
    pitch: ClosedLoopPitch,
    /// `tonStabState`: the resonance counter and the seven-tap gain history.
    tone: ToneStability,
    /// `gainQuantState`: two MA energy predictors and 7.95's gain adaptor.
    gains: GainQuantiser,

    // --- filter memories ---------------------------------------------------
    /// `st->old_exc`: the excitation, history first.
    old_exc: [Word16; EXC_TOTAL],
    /// `st->mem_syn`: the local synthesis filter's memory.
    mem_syn: [Word16; M],
    /// `st->mem_w0`: the weighting filter's memory, i.e. the target's.
    mem_w0: [Word16; M],
    /// `st->mem_err`: the last ten samples of speech minus local synthesis.
    mem_err: [Word16; M],
    /// `st->sharp`: the previous subframe's clamped pitch gain, Q14.
    sharp: Word16,

    // --- discontinuous transmission -----------------------------------------
    /// `vadState`: the VAD1 detector. Driven only when DTX is enabled, because
    /// its pitch, tone and complex registers are fed from the open-loop stage
    /// and the reference guards every one of those hooks on `st->dtx`.
    vad: VoiceActivityDetector,
    /// `dtx_encState`: the transmit-side hangover machine and history rings.
    dtx: DtxEncoder,
    /// `st->dtx`: whether this encoder may emit comfort noise at all.
    dtx_enabled: bool,

    trace: Option<EncoderTrace>,
    frame_index: usize,
}

impl Default for NbEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl NbEncoder {
    /// A cold-start encoder, matching `Speech_Encode_Frame_reset`.
    ///
    /// Note what is *not* primed: `Speech_Encode_Frame_First` exists in the
    /// reference, would fill the 40-sample lookahead ahead of the first frame,
    /// and is never called by `coder.c`. The published vectors are produced
    /// with that region left at zero.
    #[must_use]
    pub fn new() -> Self {
        Self {
            vad: VoiceActivityDetector::new(),
            dtx: DtxEncoder::new(),
            dtx_enabled: false,
            preprocessor: Preprocessor::new(),
            speech: SpeechBuffer::new(),
            lp: LpAnalysis::new(),
            lsp_old: initial_lsp(),
            lsp_old_q: initial_lsp(),
            lsf: LsfQuantiser::new(),
            weighting: WeightedSpeech::new(),
            old_wsp: [Word16(0); WSP_TOTAL],
            weighted_open_loop: WeightedOpenLoop::new(),
            old_lags: [Word16(40); 5],
            ol_gain_flg: [false; 2],
            pitch: ClosedLoopPitch::new(),
            tone: ToneStability::new(),
            gains: GainQuantiser::new(),
            old_exc: [Word16(0); EXC_TOTAL],
            mem_syn: [Word16(0); M],
            mem_w0: [Word16(0); M],
            mem_err: [Word16(0); M],
            // SHARPMIN.
            sharp: Word16(0),
            trace: None,
            frame_index: 0,
        }
    }

    /// Start recording per-stage intermediates.
    ///
    /// A diagnostic rather than part of encoding: the recording is what a
    /// divergence against `testdata/nb_enc_trace.txt` is found with.
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

    /// Encode one 20 ms frame of 8 kHz mono PCM into its RFC 4867 payload.
    ///
    /// The returned bytes are the frame's speech bits only —
    /// [`Rate::packed_bytes`] of them — with no table-of-contents byte in
    /// front; [`Rate::toc_byte`] supplies that.
    pub fn encode_frame(&mut self, pcm: &[i16; L_FRAME], rate: Rate) -> Vec<u8> {
        self.encode_frame_typed(pcm, rate).1
    }

    /// Whether this encoder may replace speech frames with comfort noise.
    ///
    /// Off by default, and off is not merely "never emit a SID": with DTX
    /// disabled the reference does not run the detector at all, and the
    /// open-loop stage's four VAD hooks are skipped with it. So this is a
    /// switch over the analysis, not a filter on the output.
    pub const fn set_allow_dtx(&mut self, allow: bool) {
        self.dtx_enabled = allow;
    }

    /// Encode one frame, reporting whether it came out as comfort noise.
    ///
    /// The payload of a comfort-noise frame is the SID's 35 bits packed into
    /// five octets, with the STI bit and the mode indication left clear —
    /// those depend on the *cadence*, which is the caller's to decide, since
    /// only it knows whether this SID is the first of a silence or a periodic
    /// update. [`super::super::bitstream`] finishes it.
    pub fn encode_frame_typed(&mut self, pcm: &[i16; L_FRAME], rate: Rate) -> (bool, Vec<u8>) {
        let mut prms = Parameters::new();
        let comfort_noise = self.encode_into(pcm, rate, &mut prms);
        let payload = if comfort_noise {
            prms.pack_sid()
        } else {
            prms.pack(rate)
        };
        self.frame_index += 1;
        (comfort_noise, payload)
    }

    /// One frame of `Speech_Encode_Frame`, writing parameters in codec order.
    ///
    /// Returns whether the frame came out as comfort noise.
    fn encode_into(&mut self, pcm: &[i16; L_FRAME], rate: Rate, prms: &mut Parameters) -> bool {
        let mode = rate.index();
        let mut ctx = DspContext::default();

        // --- input conditioning ---------------------------------------------
        // The 13-bit mask and the high-pass both mutate the caller's frame in
        // the reference, and `cod_amr` then copies the *processed* samples into
        // its speech history. Nothing downstream ever sees the raw PCM.
        let mut frame = [Word16(0); L_FRAME];
        for (slot, &sample) in frame.iter_mut().zip(pcm.iter()) {
            *slot = Word16(sample);
        }
        self.preprocessor.condition(&mut ctx, &mut frame);
        self.speech.push(&frame);
        self.trc(-1, "speech", &frame);

        // --- the DTX decision, before any analysis -----------------------------
        // `used_mode` may change here, and everything downstream reads it
        // rather than the requested rate.
        let (comfort_noise, compute_sid) = if self.dtx_enabled {
            let window = *self.speech.vad_window();
            let voice_active = self.vad.process(&mut ctx, &window);
            let (decision, compute) = self.dtx.classify(&mut ctx, voice_active);
            (decision == TxDecision::ComfortNoise, compute)
        } else {
            (false, false)
        };

        // --- LP analysis ------------------------------------------------------
        // Fills slot 3 — and slot 1 at 12.2 kbit/s, from a second window. The
        // interpolation in `spectrum` fills exactly the complement.
        let mut az = [Word16(0); AZ_SIZE];
        self.lp.analyse(&mut ctx, mode, &self.speech, &mut az);

        // --- A(z) to LSP, quantisation, interpolation — `lsp()` ---------------
        // A comfort-noise frame still runs the analysis and the interpolation;
        // only the *quantisation* is skipped, because the SID carries its own
        // spectrum from the averaged history instead.
        let azq = self.spectrum(&mut ctx, mode, &mut az, prms, comfort_noise);
        self.trc(-1, "A_t", &az);

        // `dtx_buffer`, on every frame including speech ones -- the history it
        // fills is what the *next* silence averages.
        if self.dtx_enabled {
            let lsp_new = self.lsp_old;
            let newest = *self.speech.newest();
            self.dtx.buffer(&mut ctx, &lsp_new, &newest);
        }

        if comfort_noise {
            self.build_sid(&mut ctx, compute_sid, prms);
        }

        // `check_lsp` reads `lsp_old` *after* `lsp()` overwrote it with this
        // frame's vector, so the resonance test is on the current frame. Once
        // per frame: reading it per subframe agrees on any fixture where the
        // twelve-frame counter never fires, and diverges on one where it does.
        //
        // Not on a comfort-noise frame: the reference's `else` branch. The
        // resonance counter must not advance on a frame whose spectrum came
        // from the noise history.
        let lsp_new = self.lsp_old;
        let lsp_flag = if comfort_noise {
            false
        } else {
            self.tone.check_lsp(&mut ctx, &lsp_new)
        };

        // --- weighted speech and open-loop pitch ------------------------------
        // The reference interleaves `pre_big` and `ol_ltp` half-frame by
        // half-frame. Each search reads only its own half plus the history in
        // front of it, and `ol_ltp` never touches `mem_w`, so filling the whole
        // frame first is the same computation in a shape that borrows cleanly.
        let mut wsp = [Word16(0); L_FRAME];
        for half in 0..2 {
            self.weighting.half_frame(
                &mut ctx,
                mode,
                &az,
                half * (L_FRAME / 2),
                &self.speech,
                &mut wsp,
            );
        }
        self.old_wsp[PIT_MAX as usize..].copy_from_slice(&wsp);

        let t_op = open_loop_lags(
            &mut ctx,
            mode,
            &mut self.weighted_open_loop,
            &self.old_wsp,
            PIT_MAX as usize,
            &mut self.old_lags,
            &mut self.ol_gain_flg,
            self.dtx_enabled.then_some(&mut self.vad),
        );

        // `vad_pitch_detection`, after both open-loop lags exist and before
        // anything else uses them.
        if self.dtx_enabled {
            self.vad.observe_pitch(&mut ctx, [Word16(t_op[0]), Word16(t_op[1])]);
        }

        // `goto the_end`: a comfort-noise frame runs the analysis and the
        // open-loop search -- both feed state the next speech frame needs --
        // and stops before the subframe loop. Note where the label sits: the
        // excitation slide is *inside* the loop's scope and is skipped, while
        // the weighted-speech and speech slides below are not.
        if comfort_noise {
            self.old_wsp.copy_within(L_FRAME.., 0);
            self.speech.shift();
            return true;
        }
        if mode <= 1 {
            // Only the two rates that search the whole frame at once reach the
            // `T_op[1] = T_op[0]` line the fixture records.
            self.trc1(-1, "T_op0", i32::from(t_op[0]));
        }

        // --- the subframe loop -------------------------------------------------
        let mut exc = self.old_exc;
        let frame_state = FrameState {
            mode,
            az,
            azq,
            t_op,
            lsp_flag,
            lsp_new,
        };
        if mode == MR475 {
            self.subframe_loop_joint(&mut ctx, &frame_state, &mut exc, prms);
        } else {
            self.subframe_loop(&mut ctx, &frame_state, &mut exc, prms);
        }

        // `Copy(&old_exc[L_FRAME], &old_exc[0], PIT_MAX + L_INTERPOL)`.
        exc.copy_within(L_FRAME.., 0);
        self.old_exc = exc;

        // `Copy(&old_wsp[L_FRAME], &old_wsp[0], PIT_MAX)` and the speech
        // buffer's own slide. Both run on every frame.
        self.old_wsp.copy_within(L_FRAME.., 0);
        self.speech.shift();
        false
    }

    /// `dtx_enc`, and the state reset that follows it.
    ///
    /// `compute_sid` says whether a *new* description may be derived. When it
    /// is false the previously computed indices are retransmitted unchanged —
    /// which is the point of holding them in the DTX state rather than
    /// recomputing per frame: a SID sent immediately after a talk spurt would
    /// describe the talker.
    ///
    /// The reset afterwards is the encoder's half of the same agreement the
    /// decoder makes: excitation, weighting memory, error memory, sharpening
    /// and the closed-loop pitch history all go, and the LSP state is reset and
    /// then *overwritten with this frame's unquantised LSPs* — not with the
    /// initial vector `lsp_reset` just wrote. Leaving it at the initial vector
    /// makes the first speech frame after the silence interpolate from a
    /// spectrum neither end ever saw.
    fn build_sid(&mut self, ctx: &mut DspContext, compute_sid: bool, prms: &mut Parameters) {
        if compute_sid {
            let (lsp, _) = self.dtx.average_history(ctx);

            // Order and reorder before quantising: the averaged LSPs can come
            // out too close together or crossed, and the quantiser's weighting
            // divides by their spacing.
            let mut lsf = super::lsp_quant::lsp_to_lsf(ctx, &lsp);
            super::super::lsp::reorder_lsf(ctx, &mut lsf, Word16(205));
            let lsp = super::super::lsp::lsf_to_lsp(ctx, &lsf);

            let quantised = self.lsf.quantise_sid(ctx, &lsp);
            self.dtx.set_indices(
                Word16(i16::try_from(quantised.seed_index).expect("three bits")),
                quantised.indices.map(|i| Word16(i16::try_from(i).expect("nine bits"))),
            );

            let (ordinary, mr122) = self.dtx.predictor_reset(ctx);
            self.gains.reseed_predictors(ordinary, mr122);
        }

        for word in self.dtx.sid_parameters() {
            prms.push(u16::try_from(word.0).expect("a SID parameter is non-negative"));
        }

        let lsp_new = self.lsp_old;
        self.old_exc = [Word16(0); EXC_TOTAL];
        self.mem_w0 = [Word16(0); M];
        self.mem_err = [Word16(0); M];
        self.lsf = LsfQuantiser::new();
        self.lsp_old = lsp_new;
        self.lsp_old_q = lsp_new;
        self.pitch = ClosedLoopPitch::new();
        self.sharp = Word16(0);
    }

    /// `lsp()`: roots, interpolation of the unquantised spectrum, quantisation,
    /// interpolation of the quantised one.
    ///
    /// Writes the LSF indices into `prms` and returns `Aq_t`, Q12. `az` arrives
    /// with only the directly analysed slots filled and leaves complete.
    fn spectrum(
        &mut self,
        ctx: &mut DspContext,
        mode: u8,
        az: &mut [Word16; AZ_SIZE],
        prms: &mut Parameters,
        comfort_noise: bool,
    ) -> [Word16; AZ_SIZE] {
        if mode == MR122 {
            // Two analyses, and the second falls back on the first's roots
            // rather than on the previous frame's.
            let mid = az_lsp(ctx, slot(az, 1), &self.lsp_old);
            let new = az_lsp(ctx, slot(az, 3), &mid);

            // `Int_lpc_1and3_2`: subframes 1 and 3, i.e. slots 0 and 2. Slots 1
            // and 3 already hold the two direct analyses and must not be
            // replaced by an LSP round trip.
            let interpolated = interpolate_lsp_mid(ctx, &self.lsp_old, &mid, &new);
            copy_slot(az, 0, &interpolated);
            copy_slot(az, 2, &interpolated);

            // `if (used_mode != MRDTX)`: a comfort-noise frame transmits no
            // LSF indices and rebuilds no quantised filter -- the SID's own
            // spectrum replaces both, and `build_sid` overwrites `lsp_old_q`
            // moments later. The returned `azq` is never read on that path.
            if comfort_noise {
                self.lsp_old = new;
                return [Word16(0); AZ_SIZE];
            }

            let quantised = self.lsf.quantise_pair(ctx, &mid, &new);
            for index in quantised.indices {
                prms.push(index);
            }
            let azq = interpolate_lsp_mid(ctx, &self.lsp_old_q, &quantised.mid, &quantised.new);

            self.lsp_old = new;
            self.lsp_old_q = quantised.new;
            azq
        } else {
            let new = az_lsp(ctx, slot(az, 3), &self.lsp_old);

            // `Int_lpc_1to3_2`: subframes 1, 2 and 3, i.e. slots 0, 1 and 2.
            let interpolated = interpolate_lsp(ctx, &self.lsp_old, &new);
            copy_slot(az, 0, &interpolated);
            copy_slot(az, 1, &interpolated);
            copy_slot(az, 2, &interpolated);

            if comfort_noise {
                self.lsp_old = new;
                return [Word16(0); AZ_SIZE];
            }

            let quantised = self.lsf.quantise(ctx, mode, &new);
            for index in quantised.indices {
                prms.push(index);
            }
            let azq = interpolate_lsp(ctx, &self.lsp_old_q, &quantised.lsp);

            self.lsp_old = new;
            self.lsp_old_q = quantised.lsp;
            azq
        }
    }

    /// The subframe loop for the seven rates that code each subframe on its own.
    fn subframe_loop(
        &mut self,
        ctx: &mut DspContext,
        frame: &FrameState,
        exc: &mut [Word16; EXC_TOTAL],
        prms: &mut Parameters,
    ) {
        for subfr in 0..NB_SUBFR {
            let mem_w0 = self.mem_w0;
            let searched = self.search_subframe(ctx, frame, subfr, exc, prms, &mem_w0);
            let quantised = self.quantise_gains(ctx, frame.mode, subfr, &searched, prms, None);

            // Once per subframe in every mode, on whatever `gainQuant` left in
            // `gain_pit`.
            self.tone.update(ctx, quantised.gains.pitch);

            let mut memories = Memories {
                syn: self.mem_syn,
                err: self.mem_err,
                w0: self.mem_w0,
                sharp: self.sharp,
            };
            post_process(
                ctx,
                &PostProc {
                    mode: frame.mode,
                    subfr,
                    gain_pit: quantised.gains.pitch,
                    gain_code: quantised.gains.code,
                    aq: slot(&frame.azq, subfr),
                    speech: self.speech.coded(),
                    xn: &searched.xn,
                    code: &searched.code,
                    y1: &searched.y1,
                    y2: &searched.y2,
                },
                exc,
                &mut memories,
            );
            self.mem_syn = memories.syn;
            self.mem_err = memories.err;
            self.mem_w0 = memories.w0;
            self.sharp = memories.sharp;
        }
    }

    /// The 4.75 kbit/s subframe loop: two subframes at a time, with the even one
    /// redone once the pair's gains are known.
    fn subframe_loop_joint(
        &mut self,
        ctx: &mut DspContext,
        frame: &FrameState,
        exc: &mut [Word16; EXC_TOTAL],
        prms: &mut Parameters,
    ) {
        let mode = frame.mode;
        for pair in 0..NB_SUBFR / 2 {
            let even = pair * 2;
            let odd = even + 1;

            // What the pair rewinds to. `mem_err` is the only one of the three
            // memories the even subframe advances for real, so it is the only
            // one that has to be kept in order to be put back.
            let mem_err_save = self.mem_err;
            let sharp_save = self.sharp;
            // `mem_w0` at pair entry. The reference hands this same array to
            // both of the pair's `subframePreProc` calls; it holds the live
            // value now and the even subframe's provisional update later.
            let mut mem_w0_save = self.mem_w0;

            // --- even subframe ------------------------------------------------
            let even_search = self.search_subframe(ctx, frame, even, exc, prms, &mem_w0_save);
            // The impulse response as it was before the codebook search
            // sharpened it: the rebuilt excitation is re-convolved with *this*.
            let h1_sf0 = even_search.h1;

            let even_quantised = self.quantise_gains(ctx, mode, even, &even_search, prms, None);
            let gain_slot = even_quantised
                .reserved
                .expect("4.75 kbit/s reserves a gain slot on the even subframe");
            // The *unquantised* gain, deliberately: this call sits outside the
            // rate-specific branch of `cod_amr` and sees whatever `gainQuant`
            // left behind, which on this subframe is the closed loop's own.
            self.tone.update(ctx, even_quantised.gains.pitch);

            // The provisional synthesis. Its `syn` update is the reference's
            // `mem_syn_save`, which goes stale the moment the odd subframe
            // redoes this subframe and is never read again — so it is dropped
            // here rather than stored, and `st->mem_syn` stays where it was.
            let mut provisional = Memories {
                syn: self.mem_syn,
                err: self.mem_err,
                w0: mem_w0_save,
                sharp: self.sharp,
            };
            post_process(
                ctx,
                &PostProc {
                    mode,
                    subfr: even,
                    gain_pit: even_quantised.gains.pitch,
                    gain_code: even_quantised.gains.code,
                    aq: slot(&frame.azq, even),
                    speech: self.speech.coded(),
                    xn: &even_search.xn,
                    code: &even_search.code,
                    y1: &even_search.y1,
                    y2: &even_search.y2,
                },
                exc,
                &mut provisional,
            );
            mem_w0_save = provisional.w0;
            self.mem_err = provisional.err;
            // Restored, so both subframes of the pair search with the same
            // sharpening constant.
            self.sharp = sharp_save;

            // --- odd subframe -------------------------------------------------
            let odd_search = self.search_subframe(ctx, frame, odd, exc, prms, &mem_w0_save);
            let odd_quantised =
                self.quantise_gains(ctx, mode, odd, &odd_search, prms, Some(gain_slot));
            let even_gains = odd_quantised
                .previous
                .expect("the joint quantiser returns the even subframe's gains too");
            self.tone.update(ctx, odd_quantised.gains.pitch);

            // --- redo the even subframe with the pair's chosen gains -----------
            self.mem_err = mem_err_save;
            // Rebuild its adaptive excitation over the total excitation left
            // there provisionally, then refilter with the unsharpened response.
            predict_into(ctx, exc, even, even_search.lag);
            let rebuilt = subframe_of(exc, even);
            let y1 = convolve(ctx, &rebuilt, &h1_sf0);

            let mut memories = Memories {
                syn: self.mem_syn,
                err: self.mem_err,
                w0: self.mem_w0,
                // A throwaway sink: `st->sharp` is deliberately not advanced by
                // the even subframe of a pair.
                sharp: sharp_save,
            };
            post_process(
                ctx,
                &PostProc {
                    mode,
                    subfr: even,
                    gain_pit: even_gains.pitch,
                    gain_code: even_gains.code,
                    aq: slot(&frame.azq, even),
                    speech: self.speech.coded(),
                    xn: &even_search.xn,
                    code: &even_search.code,
                    y1: &y1,
                    y2: &even_search.y2,
                },
                exc,
                &mut memories,
            );
            self.mem_syn = memories.syn;
            self.mem_err = memories.err;
            self.mem_w0 = memories.w0;

            // --- redo the odd subframe's pre-processing -------------------------
            // The memories it originally saw were provisional. This second,
            // authoritative pass produces the `xn` the post-processing uses and
            // reconstructs the unsharpened `h1`.
            let targets = subframe_targets(
                ctx,
                mode,
                slot(&frame.az, odd),
                slot(&frame.azq, odd),
                self.speech.with_history(odd * L_SUBFR, L_SUBFR),
                &self.mem_err,
                &self.mem_w0,
            );
            // And its adaptive excitation: the history it interpolates from
            // moved when the even subframe was redone, which shows up whenever
            // the lag is shorter than a subframe.
            predict_into(ctx, exc, odd, odd_search.lag);
            let rebuilt = subframe_of(exc, odd);
            let y1 = convolve(ctx, &rebuilt, &targets.h1);

            let mut memories = Memories {
                syn: self.mem_syn,
                err: self.mem_err,
                w0: self.mem_w0,
                sharp: self.sharp,
            };
            post_process(
                ctx,
                &PostProc {
                    mode,
                    subfr: odd,
                    gain_pit: odd_quantised.gains.pitch,
                    gain_code: odd_quantised.gains.code,
                    aq: slot(&frame.azq, odd),
                    speech: self.speech.coded(),
                    xn: &targets.xn,
                    code: &odd_search.code,
                    y1: &y1,
                    y2: &odd_search.y2,
                },
                exc,
                &mut memories,
            );
            self.mem_syn = memories.syn;
            self.mem_err = memories.err;
            self.mem_w0 = memories.w0;
            self.sharp = memories.sharp;
        }
    }

    /// Everything one subframe decides before its gains: the target, the
    /// closed-loop lag, and the algebraic codevector.
    ///
    /// `mem_w0` is passed rather than read from `self` because 4.75 kbit/s hands
    /// both subframes of a pair the *saved* copy and not the live one.
    fn search_subframe(
        &mut self,
        ctx: &mut DspContext,
        frame: &FrameState,
        subfr: usize,
        exc: &mut [Word16; EXC_TOTAL],
        prms: &mut Parameters,
        mem_w0: &[Word16; M],
    ) -> Searched {
        let mode = frame.mode;
        let i_subfr = subfr * L_SUBFR;
        let targets: SubframeTargets = subframe_targets(
            ctx,
            mode,
            slot(&frame.az, subfr),
            slot(&frame.azq, subfr),
            self.speech.with_history(i_subfr, L_SUBFR),
            &self.mem_err,
            mem_w0,
        );
        // `Copy(res2, exc, L_SUBFR)`: the search reads the residual out of the
        // excitation buffer and then overwrites it in place.
        exc[EXC_ORIGIN + i_subfr..EXC_ORIGIN + i_subfr + L_SUBFR].copy_from_slice(&targets.res);

        let sf = subfr as i32;
        self.trc(sf, "lsp_new", &frame.lsp_new);
        self.trc(sf, "xn", &targets.xn);
        self.trc(sf, "h1", &targets.h1);
        self.trc(sf, "res", &targets.res);

        // `res` stays pristine for the gain quantiser; `res2` is the copy the
        // pitch search is allowed to destroy.
        let mut res2 = targets.res;
        let mut xn2 = [Word16(0); L_SUBFR];
        let mut y1 = [Word16(0); L_SUBFR];

        let mut view = view_at(exc, subfr);
        let ltp = closed_loop_ltp(
            ctx,
            mode,
            &mut self.pitch,
            &mut self.tone,
            frame.t_op,
            subfr,
            &mut view,
            &targets.xn,
            &targets.h1,
            frame.lsp_flag,
            &mut res2,
            &mut xn2,
            &mut y1,
        );
        commit_view(exc, subfr, &view);

        // The lag history the weighted open-loop search takes its median over.
        // Only 10.2 kbit/s ever raises these flags, so only 10.2 moves it.
        if subfr == 0 && self.ol_gain_flg[0] {
            self.old_lags[1] = ltp.pitch.lag.integer;
        }
        if subfr == 3 && self.ol_gain_flg[1] {
            self.old_lags[0] = ltp.pitch.lag.integer;
        }

        prms.push(ltp.pitch.index);
        if let Some(index) = ltp.gain_index {
            // 12.2 kbit/s alone quantises the pitch gain inside the pitch stage
            // and transmits it before the codevector.
            prms.push(index);
        }

        let adaptive = subframe_of(exc, subfr);
        self.trc1(sf, "T0", i32::from(ltp.pitch.lag.integer.0));
        self.trc1(sf, "T0_frac", i32::from(ltp.pitch.lag.frac.0));
        self.trc1(sf, "gain_pit_ol", i32::from(ltp.gain_pitch.0));
        self.trc(sf, "xn2", &xn2);
        self.trc(sf, "y1", &y1);
        self.trc(sf, "adapt", &adaptive);

        let innovation = codebook_search(
            ctx,
            mode,
            subfr as u8,
            &CodebookInputs {
                target: &xn2,
                impulse: &targets.h1,
                ltp_residual: &res2,
                lag: ltp.pitch.lag.integer.0,
                // Seven rates sharpen with the carried state; 12.2 sharpens
                // with `gain_pit`, and the stage picks between them by rate.
                pitch_sharp: self.sharp,
                gain_pit: ltp.gain_pitch,
            },
        );
        codebook_words(innovation.params, prms);

        self.trc(sf, "code", &innovation.code);
        self.trc(sf, "y2", &innovation.filtered);

        Searched {
            xn: targets.xn,
            h1: targets.h1,
            res: targets.res,
            xn2,
            y1,
            code: innovation.code,
            y2: innovation.filtered,
            adaptive,
            lag: ltp.pitch.lag,
            gain_pitch: ltp.gain_pitch,
            gain_limit: ltp.gain_limit,
            correlations: [
                ltp.gain_coefficients.yy,
                ltp.gain_coefficients.exp_yy,
                ltp.gain_coefficients.xy,
                ltp.gain_coefficients.exp_xy,
            ],
        }
    }

    /// `gainQuant`, plus the parameter words it emits.
    ///
    /// `pair_slot` is the slot 4.75's even subframe reserved, carried back in on
    /// the odd one.
    fn quantise_gains(
        &mut self,
        ctx: &mut DspContext,
        mode: u8,
        subfr: usize,
        searched: &Searched,
        prms: &mut Parameters,
        pair_slot: Option<usize>,
    ) -> Quantised {
        let decision = self.gains.quantise(
            ctx,
            mode,
            &SubframeSignals {
                residual: &searched.res,
                adaptive: &searched.adaptive,
                code: &searched.code,
                pitch_target: &searched.xn,
                code_target: &searched.xn2,
                filtered_adaptive: &searched.y1,
                filtered_code: &searched.y2,
                pitch_correlations: searched.correlations,
                gain_pit: searched.gain_pitch,
                gp_limit: searched.gain_limit,
                even_subframe: subfr.is_multiple_of(2),
            },
        );

        let mut reserved = None;
        match decision.params {
            GainParams::Reserve => reserved = Some(prms.reserve()),
            GainParams::Index(index) => prms.push(index),
            GainParams::Pair(index) => prms.fill(
                pair_slot.expect("4.75 kbit/s must carry the reserved slot to the odd subframe"),
                index,
            ),
            GainParams::PitchAndCode(pitch, code) => {
                prms.push(pitch);
                prms.push(code);
            }
        }

        let sf = subfr as i32;
        self.trc1(sf, "gain_pit", i32::from(decision.gains.pitch.0));
        self.trc1(sf, "gain_code", i32::from(decision.gains.code.0));

        Quantised {
            gains: Gains {
                pitch: decision.gains.pitch,
                code: decision.gains.code,
            },
            previous: decision.previous.map(|g| Gains {
                pitch: g.pitch,
                code: g.code,
            }),
            reserved,
        }
    }
}

/// Everything the subframe loop reads that the frame decided.
struct FrameState {
    mode: u8,
    /// `A_t`: the unquantised interpolated predictors, Q12.
    az: [Word16; AZ_SIZE],
    /// `Aq_t`: the quantised interpolated predictors, Q12.
    azq: [Word16; AZ_SIZE],
    /// The two half-frames' open-loop lags.
    t_op: [i16; 2],
    /// `check_lsp`'s once-per-frame resonance verdict.
    lsp_flag: bool,
    /// This frame's unquantised 4th-subframe LSPs, for the trace only.
    lsp_new: [Word16; M],
}

/// The two gains one subframe is synthesised with, Q14 and Q1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gains {
    pitch: Word16,
    code: Word16,
}

/// What one call to the gain quantiser decided, in this module's own terms.
struct Quantised {
    gains: Gains,
    /// 4.75 kbit/s, odd subframe: the now-quantised gains for the even subframe
    /// of the pair, which the caller must redo that subframe's synthesis with.
    previous: Option<Gains>,
    /// 4.75 kbit/s, even subframe: the parameter slot claimed for the pair.
    reserved: Option<usize>,
}

/// Everything one subframe's search produced that its post-processing — or, at
/// 4.75 kbit/s, the pair's rewind — needs afterwards.
#[derive(Clone, Copy, Debug)]
struct Searched {
    xn: [Word16; L_SUBFR],
    /// The **unsharpened** impulse response. 4.75 kbit/s keeps it to refilter
    /// the even subframe's rebuilt excitation.
    h1: [Word16; L_SUBFR],
    res: [Word16; L_SUBFR],
    xn2: [Word16; L_SUBFR],
    y1: [Word16; L_SUBFR],
    code: [Word16; L_SUBFR],
    y2: [Word16; L_SUBFR],
    adaptive: [Word16; L_SUBFR],
    lag: PitchLag,
    gain_pitch: Word16,
    gain_limit: Word16,
    correlations: [Word16; 4],
}

/// The four things `subframePostProc` advances.
#[derive(Clone, Copy, Debug)]
struct Memories {
    syn: [Word16; M],
    err: [Word16; M],
    w0: [Word16; M],
    sharp: Word16,
}

/// Everything `subframePostProc` reads.
struct PostProc<'a> {
    mode: u8,
    subfr: usize,
    gain_pit: Word16,
    gain_code: Word16,
    aq: &'a [Word16; MP1],
    /// Frame-base, indexed by `i_subfr + i`.
    speech: &'a [Word16; L_FRAME],
    xn: &'a [Word16; L_SUBFR],
    code: &'a [Word16; L_SUBFR],
    y1: &'a [Word16; L_SUBFR],
    y2: &'a [Word16; L_SUBFR],
}

// ---------------------------------------------------------------------------
// Excitation plumbing
// ---------------------------------------------------------------------------

/// The excitation window one call into the pitch stage sees.
///
/// That stage owns a fixed-length buffer whose subframe sits at a fixed offset,
/// which is exactly the reference's layout viewed from `&exc[i_subfr]`. Copying
/// the window in and the chosen subframe out leaves the stage untouched while
/// letting this module index the frame-long buffer directly — which 4.75 kbit/s
/// needs, since it rewrites a subframe it has already walked past.
fn view_at(exc: &[Word16; EXC_TOTAL], subfr: usize) -> Excitation {
    let at = subfr * L_SUBFR;
    let mut view = Excitation::new();
    view.all_mut().copy_from_slice(&exc[at..at + EXC_VIEW]);
    view
}

/// Write a window's chosen subframe back into the frame-long buffer.
fn commit_view(exc: &mut [Word16; EXC_TOTAL], subfr: usize, view: &Excitation) {
    let at = EXC_ORIGIN + subfr * L_SUBFR;
    exc[at..at + L_SUBFR].copy_from_slice(view.subframe());
}

/// `Pred_lt_3or6` applied to a subframe of the frame-long buffer.
///
/// Reads the history in front of the subframe and overwrites the subframe, so it
/// is what rebuilds 4.75's even subframe once the pair's gains are known.
fn predict_into(ctx: &mut DspContext, exc: &mut [Word16; EXC_TOTAL], subfr: usize, lag: PitchLag) {
    let mut view = view_at(exc, subfr);
    view.predict(ctx, lag);
    commit_view(exc, subfr, &view);
}

/// One subframe of the excitation, by value.
fn subframe_of(exc: &[Word16; EXC_TOTAL], subfr: usize) -> [Word16; L_SUBFR] {
    let at = EXC_ORIGIN + subfr * L_SUBFR;
    exc[at..at + L_SUBFR]
        .try_into()
        .expect("a subframe is L_SUBFR long")
}

/// One subframe's block of an `AZ_SIZE` predictor array.
fn slot(az: &[Word16; AZ_SIZE], subfr: usize) -> &[Word16; MP1] {
    az[subfr * MP1..(subfr + 1) * MP1]
        .try_into()
        .expect("AZ_SIZE is four MP1 blocks")
}

/// Copy one `MP1` block from an interpolation result into `az`.
fn copy_slot(az: &mut [Word16; AZ_SIZE], subfr: usize, from: &[Word16; AZ_SIZE]) {
    let range = subfr * MP1..(subfr + 1) * MP1;
    az[range.clone()].copy_from_slice(&from[range]);
}

// ---------------------------------------------------------------------------
// Local synthesis
// ---------------------------------------------------------------------------

/// `subframePostProc`: form the total excitation, synthesise it, and leave
/// behind the two memories the next subframe's target is built from.
///
/// `speech` and `exc` are frame-base and indexed by `i_subfr + i`; `xn`, `code`,
/// `y1` and `y2` are subframe-local. Mixing the two up produces plausible
/// speech and a non-conformant bitstream.
fn post_process(
    ctx: &mut DspContext,
    inputs: &PostProc<'_>,
    exc: &mut [Word16; EXC_TOTAL],
    memories: &mut Memories,
) {
    let &PostProc {
        mode,
        subfr,
        gain_pit,
        gain_code,
        aq,
        speech,
        xn,
        code,
        y1,
        y2,
    } = inputs;

    // 12.2 kbit/s carries `code` in Q12 and `y2` in Q10 rather than Q13 and
    // Q12, which is the whole of the difference between the two branches.
    let (temp_shift, k_shift, pitch_fac) = if mode == MR122 {
        (2, 4, shr(ctx, gain_pit, 1))
    } else {
        (1, 2, gain_pit)
    };

    // Clamped above only; `gain_pit` is never negative.
    memories.sharp = if sub(ctx, gain_pit, Word16(SHARPMAX)).0 > 0 {
        Word16(SHARPMAX)
    } else {
        gain_pit
    };

    let i_subfr = subfr * L_SUBFR;
    let base = EXC_ORIGIN + i_subfr;
    for i in 0..L_SUBFR {
        // In-place read-then-write: the adaptive codevector is replaced by the
        // total excitation, which is what every later subframe predicts from.
        let mut acc = l_mult(ctx, exc[base + i], pitch_fac);
        acc = l_mac(ctx, acc, code[i], gain_code);
        acc = l_shl(ctx, acc, temp_shift);
        exc[base + i] = round(ctx, acc);
    }

    let total: [Word16; L_SUBFR] = exc[base..base + L_SUBFR]
        .try_into()
        .expect("a subframe is L_SUBFR long");
    let mut synth = [Word16(0); L_SUBFR];
    memories.syn = synthesis_filter(ctx, aq, &total, &mut synth, &memories.syn);

    for (j, i) in (L_SUBFR - M..L_SUBFR).enumerate() {
        memories.err[j] = sub(ctx, speech[i_subfr + i], synth[i]);
        // `extract_h`, truncation — not `round`. Both shifts saturate, and the
        // `y1` one is a hard-coded 1 in every mode; only `y2`'s depends on the
        // rate, because only `y2`'s Q-format does.
        let scaled = l_mult(ctx, y1[i], gain_pit);
        let temp = extract_h(l_shl(ctx, scaled, 1));
        let scaled = l_mult(ctx, y2[i], gain_code);
        let k = extract_h(l_shl(ctx, scaled, k_shift));
        let both = add(ctx, temp, k);
        memories.w0[j] = sub(ctx, xn[i], both);
    }
}

// A compile-time restatement of the buffer geometry the reference gets from
// pointer arithmetic. `L_INTERPOL` is otherwise unused here, and the assertion
// is the reason it is imported: `EXC_ORIGIN` and `WSP_TOTAL` are the two places
// a transcription can silently lose the interpolation filter's reach.
const _: () = {
    assert!(EXC_ORIGIN == PIT_MAX as usize + L_INTERPOL);
    assert!(EXC_TOTAL == EXC_ORIGIN + L_FRAME);
    assert!(EXC_VIEW == EXC_ORIGIN + L_SUBFR);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::nb::bitstream::parse;

    /// The reference encoder's own per-stage trace, three frames at 7.40
    /// kbit/s.
    const TRACE: &str = include_str!("../../testdata/nb_enc_trace.txt");

    /// The 8 kHz input both the trace and the bitstreams were produced from.
    const INPUT: &[u8] = include_bytes!("../../testdata/amrnb_enc_input.pcm");

    /// The reference encoder's output at each of the eight rates.
    const BITSTREAMS: [&[u8]; 8] = [
        include_bytes!("../../testdata/amrnb_enc_mode0.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode1.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode2.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode3.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode4.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode5.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode6.amr"),
        include_bytes!("../../testdata/amrnb_enc_mode7.amr"),
    ];

    /// Frames in the committed bitstreams.
    const FRAMES: usize = 50;
    /// Frames the committed trace covers.
    const TRACED_FRAMES: usize = 3;
    /// The rate the trace was produced at.
    const TRACE_MODE: u8 = 4;
    /// The storage format's magic number, `#!AMR\n`.
    const MAGIC: usize = 6;

    fn input_frame(frame: usize) -> [i16; L_FRAME] {
        let mut samples = [0i16; L_FRAME];
        let base = frame * L_FRAME * 2;
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
        let mut encoder = NbEncoder::new();
        encoder.record_trace();
        let mut payloads = Vec::with_capacity(frames);
        for frame in 0..frames {
            payloads.push(encoder.encode_frame(&input_frame(frame), rate));
        }
        let trace = encoder.take_trace().expect("recording was enabled");
        (payloads, trace)
    }

    #[test]
    fn every_traced_intermediate_is_bit_exact_against_ts26073() {
        // The whole point of the fixture: a divergence is located by the first
        // row that differs, not by staring at the bitstream. The comparison
        // count is asserted because a harness that silently compares nothing
        // reads exactly like one that agrees.
        let (_, got) = run(TRACE_MODE, TRACED_FRAMES);
        let want = reference_rows();

        let mut compared_rows = 0usize;
        let mut compared_values = 0usize;
        for (frame, subframe, name, expected) in &want {
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
                    "frame {frame} subframe {subframe}: {name}[{i}] = {a} but TS 26.073 gives {e}"
                );
                compared_values += 1;
            }
            compared_rows += 1;
        }

        assert_eq!(compared_rows, 174, "the committed trace has 174 rows");
        assert_eq!(
            compared_values, 4632,
            "every traced value must have been compared"
        );
    }

    #[test]
    fn the_bitstream_is_byte_identical_to_ts26073_at_every_rate() {
        // The deliverable. Eight rates, fifty frames each, against the output
        // of the normative encoder driven from the same PCM.
        let mut exact = Vec::new();
        let mut failures = Vec::new();

        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let file = BITSTREAMS[mode as usize];
            assert_eq!(&file[..MAGIC], b"#!AMR\n", "mode {mode}: magic number");

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
                let (frame, byte) = first.expect("a first difference");
                failures.push(format!(
                    "mode {mode}: {wrong}/{FRAMES} frames differ, first at frame {frame} byte {byte}"
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
    fn a_one_lsb_change_to_the_input_moves_the_bitstream() {
        // Without this the comparison above could be vacuous — a harness that
        // encodes the wrong thing, or compares the fixture with itself, passes
        // just as green. One LSB *of the coded signal* is 8, not 1: the encoder
        // masks the low three bits off every input sample before anything else
        // touches it, so a change of 1 is genuinely invisible and would make a
        // weaker version of this test pass for the wrong reason.
        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let baseline = {
                let mut encoder = NbEncoder::new();
                (0..FRAMES)
                    .map(|f| encoder.encode_frame(&input_frame(f), rate))
                    .collect::<Vec<_>>()
            };
            let nudged = {
                let mut encoder = NbEncoder::new();
                (0..FRAMES)
                    .map(|f| {
                        let mut pcm = input_frame(f);
                        if f == 6 {
                            pcm[100] = pcm[100].wrapping_add(8);
                        }
                        encoder.encode_frame(&pcm, rate)
                    })
                    .collect::<Vec<_>>()
            };
            assert_ne!(
                baseline, nudged,
                "mode {mode}: a one-LSB change to sample 1060 left the bitstream \
                 unchanged, so the byte-exactness test proves nothing"
            );
        }
    }

    #[test]
    fn the_parameter_writer_fills_the_frame_exactly() {
        // A layout that overruns or underruns its frame does not fail loudly:
        // it shifts every later field and yields plausible parameters. The
        // defence is conservation, and `pack` asserts it — this drives every
        // rate through it, including 4.75's reserve-and-backfill.
        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = NbEncoder::new();
            let mut prms = Parameters::new();
            encoder.encode_into(&input_frame(0), rate, &mut prms);
            assert_eq!(
                prms.words.len(),
                rate.widths().len(),
                "mode {mode}: parameter count"
            );
            let packed = prms.pack(rate);
            assert_eq!(
                packed.len(),
                rate.packed_bytes(),
                "mode {mode}: payload size"
            );
        }
    }

    #[test]
    fn the_payload_round_trips_through_the_decoder_s_own_unpacking() {
        // Packing is meant to be the inverse of `bitstream::parse`. Saying so
        // out loud is cheap and catches a permutation applied in the wrong
        // direction, which produces a payload of exactly the right length.
        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = NbEncoder::new();
            let mut prms = Parameters::new();
            encoder.encode_into(&input_frame(0), rate, &mut prms);
            let packed = prms.pack(rate);
            let recovered = parse(mode, &packed).expect("the payload parses");
            assert_eq!(
                recovered, prms.words,
                "mode {mode}: packing is not the inverse of unpacking"
            );
        }
    }

    #[test]
    fn degenerate_input_produces_a_frame_rather_than_a_panic() {
        // Fixed-point division here carries preconditions — `div_s` wants a
        // proper fraction, `norm_l` an argument it can normalise — and several
        // call sites reach them through a chain of energies and
        // normalisations. Silence starves those chains and full-scale square
        // waves saturate them; both run at every rate, so 4.75's rewind and
        // 12.2's two analyses are covered.
        //
        // Nothing stronger is asserted about the *values*. Silence is not a
        // "nothing to send" case for an analysis-by-synthesis coder — it still
        // places pulses and still quantises a gain, and what the reference
        // makes of that is rate-dependent — so any claim beyond "a frame comes
        // out, of the right size" would be a guess dressed as a test.
        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut encoder = NbEncoder::new();
            for frame in 0..8 {
                let mut pcm = [0i16; L_FRAME];
                for (i, slot) in pcm.iter_mut().enumerate() {
                    *slot = match frame % 4 {
                        0 => 0,
                        1 => i16::MIN,
                        2 => {
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
    fn the_encoder_carries_state_across_frames_and_repeats_itself() {
        // Two failures this catches, both of which leave the output plausible:
        // a memory cleared at the frame boundary, which makes every frame
        // independent; and a hidden global, which makes a second run of the
        // same input differ from the first.
        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let stream: Vec<Vec<u8>> = {
                let mut encoder = NbEncoder::new();
                (0..6)
                    .map(|f| encoder.encode_frame(&input_frame(f), rate))
                    .collect()
            };
            let again: Vec<Vec<u8>> = {
                let mut encoder = NbEncoder::new();
                (0..6)
                    .map(|f| encoder.encode_frame(&input_frame(f), rate))
                    .collect()
            };
            assert_eq!(stream, again, "mode {mode}: encoding is not deterministic");

            let alone = NbEncoder::new().encode_frame(&input_frame(4), rate);
            assert_ne!(
                alone, stream[4],
                "mode {mode}: a cold encoder reproduced frame 4 of the stream, so \
                 nothing is being carried between frames"
            );
        }
    }

    /// The committed trace is three frames at one rate. This is the same
    /// comparison against **fifty** frames at **all eight**, which is what
    /// actually established that the byte-exactness above is not a coincidence
    /// of a short fixture: 617,600 intermediates rather than 4,632.
    ///
    /// Ignored because it needs traces regenerated from the reference tree:
    ///
    /// ```text
    /// for m in 0 1 2 3 4 5 6 7; do
    ///   tools/trace-amrnb-encoder.sh $m "$DIR/nbtrace$m"
    /// done
    /// RVOIP_NB_TRACE_DIR=$DIR cargo test -p rvoip-codec-core --all-features \
    ///     all_rates_all_frames -- --ignored --nocapture
    /// ```
    ///
    /// The script itself asserts that its instrumented build still reproduces
    /// the committed bitstream, so a trace point that changed behaviour rather
    /// than observing it fails there rather than quietly moving the target.
    #[test]
    #[ignore = "needs RVOIP_NB_TRACE_DIR; see the doc comment"]
    fn all_rates_all_frames_against_regenerated_traces() {
        let dir = std::env::var("RVOIP_NB_TRACE_DIR")
            .expect("set RVOIP_NB_TRACE_DIR to the directory holding nbtrace0..7");
        println!();
        for mode in 0..8u8 {
            let path = format!("{dir}/nbtrace{mode}/trace.txt");
            let text = std::fs::read_to_string(&path).expect("a regenerated trace");
            let (_, got) = run(mode, FRAMES);

            let mut rows = 0usize;
            let mut values = 0usize;
            for line in text.lines() {
                let mut field = line.split_whitespace();
                if field.next() != Some("T") {
                    continue;
                }
                let frame: usize = field.next().expect("frame").parse().expect("frame");
                let subframe: i32 = field.next().expect("subframe").parse().expect("subframe");
                let name = field.next().expect("name").to_owned();
                let expected: Vec<i32> = field.map(|v| v.parse().expect("value")).collect();
                let actual = got.row(frame, subframe, &name).unwrap_or_else(|| {
                    panic!("mode {mode} frame {frame}/{subframe}: never produced {name}")
                });
                assert_eq!(actual.len(), expected.len(), "{name} length");
                for (i, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
                    assert_eq!(
                        a, e,
                        "mode {mode} frame {frame} subframe {subframe}: \
                         {name}[{i}] = {a} but TS 26.073 gives {e}"
                    );
                    values += 1;
                }
                rows += 1;
            }
            // A trace that parsed to nothing reads exactly like one that agrees.
            assert!(rows > 2800, "mode {mode}: only {rows} rows compared");
            println!("mode {mode}: {rows} rows, {values} values, all match");
        }
    }

    /// Independent end-to-end check, written outside the assembly's own tests:
    /// rebuild the storage file and compare it against the reference on disk,
    /// byte for byte, magic and table of contents included.
    #[test]
    #[ignore = "independent verification"]
    fn independently_reproduce_the_reference_files() {
        let pcm: Vec<i16> = include_bytes!("../../testdata/amrnb_enc_input.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(pcm.len(), 50 * 160);
        println!();

        let refs: [&[u8]; 8] = [
            include_bytes!("../../testdata/amrnb_enc_mode0.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode1.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode2.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode3.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode4.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode5.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode6.amr"),
            include_bytes!("../../testdata/amrnb_enc_mode7.amr"),
        ];

        for mode in 0..8u8 {
            let rate = Rate::from_index(mode).expect("a speech mode");
            let mut enc = NbEncoder::new();
            let mut out: Vec<u8> = b"#!AMR\n".to_vec();
            for frame in pcm.chunks_exact(160) {
                let mut block = [0i16; 160];
                block.copy_from_slice(frame);
                out.push((mode << 3) | 0x04);
                out.extend_from_slice(&enc.encode_frame(&block, rate));
            }
            let want = refs[usize::from(mode)];
            assert_eq!(out.len(), want.len(), "mode {mode}: file length");
            let first = out.iter().zip(want).position(|(a, b)| a != b);
            assert!(first.is_none(), "mode {mode}: first differs at byte {}", first.unwrap());

            // Not vacuous. The perturbation is deliberately larger than the
            // one LSB the coded signal can resolve: the encoder masks the low
            // three bits off every sample, and beyond that a *single* sample
            // moved by 8 is genuinely invisible to the coarsest quantiser at
            // some positions — a first version of this check moved sample 1000
            // by 8 and 4.75 kbit/s produced an identical bitstream, which is
            // quantisation doing its job rather than a defect. A change this
            // size no rate can miss keeps the check about the harness rather
            // than about quantiser sensitivity.
            let mut nudged = pcm.clone();
            nudged[1000] = nudged[1000].wrapping_add(1024);
            let mut enc = NbEncoder::new();
            let mut other: Vec<u8> = b"#!AMR\n".to_vec();
            for frame in nudged.chunks_exact(160) {
                let mut block = [0i16; 160];
                block.copy_from_slice(frame);
                other.push((mode << 3) | 0x04);
                other.extend_from_slice(&enc.encode_frame(&block, rate));
            }
            assert_ne!(other, out, "mode {mode}: the input change left the bitstream alone");
            println!("mode {mode}: {} bytes byte-identical", out.len());
        }
    }
}
