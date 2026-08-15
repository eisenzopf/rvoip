//! Narrowband comfort noise, TS 26.073 `dtx_dec.c`.
//!
//! # What this is and is not shared with
//!
//! Nothing here is shared with [`super::super::wb::dtx`], and the resemblance
//! is a trap. The two state machines have the same *shape* — a three-state
//! automaton over the same frame types, a ring of spectral and energy history,
//! an interpolation between the last two SIDs — and different arithmetic
//! almost everywhere inside it. Narrowband works in LSFs where wideband works
//! in ISFs; its history is eight frames deep against wideband's; it carries a
//! per-mode level adjustment that wideband has no equivalent of; its
//! interpolation is Q14 against Q15; and its comfort noise is shaped by a
//! *prediction gain* correction that wideband does not compute at all.
//!
//! # The three states, and which transitions are reachable
//!
//! `SPEECH`, `DTX` and `DTX_MUTE`. The reference gives the table in a comment
//! above `rx_dtx_handler` and it is worth reading as the specification it is:
//! the interesting entries are the ones where the *incoming frame type is not
//! enough*. `RX_NO_DATA` means SPEECH when the previous state was SPEECH — a
//! gap in a talk spurt is a lost frame — and means DTX when it was DTX, where
//! the same gap is the encoder deliberately saying nothing.
//!
//! `DTX_MUTE` is the "the noise description is too old" state, reached after
//! [`DTX_MAX_EMPTY_THRESH`] frames without an update, and it is sticky: from
//! `DTX_MUTE` only a genuine `SID_UPDATE` or real speech gets out.
//!
//! # The counter that must not start at zero
//!
//! `decAnaElapsedCount` resets to **32767**, not 0. It counts frames since the
//! decoder last believed the encoder had added its DTX hangover, and starting
//! it at zero collapses the seven-frame hangover on the first silence of every
//! stream — a divergence that appears seven frames after the first SID and
//! looks like a backward-analysis bug rather than an initialisation one.

use super::cn::{a_refl, build_cn_code, pseudonoise, PN_INITIAL_SEED};
use super::decoder_tables::{LSP_INIT, MEAN_LSF_3};
use super::enc::dtx::DTX_HIST_SIZE;
use super::enc::lsp_quant::lsp_to_lsf;
use super::gain::CodeGainPredictor;
use super::lsp::{lsf_to_lsp, lsp_to_lp, reorder_lsf, LsfDecoder, M, MP1};
use super::math::{log2, pow2};
use super::synthesis::synthesis_filter;
use super::L_FRAME;
use crate::fixed_point::arith::{abs_s, add, extract_h, extract_l, mult, sub};
use crate::fixed_point::arith32::{l_add, l_deposit_h, l_deposit_l, l_mac, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::{l_shl, l_shr, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Subframe length, and there are four to a frame.
const L_SUBFR: usize = 40;

/// The minimum spacing `Reorder_lsf` enforces, Q15.
const LSF_GAP: Word16 = Word16(205);

/// `DTX_HANG_CONST`: how many frames of hangover the encoder is assumed to add.
const DTX_HANG_CONST: Word16 = Word16(7);

/// `DTX_ELAPSED_FRAMES_THRESH`: `24 + DTX_HANG_CONST - 1`.
const DTX_ELAPSED_FRAMES_THRESH: Word16 = Word16(24 + 7 - 1);

/// How long the noise description may go unrefreshed before muting, in frames.
pub const DTX_MAX_EMPTY_THRESH: Word16 = Word16(50);

/// How far the LSF history is pulled toward its own mean, per coefficient.
///
/// `lsf_hist_mean_scale`. Zero for the top two coefficients: the highest LSFs
/// carry the least perceptually and vary the most, so the variability injected
/// into comfort noise deliberately excludes them.
const LSF_HIST_MEAN_SCALE: [Word16; M] = [
    Word16(20000),
    Word16(20000),
    Word16(20000),
    Word16(20000),
    Word16(20000),
    Word16(18000),
    Word16(16384),
    Word16(8192),
    Word16(0),
    Word16(0),
];

/// Per-mode level adjustment, Q11: `dtx_log_en_adjust`.
///
/// Indexed by mode, with the ninth entry for the SID frame itself. Comfort
/// noise is generated at a level the *speech* coder mode would have produced,
/// because a listener switching between a talk spurt at 4.75 and the silence
/// after it would otherwise hear the background jump.
const DTX_LOG_EN_ADJUST: [Word16; 9] = [
    Word16(-1023),
    Word16(-878),
    Word16(-732),
    Word16(-586),
    Word16(-440),
    Word16(-294),
    Word16(-148),
    Word16(0),
    Word16(0),
];

/// How a frame arrived, as the receiver classifies it — `RXFrameType`.
///
/// This is the decoder's own vocabulary, not RFC 4867's. The mapping from a
/// payload's table of contents to one of these is the caller's job, and the
/// two that are easy to conflate are [`SidBad`](Self::SidBad) — a SID whose
/// bits are damaged, which still means "be quiet" — and
/// [`SpeechBad`](Self::SpeechBad), which does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxFrameType {
    /// Speech, intact.
    SpeechGood,
    /// Speech, flagged as degraded but decodable.
    SpeechDegraded,
    /// Speech, damaged.
    SpeechBad,
    /// The first SID of a silence: no parameters, only the transition.
    SidFirst,
    /// A SID carrying a new noise description.
    SidUpdate,
    /// A SID whose bits are damaged.
    SidBad,
    /// Nothing arrived.
    NoData,
    /// The encoder signalled speech onset during a silence.
    Onset,
}

/// The synthesis state — `DTXStateType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtxState {
    /// Decode the frame normally.
    Speech,
    /// Generate comfort noise from the stored description.
    Dtx,
    /// Generate comfort noise, fading, because the description is stale.
    DtxMute,
}

/// What one comfort-noise frame produced.
pub struct ComfortNoise {
    /// 160 synthesised samples.
    pub synth: [Word16; L_FRAME],
    /// The LP coefficients, the same set repeated for all four subframes.
    ///
    /// Repeated deliberately: the post-filter runs per subframe and needs a
    /// filter for each, and comfort noise has only one. Note these are the
    /// coefficients from the *un-perturbed* LSPs — the synthesis filter uses a
    /// different set, and the reference is explicit about why. Using the
    /// perturbed ones for the post-filter makes the high band jump about.
    pub a_t: [Word16; 4 * MP1],
    /// The interpolated LSFs, which the caller copies into the LSF decoder.
    pub lsf: [Word16; M],
}

/// The decode-side DTX state, `dtx_decState`.
///
/// The four flags are separate fields rather than one packed value because
/// they are separate fields in the reference and are read independently by
/// four different tests; grouping them would make the transcription harder to
/// check against the C, which is the only thing keeping it correct.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct DtxDecoder {
    since_last_sid: Word16,
    true_sid_period_inv: Word16,
    log_en: Word16,
    old_log_en: Word16,
    pn_seed_rx: Word32,
    lsp: [Word16; M],
    lsp_old: [Word16; M],

    lsf_hist: [Word16; M * DTX_HIST_SIZE],
    lsf_hist_ptr: usize,
    lsf_hist_mean: [Word16; M * DTX_HIST_SIZE],
    log_pg_mean: Word16,
    log_en_hist: [Word16; DTX_HIST_SIZE],
    log_en_hist_ptr: usize,

    log_en_adjust: Word16,

    dtx_hangover_count: Word16,
    dec_ana_elapsed_count: Word16,

    sid_frame: bool,
    valid_data: bool,
    dtx_hangover_added: bool,

    /// The state the *previous* frame was synthesised in.
    global_state: DtxState,
    /// Whether a noise description has ever been received.
    data_updated: bool,
}

impl Default for DtxDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DtxDecoder {
    /// The state `dtx_dec_reset` leaves behind.
    ///
    /// Two initialisers are load-bearing and neither is the obvious value.
    /// `log_en` starts at 3500 rather than at silence — the reference's comment
    /// says "low level noise for better performance in DTX handover cases", and
    /// starting at zero makes a stream that opens in silence emit nothing at
    /// all until its first SID. And `dec_ana_elapsed_count` starts at 32767;
    /// see the module header.
    #[must_use]
    pub fn new() -> Self {
        let mean = MEAN_LSF_3.map(Word16);
        let mut lsf_hist = [Word16(0); M * DTX_HIST_SIZE];
        for frame in 0..DTX_HIST_SIZE {
            lsf_hist[frame * M..(frame + 1) * M].copy_from_slice(&mean);
        }

        Self {
            since_last_sid: Word16(0),
            true_sid_period_inv: Word16(1 << 13),
            log_en: Word16(3500),
            old_log_en: Word16(3500),
            pn_seed_rx: PN_INITIAL_SEED,
            lsp: LSP_INIT.map(Word16),
            lsp_old: LSP_INIT.map(Word16),
            lsf_hist,
            lsf_hist_ptr: 0,
            lsf_hist_mean: [Word16(0); M * DTX_HIST_SIZE],
            log_pg_mean: Word16(0),
            log_en_hist: [Word16(3500); DTX_HIST_SIZE],
            log_en_hist_ptr: 0,
            log_en_adjust: Word16(0),
            dtx_hangover_count: DTX_HANG_CONST,
            dec_ana_elapsed_count: Word16(32767),
            sid_frame: false,
            valid_data: false,
            dtx_hangover_added: false,
            global_state: DtxState::Dtx,
            data_updated: false,
        }
    }

    /// The state the previous frame was synthesised in.
    #[must_use]
    pub const fn global_state(&self) -> DtxState {
        self.global_state
    }

    /// Record the state this frame was synthesised in — the last thing the
    /// frame loop does, on every path including the speech one.
    /// Record the state this frame was synthesised in.
    pub const fn commit(&mut self, state: DtxState) {
        self.global_state = state;
    }

    /// `rx_dtx_handler`: classify the frame and advance the hangover
    /// bookkeeping.
    ///
    /// Called for *every* frame, speech included, and its side effects on
    /// `since_last_sid`, `dec_ana_elapsed_count` and `dtx_hangover_added`
    /// happen whether or not comfort noise follows. Calling it only when the
    /// decoder expects silence loses the hangover synchronisation.
    pub fn receive(&mut self, ctx: &mut DspContext, frame_type: RxFrameType) -> DtxState {
        use RxFrameType as F;

        let in_dtx = matches!(self.global_state, DtxState::Dtx | DtxState::DtxMute);
        let quiet_continuation =
            in_dtx && matches!(frame_type, F::NoData | F::SpeechBad | F::Onset);

        let new_state =
            if matches!(frame_type, F::SidFirst | F::SidUpdate | F::SidBad) || quiet_continuation {
                // Muting is sticky: these four inputs carry no new description, so
                // a decoder already muting stays muted.
                let sticky = self.global_state == DtxState::DtxMute
                    && matches!(frame_type, F::SidBad | F::SidFirst | F::Onset | F::NoData);
                let mut state = if sticky {
                    DtxState::DtxMute
                } else {
                    DtxState::Dtx
                };

                self.since_last_sid = add(ctx, self.since_last_sid, Word16(1));

                // A SID_UPDATE is exempt even when the counter has already passed
                // the threshold: the counter is incremented before this test, so a
                // late but genuine update would otherwise be punished for its own
                // lateness.
                if frame_type != F::SidUpdate
                    && sub(ctx, self.since_last_sid, DTX_MAX_EMPTY_THRESH).0 > 0
                {
                    state = DtxState::DtxMute;
                }
                state
            } else {
                self.since_last_sid = Word16(0);
                DtxState::Speech
            };

        // First ever description: resynchronise the hangover counter, which
        // may be arbitrarily stale after a handover.
        if !self.data_updated && frame_type == F::SidUpdate {
            self.dec_ana_elapsed_count = Word16(0);
        }

        self.dec_ana_elapsed_count = add(ctx, self.dec_ana_elapsed_count, Word16(1));
        self.dtx_hangover_added = false;

        // What the *encoder* was most likely doing, which is not the same
        // question as what this decoder should do.
        // A NO_DATA that this decoder reads as speech is a lost speech packet,
        // so the encoder was speaking after all.
        let enc_speaking = !matches!(
            frame_type,
            F::SidFirst | F::SidUpdate | F::SidBad | F::Onset | F::NoData
        ) || (frame_type == F::NoData && new_state == DtxState::Speech);

        if enc_speaking {
            self.dtx_hangover_count = DTX_HANG_CONST;
        } else if sub(ctx, self.dec_ana_elapsed_count, DTX_ELAPSED_FRAMES_THRESH).0 > 0 {
            self.dtx_hangover_added = true;
            self.dec_ana_elapsed_count = Word16(0);
            self.dtx_hangover_count = Word16(0);
        } else if self.dtx_hangover_count.0 == 0 {
            self.dec_ana_elapsed_count = Word16(0);
        } else {
            self.dtx_hangover_count = sub(ctx, self.dtx_hangover_count, Word16(1));
        }

        if new_state != DtxState::Speech {
            self.sid_frame = false;
            self.valid_data = false;

            match frame_type {
                // A SID_FIRST carries no parameters. It still counts as a SID
                // frame, because the backward analysis runs on it -- that is
                // where the description comes from when the encoder has added
                // hangover.
                F::SidFirst => self.sid_frame = true,
                F::SidUpdate => {
                    self.sid_frame = true;
                    self.valid_data = true;
                }
                F::SidBad => {
                    self.sid_frame = true;
                    // Damaged bits: keep the old description rather than
                    // re-deriving one from a history the sender did not intend.
                    self.dtx_hangover_added = false;
                }
                _ => {}
            }
        }

        new_state
    }

    /// `dtx_dec_activity_update`: fold a decoded speech frame into the history.
    ///
    /// Runs on every speech frame, and it is what the backward analysis
    /// averages when a `SID_FIRST` arrives with no parameters of its own. The
    /// energy is the *synthesised output's*, not the excitation's.
    pub fn observe_speech(&mut self, ctx: &mut DspContext, lsf: &[Word16; M], frame: &[Word16]) {
        self.lsf_hist_ptr = (self.lsf_hist_ptr + M) % (M * DTX_HIST_SIZE);
        self.lsf_hist[self.lsf_hist_ptr..self.lsf_hist_ptr + M].copy_from_slice(lsf);

        let mut frame_energy = Word32(0);
        for &sample in frame.iter().take(L_FRAME) {
            frame_energy = l_mac(ctx, frame_energy, sample, sample);
        }
        let (exponent, mantissa) = log2(ctx, frame_energy);

        let mut log_en = shl(ctx, exponent, 10);
        let fraction = shr(ctx, mantissa, 15 - 10);
        log_en = add(ctx, log_en, fraction);
        // Divide by L_FRAME: log2(160) = 7.32193, which is 7497 in Q10 plus
        // one whole unit. Written as `7497 + 1024` in the reference and kept
        // that way, because the split is what makes the constant checkable.
        log_en = sub(ctx, log_en, Word16(7497 + 1024));

        self.log_en_hist_ptr = (self.log_en_hist_ptr + 1) % DTX_HIST_SIZE;
        self.log_en_hist[self.log_en_hist_ptr] = log_en;
    }

    /// `dtx_dec`: synthesise one frame of comfort noise.
    ///
    /// `parm` is the SID frame's five parameters and is read only when this
    /// frame is a `SID_UPDATE`; the other paths interpolate from what is
    /// already stored. `mode` indexes [`DTX_LOG_EN_ADJUST`] — for a SID it is
    /// 8, the SID's own entry.
    ///
    /// The caller must have reset the speech decoder first, the way
    /// `Decoder_amr_reset(st, MRDTX)` does: excitation, sharpening, lag, bad-frame
    /// memories and the pitch-gain history all go, while `mem_syn`, `lsp_old`,
    /// the excitation energy history, the LSP average and the gain predictor
    /// stay. This function writes into `mem_syn` and the two states it is
    /// handed, and expects the rest already cleared.
    ///
    /// # Panics
    /// If `mode` is above 8.
    #[allow(
        clippy::too_many_lines,
        clippy::similar_names,
        clippy::too_many_arguments
    )]
    pub fn comfort_noise(
        &mut self,
        ctx: &mut DspContext,
        new_state: DtxState,
        mode: usize,
        parm: &[u16],
        mem_syn: &mut [Word16; M],
        lsf_state: &mut LsfDecoder,
        predictor: &mut CodeGainPredictor,
    ) -> ComfortNoise {
        assert!(mode <= 8, "mode {mode} has no level adjustment");

        if self.dtx_hangover_added && self.sid_frame {
            self.backward_analysis(ctx, mode);
        }

        if self.sid_frame {
            // Shift even when there is no new data: the interpolation's left
            // endpoint is always the previous description, and a SID_FIRST
            // that did not shift would interpolate a frame against itself.
            self.lsp_old = self.lsp;
            self.old_log_en = self.log_en;

            if self.valid_data {
                self.absorb_sid(ctx, parm);
            }

            // Seed the *other* modes' gain predictors from this level, so a
            // talk spurt resuming at any rate starts from the right energy.
            predictor.reseed_from_sid(ctx, self.log_en);
        }

        // Level adjustment tracks the mode with a first-order filter rather
        // than jumping: 0.9 old + 0.1 new, in Q11 throughout.
        let held = mult(ctx, self.log_en_adjust, Word16(29491));
        let raised = shl(ctx, DTX_LOG_EN_ADJUST[mode], 5);
        let scaled = mult(ctx, raised, Word16(3277));
        let fresh = shr(ctx, scaled, 5);
        self.log_en_adjust = add(ctx, held, fresh);

        let (lsp_int, l_log_en_int) = self.interpolate(ctx);
        let (lsf_int, lsf_int_variab) = self.apply_variability(ctx, &lsp_int);

        lsf_state.set_last_lsf(lsf_int);

        let lsp_int = lsf_to_lsp(ctx, &lsf_int);
        let lsp_int_variab = lsf_to_lsp(ctx, &lsf_int_variab);

        let acoeff = lsp_to_lp(ctx, &lsp_int);
        let acoeff_variab = lsp_to_lp(ctx, &lsp_int_variab);

        let mut a_t = [Word16(0); 4 * MP1];
        for sf in 0..4 {
            a_t[sf * MP1..(sf + 1) * MP1].copy_from_slice(&acoeff);
        }

        let level = self.level(ctx, &acoeff, l_log_en_int);

        let mut synth = [Word16(0); L_FRAME];
        for sf in 0..4 {
            let mut ex = build_cn_code(ctx, &mut self.pn_seed_rx);
            for sample in &mut ex {
                *sample = mult(ctx, level, *sample);
            }
            *mem_syn = synthesis_filter(
                ctx,
                &acoeff_variab,
                &ex,
                &mut synth[sf * L_SUBFR..(sf + 1) * L_SUBFR],
                mem_syn,
            );
        }

        if new_state == DtxState::DtxMute {
            self.mute(ctx);
        }

        // Only now, after the frame is built: a description that arrived this
        // frame has been used, so the interpolation clock restarts. The
        // `dtxHangoverAdded` half covers a SID_FIRST, which carries no data of
        // its own but did refresh the estimate by backward analysis.
        if self.sid_frame && (self.valid_data || self.dtx_hangover_added) {
            self.since_last_sid = Word16(0);
            self.data_updated = true;
        }

        ComfortNoise {
            synth,
            a_t,
            lsf: lsf_int,
        }
    }

    /// Derive a noise description from the decoded speech that preceded it.
    ///
    /// Runs when a SID arrives *after* the encoder added its hangover: those
    /// hangover frames were speech-coded but were already background, so the
    /// eight-frame history holds a clean picture of the noise and no
    /// transmitted description is needed.
    fn backward_analysis(&mut self, ctx: &mut DspContext, mode: usize) {
        self.log_en_adjust = DTX_LOG_EN_ADJUST[mode];

        // Duplicate the newest entry forward one slot. The history is about to
        // be averaged and the newest frame is the most representative, so it
        // gets counted twice -- and the pointers themselves do *not* advance.
        let ptr = (self.lsf_hist_ptr + M) % (M * DTX_HIST_SIZE);
        let newest: [Word16; M] = self.lsf_hist[self.lsf_hist_ptr..self.lsf_hist_ptr + M]
            .try_into()
            .expect("M wide");
        self.lsf_hist[ptr..ptr + M].copy_from_slice(&newest);

        let ptr = (self.log_en_hist_ptr + 1) % DTX_HIST_SIZE;
        self.log_en_hist[ptr] = self.log_en_hist[self.log_en_hist_ptr];

        self.log_en = Word16(0);
        let mut l_lsf = [Word32(0); M];
        for i in 0..DTX_HIST_SIZE {
            let eighth = shr(ctx, self.log_en_hist[i], 3);
            self.log_en = add(ctx, self.log_en, eighth);
            let frame = &self.lsf_hist[i * M..(i + 1) * M];
            for (acc, &coefficient) in l_lsf.iter_mut().zip(frame.iter()) {
                *acc = l_add(ctx, *acc, l_deposit_l(coefficient));
            }
        }

        let mut lsf = [Word16(0); M];
        for (slot, &acc) in lsf.iter_mut().zip(l_lsf.iter()) {
            *slot = extract_l(l_shr(ctx, acc, 3));
        }
        self.lsp = lsf_to_lsp(ctx, &lsf);

        // Store the level mode-independently; the mode's adjustment is added
        // back just before synthesis, so a rate change mid-silence is handled.
        self.log_en = sub(ctx, self.log_en, self.log_en_adjust);

        self.compute_variability_vectors(ctx);
    }

    /// Build `lsf_hist_mean`: how far each history frame sits from the mean.
    ///
    /// This becomes the perturbation added to the interpolated spectrum, so
    /// comfort noise is not a single frozen filter repeated. The limiting is
    /// two-stage on purpose — a soft knee at 655 that compresses four to one,
    /// then a hard stop at 1310 — so an unusual frame widens the variability a
    /// little rather than dominating it.
    fn compute_variability_vectors(&mut self, ctx: &mut DspContext) {
        self.lsf_hist_mean = self.lsf_hist;

        for (i, &scale) in LSF_HIST_MEAN_SCALE.iter().enumerate() {
            let mut l_mean = Word32(0);
            for j in 0..DTX_HIST_SIZE {
                l_mean = l_add(ctx, l_mean, l_deposit_l(self.lsf_hist_mean[i + j * M]));
            }
            let mean = extract_l(l_shr(ctx, l_mean, 3));

            for j in 0..DTX_HIST_SIZE {
                let at = i + j * M;
                let mut v = sub(ctx, self.lsf_hist_mean[at], mean);
                v = mult(ctx, v, scale);

                let negative = v.0 < 0;
                v = abs_s(ctx, v);

                if sub(ctx, v, Word16(655)).0 > 0 {
                    let excess = sub(ctx, v, Word16(655));
                    let compressed = shr(ctx, excess, 2);
                    v = add(ctx, Word16(655), compressed);
                }
                if sub(ctx, v, Word16(1310)).0 > 0 {
                    v = Word16(1310);
                }
                self.lsf_hist_mean[at] = if negative { Word16(-v.0) } else { v };
            }
        }
    }

    /// Take a `SID_UPDATE`'s parameters as the new description.
    fn absorb_sid(&mut self, ctx: &mut DspContext, parm: &[u16]) {
        assert!(parm.len() >= 5, "a SID frame carries five parameters");

        // How many frames the last description had to cover. The division
        // below is only defined below 32, so a longer gap is treated as 32 --
        // by then the interpolation has reached its endpoint anyway.
        let mut span = self.since_last_sid;
        self.since_last_sid = Word16(0);
        if sub(ctx, span, Word16(32)).0 > 0 {
            span = Word16(32);
        }
        self.true_sid_period_inv = if sub(ctx, span, Word16(2)).0 >= 0 {
            let scaled = shl(ctx, span, 10);
            div_s(Word16(1 << 10), scaled)
        } else {
            Word16(1 << 14)
        };

        let lsf_state_seed = parm[0];
        let mut lsf_state = LsfDecoder::at_reset();
        lsf_state.seed_predictor(lsf_state_seed);
        self.lsp = lsf_state.decode_sid(&parm[1..4]);

        let log_en_index = Word16(i16::try_from(parm[4]).expect("six bits"));
        // Q11, and divided by four: the index is a quarter-unit step.
        self.log_en = shl(ctx, log_en_index, 11 - 2);
        self.log_en = sub(ctx, self.log_en, Word16(2560 * 2));
        if log_en_index.0 == 0 {
            // Index 0 is reserved for digital silence, not for "very quiet".
            self.log_en = Word16(i16::MIN);
        }

        // Nothing to interpolate *from* at the start of a stream, or when a
        // SID lands directly on the heels of speech.
        if !self.data_updated || self.global_state == DtxState::Speech {
            self.lsp_old = self.lsp;
            self.old_log_en = self.log_en;
        }
    }

    /// Interpolate the spectrum and the level between the last two
    /// descriptions.
    ///
    /// Returns the LSPs in Q15 and the log energy as a Q26 accumulator.
    fn interpolate(&self, ctx: &mut DspContext) -> ([Word16; M], Word32) {
        let elapsed = add(ctx, Word16(1), self.since_last_sid);
        let mut int_fac = shl(ctx, elapsed, 10);
        int_fac = mult(ctx, int_fac, self.true_sid_period_inv);
        if sub(ctx, int_fac, Word16(1024)).0 > 0 {
            int_fac = Word16(1024);
        }
        int_fac = shl(ctx, int_fac, 4);

        let mut l_log_en_int = l_mult(ctx, int_fac, self.log_en);
        let mut lsp_int = [Word16(0); M];
        for (slot, &lsp) in lsp_int.iter_mut().zip(self.lsp.iter()) {
            *slot = mult(ctx, int_fac, lsp);
        }

        let int_fac = sub(ctx, Word16(16384), int_fac);
        l_log_en_int = l_mac(ctx, l_log_en_int, int_fac, self.old_log_en);
        for (slot, &old_lsp) in lsp_int.iter_mut().zip(self.lsp_old.iter()) {
            let old = mult(ctx, int_fac, old_lsp);
            *slot = add(ctx, *slot, old);
            *slot = shl(ctx, *slot, 1);
        }

        (lsp_int, l_log_en_int)
    }

    /// Add the per-frame spectral perturbation, and reorder both results.
    ///
    /// Returns the plain interpolated LSFs and the perturbed ones. Both are
    /// reordered — the plain set is what the rest of the decoder is told the
    /// spectrum was, so it has to be a legal one too.
    #[allow(clippy::similar_names)]
    fn apply_variability(
        &mut self,
        ctx: &mut DspContext,
        lsp_int: &[Word16; M],
    ) -> ([Word16; M], [Word16; M]) {
        // How much variability: driven by the running prediction gain, so a
        // strongly-shaped background gets perturbed less than a flat one.
        let mut factor = sub(ctx, self.log_pg_mean, Word16(2457));
        let scaled = mult(ctx, factor, Word16(9830));
        factor = sub(ctx, Word16(4096), scaled);
        if sub(ctx, factor, Word16(4096)).0 > 0 {
            factor = Word16(4096);
        }
        if factor.0 < 0 {
            factor = Word16(0);
        }
        factor = shl(ctx, factor, 3);

        // Which of the eight history frames to perturb toward. Drawn from the
        // same register the pulse positions come from, and before them.
        let index = pseudonoise(&mut self.pn_seed_rx, 3);
        let index = usize::try_from(index.0).expect("three bits");

        let mut lsf_int = lsp_to_lsf(ctx, lsp_int);
        let mut lsf_int_variab = lsf_int;
        let deviation = &self.lsf_hist_mean[index * M..(index + 1) * M];
        for (slot, &d) in lsf_int_variab.iter_mut().zip(deviation.iter()) {
            let offset = mult(ctx, factor, d);
            *slot = add(ctx, *slot, offset);
        }

        reorder_lsf(ctx, &mut lsf_int, LSF_GAP);
        reorder_lsf(ctx, &mut lsf_int_variab, LSF_GAP);

        (lsf_int, lsf_int_variab)
    }

    /// The excitation gain, Q4, and the prediction-gain running mean it
    /// updates.
    ///
    /// The interpolated log energy describes the *signal*, but the excitation
    /// is what gets scaled, so the synthesis filter's own gain has to come back
    /// out. That gain is computed from the reflection coefficients: the product
    /// of `1 - k²` is the prediction error power, and its logarithm is
    /// subtracted from the target level.
    fn level(
        &mut self,
        ctx: &mut DspContext,
        acoeff: &[Word16; MP1],
        mut l_log_en: Word32,
    ) -> Word16 {
        let refl = a_refl(ctx, &acoeff[1..]);

        let mut pred_err = Word16(i16::MAX);
        for &k in &refl {
            let k2 = mult(ctx, k, k);
            let residual = sub(ctx, Word16(i16::MAX), k2);
            pred_err = mult(ctx, pred_err, residual);
        }

        let (log_pg_e, log_pg_m) = log2(ctx, l_deposit_l(pred_err));
        let exponent = sub(ctx, log_pg_e, Word16(15));
        let mut log_pg = shl(ctx, exponent, 12);
        let mantissa = shr(ctx, log_pg_m, 15 - 12);
        let whole = add(ctx, log_pg, mantissa);
        let negated = sub(ctx, Word16(0), whole);
        log_pg = shr(ctx, negated, 1);

        let held = mult(ctx, Word16(29491), self.log_pg_mean);
        let fresh = mult(ctx, Word16(3277), log_pg);
        self.log_pg_mean = add(ctx, held, fresh);

        l_log_en = l_shr(ctx, l_log_en, 10);
        l_log_en = l_add(ctx, l_log_en, Word32(4 * 65536));
        let gain_term = l_shl(ctx, l_deposit_l(log_pg), 4);
        l_log_en = l_sub(ctx, l_log_en, gain_term);
        let adjust_term = l_shl(ctx, l_deposit_l(self.log_en_adjust), 5);
        l_log_en = l_add(ctx, l_log_en, adjust_term);

        let exponent = extract_h(l_log_en);
        let remainder = l_sub(ctx, l_log_en, l_deposit_h(exponent));
        let mantissa = extract_l(l_shr(ctx, remainder, 1));
        extract_l(pow2(ctx, exponent, mantissa))
    }

    /// Fade the comfort noise, and restart the interpolation from where it is.
    ///
    /// Muting is not a separate synthesis path — the frame has already been
    /// built by the time this runs. It lowers the *stored* level by 0.75 dB
    /// and repoints the interpolation at it, so the fade happens over the
    /// frames that follow.
    fn mute(&mut self, ctx: &mut DspContext) {
        let mut span = self.since_last_sid;
        if sub(ctx, span, Word16(32)).0 > 0 {
            span = Word16(32);
        }
        // The reference guards this explicitly: `since_last_sid` can be zero
        // here, and `div_s` by zero is undefined.
        if span.0 <= 0 {
            span = Word16(8);
        }
        let scaled = shl(ctx, span, 10);
        self.true_sid_period_inv = div_s(Word16(1 << 10), scaled);

        self.since_last_sid = Word16(0);
        self.lsp_old = self.lsp;
        self.old_log_en = self.log_en;
        // 256 in Q11 is 1/8, which is -6/8 dB.
        self.log_en = sub(ctx, self.log_en, Word16(256));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive a whole stream the way the frame loop does.
    ///
    /// `receive` *increments* `since_last_sid`; only `comfort_noise` resets
    /// it. So a helper that called `receive` alone would mute every stream
    /// eventually no matter what arrived, and the muting tests below would
    /// pass for the wrong reason.
    fn drive(types: &[RxFrameType]) -> Vec<DtxState> {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        let mut mem_syn = [Word16(0); M];
        let mut lsf_state = LsfDecoder::at_reset();
        let mut predictor = CodeGainPredictor::new();
        // A plausible SID: seed vector 0, a mid codebook entry per split, and
        // a mid energy index.
        let parm = [0u16, 100, 200, 300, 30];

        types
            .iter()
            .map(|&t| {
                let state = dtx.receive(&mut ctx, t);
                if state != DtxState::Speech {
                    dtx.comfort_noise(
                        &mut ctx,
                        state,
                        8,
                        &parm,
                        &mut mem_syn,
                        &mut lsf_state,
                        &mut predictor,
                    );
                }
                dtx.commit(state);
                state
            })
            .collect()
    }

    /// The reference's own state table, transcribed and checked entry by entry.
    ///
    /// Every row of the comment above `rx_dtx_handler`, for all three previous
    /// states. The two rows that a plausible implementation gets wrong are
    /// `RX_NO_DATA` and `RX_SPEECH_BAD`, whose meaning depends entirely on
    /// where the decoder already was.
    #[test]
    fn the_state_table_matches_the_specification_entry_for_entry() {
        use DtxState::{Dtx, DtxMute, Speech};
        use RxFrameType as F;

        // (incoming, from SPEECH, from DTX, from DTX_MUTE)
        let table = [
            (F::SpeechGood, Speech, Speech, Speech),
            (F::SpeechDegraded, Speech, Speech, Speech),
            // The reference's comment table says DTX_MUTE in the third
            // column here. Its code says otherwise: `RX_SPEECH_BAD` is not
            // among the four sticky inputs, and the staleness test cannot
            // catch it either, because entering DTX_MUTE runs the fade, and
            // the fade zeroes `since_last_sid`. So a damaged speech frame
            // genuinely drops a muting decoder back to plain DTX. The
            // comment and the code disagree; the code is what the reference
            // executes and what the bit-exactness is measured against, and
            // the assertion below pins the disagreement so it is not
            // rediscovered as a bug.
            (F::SpeechBad, Speech, Dtx, Dtx),
            (F::SidFirst, Dtx, Dtx, DtxMute),
            (F::SidUpdate, Dtx, Dtx, Dtx),
            (F::SidBad, Dtx, Dtx, DtxMute),
            (F::NoData, Speech, Dtx, DtxMute),
            (F::Onset, Speech, Dtx, DtxMute),
        ];

        let mut checked = 0;
        for (incoming, from_speech, from_dtx, from_mute) in table {
            for (start, want) in [(Speech, from_speech), (Dtx, from_dtx), (DtxMute, from_mute)] {
                let mut ctx = DspContext::default();
                let mut dtx = DtxDecoder::new();
                dtx.commit(start);
                // A fresh decoder has since_last_sid == 0, so the staleness
                // test cannot fire and this isolates the table itself.
                let got = dtx.receive(&mut ctx, incoming);
                assert_eq!(got, want, "{incoming:?} from {start:?}");
                checked += 1;
            }
        }
        assert_eq!(checked, 24, "the table has 24 entries");

        // On a stream that really does reach DTX_MUTE, and with the fade
        // actually run: a damaged speech frame leaves muting. This is the
        // half of the comment table that the code contradicts.
        let mut stream = vec![F::SidFirst];
        stream.extend(std::iter::repeat_n(F::NoData, 55));
        stream.push(F::SpeechBad);
        let states = drive(&stream);
        assert_eq!(
            states[55], DtxMute,
            "the stream must be muting first, or this proves nothing"
        );
        assert_eq!(*states.last().expect("non-empty"), Dtx);
    }

    /// Staleness mutes, and a genuine update rescues it.
    #[test]
    fn a_long_silence_mutes_and_a_sid_update_does_not() {
        let mut quiet = vec![RxFrameType::SidFirst];
        quiet.extend(std::iter::repeat_n(RxFrameType::NoData, 60));
        let states = drive(&quiet);

        // The SID_FIRST at index 0 zeroes the counter on its way out of
        // `comfort_noise`, so the NO_DATA at index n leaves it at n, and the
        // threshold test is strict. Index 51 is the first mute.
        let first_mute = states.iter().position(|&s| s == DtxState::DtxMute);
        assert_eq!(first_mute, Some(51), "muting did not begin where expected");
        assert!(states[..51].iter().all(|&s| s == DtxState::Dtx));

        // The same stream with an update at frame 45 never mutes: the update
        // resets the counter.
        let mut rescued = quiet.clone();
        rescued[45] = RxFrameType::SidUpdate;
        let states = drive(&rescued);
        assert!(
            states.iter().all(|&s| s != DtxState::DtxMute),
            "an update partway through should have prevented muting"
        );
    }

    /// A late `SID_UPDATE` is not punished for its own lateness.
    ///
    /// The counter is incremented before the staleness test, so an update
    /// arriving *after* the threshold has already passed would mute if it were
    /// not exempted. The reference exempts it explicitly, with a comment
    /// saying so, and this is that comment as a test.
    #[test]
    fn an_update_arriving_after_the_threshold_still_returns_plain_dtx() {
        let mut stream = vec![RxFrameType::SidFirst];
        stream.extend(std::iter::repeat_n(RxFrameType::NoData, 55));
        stream.push(RxFrameType::SidUpdate);
        let states = drive(&stream);
        assert_eq!(*states.last().expect("non-empty"), DtxState::Dtx);
        assert_eq!(
            states[55],
            DtxState::DtxMute,
            "it should have been muting before"
        );
    }

    /// Speech resets the staleness counter even when it never reaches DTX.
    #[test]
    fn speech_clears_the_staleness_counter() {
        let mut stream = vec![RxFrameType::SidFirst];
        stream.extend(std::iter::repeat_n(RxFrameType::NoData, 40));
        stream.push(RxFrameType::SpeechGood);
        stream.push(RxFrameType::SidFirst);
        stream.extend(std::iter::repeat_n(RxFrameType::NoData, 45));
        let states = drive(&stream);
        assert!(
            states.iter().all(|&s| s != DtxState::DtxMute),
            "the speech frame should have restarted the clock"
        );
    }

    /// The hangover counter starts at 32767, and it shows.
    ///
    /// `dec_ana_elapsed_count` starting at zero would make the first silence's
    /// backward analysis wait seven frames it should not. Observable through
    /// `dtx_hangover_added`, which a stream's first `SID_FIRST` must set.
    #[test]
    fn the_first_silence_of_a_stream_triggers_backward_analysis_at_once() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.commit(DtxState::Speech);
        let state = dtx.receive(&mut ctx, RxFrameType::SidFirst);
        assert_eq!(state, DtxState::Dtx);
        assert!(
            dtx.dtx_hangover_added,
            "hangover was not detected on the first SID"
        );
        assert!(dtx.sid_frame);
        assert!(!dtx.valid_data, "a SID_FIRST carries no parameters");

        // And with the counter zeroed instead, it would not have fired.
        let mut dtx = DtxDecoder::new();
        dtx.dec_ana_elapsed_count = Word16(0);
        dtx.commit(DtxState::Speech);
        dtx.receive(&mut ctx, RxFrameType::SidFirst);
        assert!(
            !dtx.dtx_hangover_added,
            "the test is vacuous if this also fires"
        );
    }

    /// A damaged SID keeps the old description rather than re-deriving one.
    #[test]
    fn a_damaged_sid_does_not_run_backward_analysis() {
        let mut ctx = DspContext::default();
        let mut dtx = DtxDecoder::new();
        dtx.commit(DtxState::Speech);
        dtx.receive(&mut ctx, RxFrameType::SidBad);
        assert!(dtx.sid_frame, "it is still a SID frame");
        assert!(!dtx.valid_data);
        assert!(
            !dtx.dtx_hangover_added,
            "damaged bits must not drive the analysis"
        );
    }
}
