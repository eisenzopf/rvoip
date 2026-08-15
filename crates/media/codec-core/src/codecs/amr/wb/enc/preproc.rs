//! Encoder input conditioning: 16 kHz → 12.8 kHz, 50 Hz high-pass,
//! pre-emphasis, and the adaptive frame scaling every later stage inherits.
//!
//! Implements the parts of TS 26.173 `coder()` that run before the LP analysis:
//! `Decim_12k8` / `Down_samp` / `Interpol` (`decim54.c`), the `L_FILT` tail
//! approximation (`cod_main.c` 313–318), `HP50_12k8` (`hp50.c`, reused from the
//! decoder's [`HighPass50`]), and the inline pre-emphasis-with-scaling loop
//! (`cod_main.c` 335–420) that derives `Q_new`.
//!
//! Validated against the `speech16k`, `decimated`, `Q_new` and `window` rows
//! of `testdata/wb_enc_trace.txt`, replayed across all three committed frames
//! so the carried filter memories and the two-frame `Q_max` history are
//! exercised rather than assumed. Checked once, off-tree, against the full
//! 50-frame trace `tools/trace-amrwb-encoder.sh` produces: every row of every
//! frame matched.
//!
//! # Q-formats
//!
//! Input and decimator output are Q0 (raw 16-bit PCM). After [`Preprocessor::
//! preemphasise`] the frame is still nominally Q0 but carries `Q_new` bits of
//! extra headroom — the whole point of the exercise — so every buffer that has
//! to stay commensurate with it must be rescaled by the returned `exp`.

// This module transcribes reference fixed-point arithmetic. The lints below
// fight the transcription rather than the code: the reference's magic
// constants are the specification, its index arithmetic is deliberately
// unchecked, and the coefficient table is laid out four per line to keep it
// diffable against `decim54.c`.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::unreadable_literal
)]

use super::super::synthesis::{HighPass50, PREEMPH_FAC};
use crate::fixed_point::arith::{add, extract_h, mult, round, sub};
use crate::fixed_point::arith32::{l_abs, l_mac, l_msu, l_mult};
use crate::fixed_point::shift::{l_shl, norm_s, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Input frame at 16 kHz: 20 ms.
pub const L_FRAME16K: usize = 320;

/// Internal frame at 12.8 kHz.
pub const L_FRAME: usize = 256;

/// Group delay of the down-sampling filter, at 16 kHz.
pub const L_FILT16K: usize = 15;

/// Length of the synthesised tail, at 12.8 kHz.
pub const L_FILT: usize = 12;

/// Total internal speech buffer, which is also the LP analysis window.
pub const L_TOTAL: usize = 384;

/// Offset of `new_speech` within the total speech buffer.
///
/// `L_TOTAL - L_FRAME - L_FILT`. The freshly decimated frame plus its tail
/// occupy `[NEW_SPEECH, L_TOTAL)`; everything below is carried history.
pub const NEW_SPEECH: usize = L_TOTAL - L_FRAME - L_FILT;

/// Ceiling on the adaptive scaling exponent (`Q_MAX`).
///
/// Bounded so that low-level frames keep dynamic range for filter ringing, and
/// so the 32-bit synthesis filter cannot see `a[0] < 1`.
const Q_MAX: i16 = 8;

/// Half-length of the decimation FIR; the filter has `2 * NB_COEF_DOWN` taps.
const NB_COEF_DOWN: usize = 15;

/// 4/5 in Q15.
///
/// 0.8 · 32768 = 26214.4 rounded **up**. The reference relies on that: it
/// derives the output length as `mult(lg, DOWN_FAC)`, which floors, and 26214
/// would give 255 samples for a 320-sample frame instead of 256.
const DOWN_FAC: Word16 = Word16(26215);

/// Output-phase resolution of the polyphase bank.
const FAC4: i16 = 4;

/// Position increment in Q2 — 5/4 of an input sample per output sample.
const FAC5: i16 = 5;

/// Decimation FIR, Q14, as a 4-phase polyphase bank read with stride 4.
///
/// Transcribed from `fir_down` in `decim54.c` and kept in its four-per-line
/// layout, because each column is one phase and the per-column sums are the
/// cheapest check that the transcription is intact — see the
/// `fir_phases_have_unity_dc_gain` test.
const FIR_DOWN: [Word16; 4 * NB_COEF_DOWN * 2] = {
    const fn w(v: i16) -> Word16 {
        Word16(v)
    }
    [
        w(-1),
        w(-3),
        w(-6),
        w(-5),
        w(0),
        w(9),
        w(19),
        w(24),
        w(18),
        w(0),
        w(-26),
        w(-50),
        w(-58),
        w(-41),
        w(0),
        w(54),
        w(99),
        w(111),
        w(77),
        w(0),
        w(-95),
        w(-170),
        w(-188),
        w(-128),
        w(0),
        w(153),
        w(270),
        w(294),
        w(198),
        w(0),
        w(-233),
        w(-408),
        w(-441),
        w(-295),
        w(0),
        w(344),
        w(601),
        w(649),
        w(434),
        w(0),
        w(-507),
        w(-888),
        w(-964),
        w(-647),
        w(0),
        w(770),
        w(1366),
        w(1505),
        w(1030),
        w(0),
        w(-1293),
        w(-2379),
        w(-2746),
        w(-1997),
        w(0),
        w(3034),
        w(6575),
        w(9894),
        w(12254),
        w(13107),
        w(12254),
        w(9894),
        w(6575),
        w(3034),
        w(0),
        w(-1997),
        w(-2746),
        w(-2379),
        w(-1293),
        w(0),
        w(1030),
        w(1505),
        w(1366),
        w(770),
        w(0),
        w(-647),
        w(-964),
        w(-888),
        w(-507),
        w(0),
        w(434),
        w(649),
        w(601),
        w(344),
        w(0),
        w(-295),
        w(-441),
        w(-408),
        w(-233),
        w(0),
        w(198),
        w(294),
        w(270),
        w(153),
        w(0),
        w(-128),
        w(-188),
        w(-170),
        w(-95),
        w(0),
        w(77),
        w(111),
        w(99),
        w(54),
        w(0),
        w(-41),
        w(-58),
        w(-50),
        w(-26),
        w(0),
        w(18),
        w(24),
        w(19),
        w(9),
        w(0),
        w(-5),
        w(-6),
        w(-3),
        w(-1),
        w(0),
    ]
};

/// Discard the two least significant bits of every input sample.
///
/// AMR-WB is specified for **14-bit** uniform PCM, and TS 26.173 enforces that
/// at the encoder's caller rather than inside `coder()`: `coder.c` 233–236
/// masks the whole frame with `0xFFFC` immediately before the call. It is
/// therefore not part of [`Preprocessor`] — but it *is* part of the codec, and
/// skipping it changes the decimator output within the first dozen samples and
/// everything after. Apply it at the encoder's public entry point, once per
/// frame, before anything else.
///
/// Validated against the `speech16k` rows of the reference trace, which record
/// what `coder()` actually received.
pub fn restrict_to_14_bit(frame: &mut [Word16]) {
    for sample in frame.iter_mut() {
        // A plain C bitwise AND in the reference, not a basic operator.
        sample.0 &= -4; // 0xFFFC
    }
}

/// The frame scaling chosen by [`Preprocessor::preemphasise`].
///
/// Every buffer that has to stay commensurate with the new frame must be
/// rescaled by `exp` — see [`Scaling::exp`] for the list the reference applies
/// it to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scaling {
    /// `Q_new`: the left shift folded into this frame's pre-emphasis output.
    ///
    /// The minimum of this frame's headroom and the previous two frames', so
    /// that a sudden quiet frame cannot rescale the past out from under the
    /// filter memories.
    pub q_new: i16,
    /// `Q_new - Q_old`: how far the new frame moved relative to the last one.
    ///
    /// The reference applies this to `old_speech[0..116)`, `old_exc`,
    /// `mem_syn`, `mem_decim2`, `mem_wsp` and `mem_w0` — and to nothing else.
    /// `mem_preemph` is deliberately excluded: it is stored in the unscaled,
    /// post-high-pass domain.
    pub exp: i16,
}

/// The 16 kHz → 12.8 kHz polyphase decimator (`Decim_12k8`).
///
/// Carries the last 30 input samples, which is exactly the FIR's span. `Clone`
/// because the tail approximation of [`Preprocessor::band_limit`] must run the
/// filter forward from a *copy* of the state and then throw the copy away.
#[derive(Clone, Debug)]
pub struct Decimator {
    /// The previous call's last `2 * NB_COEF_DOWN` input samples.
    memory: [Word16; 2 * NB_COEF_DOWN],
}

impl Default for Decimator {
    fn default() -> Self {
        Self::new()
    }
}

impl Decimator {
    /// A decimator with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            memory: [Word16(0); 2 * NB_COEF_DOWN],
        }
    }

    /// Decimate `sig16k` (Q0, 16 kHz) into `out` (Q0, 12.8 kHz).
    ///
    /// # Panics
    ///
    /// If `out` is not `floor(len * 4/5)` long, or the input is longer than a
    /// frame — both are contract violations rather than data conditions.
    pub fn decimate(&mut self, sig16k: &[Word16], out: &mut [Word16]) {
        let lg = sig16k.len();
        assert!(lg <= L_FRAME16K, "decimator input is at most one frame");

        // The concatenation the reference builds on the stack: 30 words of
        // carried input, then the new input.
        let mut signal = [Word16(0); 2 * NB_COEF_DOWN + L_FRAME16K];
        signal[..2 * NB_COEF_DOWN].copy_from_slice(&self.memory);
        signal[2 * NB_COEF_DOWN..2 * NB_COEF_DOWN + lg].copy_from_slice(sig16k);

        let mut ctx = DspContext::default();
        let lg_down = mult(&mut ctx, Word16(lg as i16), DOWN_FAC).0 as usize;
        assert_eq!(
            out.len(),
            lg_down,
            "output length must be floor(4/5 · input)"
        );

        down_sample(&signal, out);

        // The last 30 words of the *concatenation*, not of the input. For a
        // short call (the 15-sample tail) that is part carried state and part
        // new input, which is why the tail call must be given a scratch copy.
        self.memory
            .copy_from_slice(&signal[lg..lg + 2 * NB_COEF_DOWN]);
    }
}

/// Resample `signal` by 4/5 (`Down_samp`).
///
/// `signal` is the full concatenated buffer. The reference offsets it by
/// `NB_COEF_DOWN` and then `Interpol` walks back `nb_coef - 1`; the two cancel
/// to a single `+1`, so output `j` reads `signal[i + 1 ..= i + 30]` for
/// `i = floor(5j/4)`.
fn down_sample(signal: &[Word16], out: &mut [Word16]) {
    let mut ctx = DspContext::default();
    // Position in Q2 — quarter-input-sample resolution. It reaches 5·255 =
    // 1275 at most, so `add` never saturates here.
    let mut pos = Word16(0);
    for slot in out.iter_mut() {
        let i = shr(&mut ctx, pos, 2).0 as usize;
        // A plain C bitwise AND in the reference, not a basic operator.
        let frac = pos.0 & 3;
        *slot = interpolate(&signal[i + 1..], frac);
        pos = add(&mut ctx, pos, Word16(FAC5));
    }
}

/// One polyphase output sample (`Interpol` specialised to `fir_down`).
///
/// `x` must expose at least `2 * NB_COEF_DOWN` samples. `frac` selects the
/// phase: the bank is read from `k = resol - 1 - frac` with stride `resol`, so
/// `k` sweeps `[0, 119]` and the whole 120-entry table is consumed.
fn interpolate(x: &[Word16], frac: i16) -> Word16 {
    let mut ctx = DspContext::default();
    let mut sum = Word32(0);
    // The reference advances `k` with a plain C add, not `add()`; it cannot
    // leave the table, so there is nothing to saturate.
    let mut k = (FAC4 - 1 - frac) as usize;
    for &sample in x.iter().take(2 * NB_COEF_DOWN) {
        sum = l_mac(&mut ctx, sum, sample, FIR_DOWN[k]);
        k += FAC4 as usize;
    }
    // Q14 coefficients against `L_mac`'s implicit doubling: this shift plus
    // `round`'s >>16 give the bank unity DC gain. Saturation can occur.
    let sum = l_shl(&mut ctx, sum, 1);
    round(&mut ctx, sum)
}

/// Input conditioning and the adaptive frame scaling, with its carried state.
///
/// Corresponds to the `mem_decim`, `mem_sig_in`, `mem_preemph`, `Q_old` and
/// `Q_max` fields of the reference's `Coder_State`.
#[derive(Clone, Debug)]
pub struct Preprocessor {
    /// `mem_decim`.
    decimator: Decimator,
    /// `mem_sig_in`.
    highpass: HighPass50,
    /// `mem_preemph`: the last real input sample of the previous frame, stored
    /// **unscaled** and never touched by the `Scale_sig` sweep.
    preemph_memory: Word16,
    /// `Q_old`: the scaling the previous frame settled on.
    q_old: i16,
    /// `Q_max`: the raw headroom of the previous two frames, most recent first.
    /// The *unclamped* `shift` is stored, never `Q_new` — storing the clamped
    /// value would let the limiter ratchet downward across frames.
    q_max: [i16; 2],
}

impl Default for Preprocessor {
    fn default() -> Self {
        Self::new()
    }
}

impl Preprocessor {
    /// A cold-start preprocessor, matching `Reset_encoder(st, 1)`.
    ///
    /// `Q_old` and both `Q_max` slots start at 15 even though [`Q_MAX`] is 8:
    /// the reset values sit deliberately *above* the ceiling so the first
    /// frame's `Q_new` is unconstrained by history. The resulting first `exp`
    /// is a large right shift applied to all-zero buffers, which is harmless.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            decimator: Decimator::new(),
            highpass: HighPass50::new(),
            preemph_memory: Word16(0),
            q_old: 15,
            q_max: [15, 15],
        }
    }

    /// Decimate one 16 kHz frame to 12.8 kHz and high-pass it at 50 Hz.
    ///
    /// `new_speech` receives `L_FRAME + L_FILT` samples in Q0: 256 real ones
    /// followed by a 12-sample tail. The tail is what the *next* 15 input
    /// samples would decimate to if they were silence — a stand-in for the
    /// decimator's group delay, so the coder need not add 15 samples of
    /// algorithmic delay. It is consumed by the LP analysis window and by the
    /// headroom scan in [`Self::preemphasise`], and is overwritten by real
    /// speech next frame.
    ///
    /// Both tail filters run from a **copy** of the persistent state, and both
    /// copies are dropped. Letting either write back would leave the next
    /// frame starting from a state that is half real speech and half zeros.
    ///
    /// # Panics
    ///
    /// If `new_speech` is not `L_FRAME + L_FILT` long.
    pub fn band_limit(&mut self, speech16k: &[Word16; L_FRAME16K], new_speech: &mut [Word16]) {
        assert_eq!(
            new_speech.len(),
            L_FRAME + L_FILT,
            "new_speech spans the frame plus its tail approximation"
        );
        let (frame, tail) = new_speech.split_at_mut(L_FRAME);

        self.decimator.decimate(speech16k, frame);
        let mut tail_decimator = self.decimator.clone();
        tail_decimator.decimate(&[Word16(0); L_FILT16K], tail);

        self.highpass.filter(frame);
        let mut tail_highpass = self.highpass.clone();
        tail_highpass.filter(tail);
    }

    /// Pre-emphasise `new_speech` in place through `1 - 0.68 z⁻¹`, folding in
    /// the frame's adaptive left shift.
    ///
    /// Returns the [`Scaling`] every later stage inherits. `new_speech` must be
    /// the `L_FRAME + L_FILT` samples [`Self::band_limit`] produced.
    ///
    /// The maximum is taken over the tail as well as the frame, so the 12
    /// synthesised samples help choose `Q_new` and therefore influence every
    /// subsequent frame through `Q_max`.
    ///
    /// # Panics
    ///
    /// If `new_speech` is not `L_FRAME + L_FILT` long.
    pub fn preemphasise(&mut self, new_speech: &mut [Word16]) -> Scaling {
        assert_eq!(
            new_speech.len(),
            L_FRAME + L_FILT,
            "new_speech spans the frame plus its tail approximation"
        );
        let mut ctx = DspContext::default();

        // 0.68 in Q14 — the pre-emphasis runs at half scale so that
        // `L_mult(x, 16384)` can carry the undelayed term.
        let mu = shr(&mut ctx, PREEMPH_FAC, 1);

        let shift = self.headroom(&mut ctx, new_speech, mu);

        // `Q_new` is the minimum of this frame's headroom and the previous
        // two frames' — but `q_max` keeps the raw `shift`, so the limiter
        // cannot compound.
        let q_new = shift.min(self.q_max[0]).min(self.q_max[1]);
        let exp = sub(&mut ctx, Word16(q_new), Word16(self.q_old)).0;
        self.q_old = q_new;
        self.q_max[1] = self.q_max[0];
        self.q_max[0] = shift;

        // The memory is the last *real* sample, index 255 — not the last tail
        // sample. Next frame's speech continues from there, and it is captured
        // before filtering because the loop below overwrites it.
        let memory = new_speech[L_FRAME - 1];

        // Downward, because the filter is in place and reads x[i-1]: the
        // descending order is what keeps that neighbour unfiltered.
        for i in (1..L_FRAME + L_FILT).rev() {
            let mut acc = l_mult(&mut ctx, new_speech[i], Word16(16384));
            acc = l_msu(&mut ctx, acc, new_speech[i - 1], mu);
            acc = l_shl(&mut ctx, acc, q_new);
            new_speech[i] = round(&mut ctx, acc);
        }
        let mut acc = l_mult(&mut ctx, new_speech[0], Word16(16384));
        acc = l_msu(&mut ctx, acc, self.preemph_memory, mu);
        acc = l_shl(&mut ctx, acc, q_new);
        new_speech[0] = round(&mut ctx, acc);

        self.preemph_memory = memory;

        Scaling { q_new, exp }
    }

    /// The headroom of the frame once pre-emphasised, as a left shift.
    ///
    /// A dry run of the filter: `new_speech` is not modified, only scanned.
    /// One bit of headroom is deliberately given away (`norm_s(...) - 1`)
    /// because `Residu` and the stages after it add gain.
    fn headroom(&self, ctx: &mut DspContext, new_speech: &[Word16], mu: Word16) -> i16 {
        let mut acc = l_mult(ctx, new_speech[0], Word16(16384));
        acc = l_msu(ctx, acc, self.preemph_memory, mu);
        let mut peak = l_abs(ctx, acc);

        for i in 1..L_FRAME + L_FILT {
            let mut acc = l_mult(ctx, new_speech[i], Word16(16384));
            acc = l_msu(ctx, acc, new_speech[i - 1], mu);
            let magnitude = l_abs(ctx, acc);
            if magnitude.0 > peak.0 {
                peak = magnitude;
            }
        }

        let top = extract_h(peak);
        if top.0 == 0 {
            // Silence short-circuits to the ceiling rather than falling
            // through `norm_s(0) - 1 = -1`.
            Q_MAX
        } else {
            sub(ctx, Word16(norm_s(top)), Word16(1)).0.clamp(0, Q_MAX)
        }
    }
}

/// Test-only access to the reference encoder's trace and its input.
///
/// Shared with [`super::analysis`] so both halves of the front end compare
/// against the same rows, parsed the same way. A row that is missing must be a
/// hard failure and not an empty comparison, which is why every accessor
/// panics rather than returning an `Option`.
#[cfg(test)]
pub(crate) mod trace_support {
    use crate::fixed_point::types::Word16;

    use super::L_FRAME16K;

    /// The committed trace: `T <frame> <subframe> <name> <values...>`.
    pub const TRACE: &str = include_str!("../../testdata/wb_enc_trace.txt");

    /// The 16 kHz input the reference encoder was driven with.
    const INPUT: &[u8] = include_bytes!("../../testdata/amrwb_enc_input.pcm");

    /// How many frames the committed trace covers.
    pub fn frames() -> usize {
        let mut highest = None;
        for line in TRACE.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() != Some("T") {
                continue;
            }
            let frame: usize = parts.next().expect("frame index").parse().expect("integer");
            highest = Some(highest.map_or(frame, |h: usize| h.max(frame)));
        }
        highest.expect("trace carries at least one frame") + 1
    }

    /// One labelled row, as raw integers.
    ///
    /// `subframe` is -1 for frame-level rows.
    pub fn row(frame: usize, subframe: i32, name: &str) -> Vec<i32> {
        for line in TRACE.lines() {
            let mut parts = line.split_whitespace();
            if parts.next() != Some("T") {
                continue;
            }
            let f: usize = parts.next().expect("frame index").parse().expect("integer");
            let s: i32 = parts.next().expect("subframe").parse().expect("integer");
            if f != frame || s != subframe || parts.next() != Some(name) {
                continue;
            }
            return parts.map(|v| v.parse().expect("integer")).collect();
        }
        panic!("trace has no row {name:?} for frame {frame} subframe {subframe}");
    }

    /// One frame-level row, as `Word16`s, with its length asserted.
    pub fn words(frame: usize, name: &str, expected: usize) -> Vec<Word16> {
        let values = row(frame, -1, name);
        assert_eq!(values.len(), expected, "frame {frame}: {name} length");
        values
            .into_iter()
            .map(|v| Word16(i16::try_from(v).expect("row holds Word16 values")))
            .collect()
    }

    /// One frame-level scalar row.
    pub fn scalar(frame: usize, name: &str) -> i32 {
        let values = row(frame, -1, name);
        assert_eq!(values.len(), 1, "frame {frame}: {name} is a scalar");
        values[0]
    }

    /// One 320-sample input frame, decoded from the little-endian PCM.
    ///
    /// Raw: the caller must apply [`super::restrict_to_14_bit`] to get what
    /// the reference's `coder()` was handed.
    pub fn input_frame(frame: usize) -> [Word16; L_FRAME16K] {
        let mut samples = [Word16(0); L_FRAME16K];
        let base = frame * L_FRAME16K * 2;
        for (n, slot) in samples.iter_mut().enumerate() {
            let lo = u16::from(INPUT[base + n * 2]);
            let hi = u16::from(INPUT[base + n * 2 + 1]);
            *slot = Word16((lo | (hi << 8)) as i16);
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::trace_support::{frames, input_frame, scalar, words};
    use super::*;

    /// Replay the preprocessor across every committed frame, keeping its
    /// state, and hand each frame to `check` as
    /// `(frame, band_limited, preemphasised, scaling)`.
    ///
    /// Returns the number of frames replayed so callers can assert it: a
    /// harness that silently yields nothing reads as agreement.
    fn replay(mut check: impl FnMut(usize, &[Word16], &[Word16], Scaling)) -> usize {
        let mut pre = Preprocessor::new();
        let total = frames();
        for frame in 0..total {
            let mut input = input_frame(frame);
            restrict_to_14_bit(&mut input);
            let mut new_speech = [Word16(0); L_FRAME + L_FILT];
            pre.band_limit(&input, &mut new_speech);
            let band_limited = new_speech;
            let scaling = pre.preemphasise(&mut new_speech);
            check(frame, &band_limited, &new_speech, scaling);
        }
        total
    }

    #[test]
    fn input_is_restricted_to_fourteen_bits_before_the_coder_sees_it() {
        // The `speech16k` rows record what the reference's `coder()` was
        // actually handed. They differ from the raw PCM in the two low bits of
        // roughly three quarters of all samples, and skipping the mask moves
        // the decimator output within the first twenty samples.
        let mut compared = 0usize;
        for frame in 0..frames() {
            let mut got = input_frame(frame);
            restrict_to_14_bit(&mut got);
            let want = words(frame, "speech16k", L_FRAME16K);
            for (i, &sample) in got.iter().enumerate() {
                assert_eq!(
                    sample.0, want[i].0,
                    "frame {frame}: input sample {i} differs from what coder() received"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, 3 * L_FRAME16K, "960 input samples compared");
    }

    #[test]
    fn decimation_and_highpass_are_bit_exact_against_ts26173() {
        let mut compared = 0usize;
        let count = replay(|frame, decimated, _, _| {
            let want = words(frame, "decimated", L_FRAME);
            for (i, &got) in decimated.iter().take(L_FRAME).enumerate() {
                assert_eq!(
                    got.0, want[i].0,
                    "frame {frame}: decimated[{i}] differs from the reference"
                );
                compared += 1;
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * L_FRAME, "768 samples compared");
    }

    #[test]
    fn preemphasised_frame_is_bit_exact_against_ts26173() {
        // `window[NEW_SPEECH..]` is exactly the pre-emphasised, scaled frame
        // plus its tail, so the trace's 384-word row validates all 268 of them
        // without needing the carried history that `analysis` assembles.
        let mut compared = 0usize;
        let count = replay(|frame, _, preemphasised, _| {
            let window = words(frame, "window", L_TOTAL);
            for (i, &got) in preemphasised.iter().enumerate() {
                assert_eq!(
                    got.0,
                    window[NEW_SPEECH + i].0,
                    "frame {frame}: preemphasised sample {i} differs from the reference"
                );
                compared += 1;
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * (L_FRAME + L_FILT), "804 samples compared");
    }

    #[test]
    fn adaptive_scaling_is_bit_exact_against_ts26173() {
        let mut seen = Vec::new();
        let count = replay(|frame, _, _, scaling| {
            assert_eq!(
                i32::from(scaling.q_new),
                scalar(frame, "Q_new"),
                "frame {frame}: Q_new differs from the reference"
            );
            seen.push(scaling.q_new);
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        // Not a constant across the three frames, so the two-frame `Q_max`
        // history is genuinely exercised rather than coincidentally right.
        assert_eq!(seen, vec![3, 2, 2], "Q_new must vary across the trace");
    }

    #[test]
    fn exp_tracks_the_change_in_scaling_across_frames() {
        // `exp` is computed against the *previous* Q_old and is what every
        // carried buffer is rescaled by, so it must be the difference of
        // consecutive traced `Q_new` values — with the cold-start Q_old of 15
        // in front.
        let mut previous = 15i16;
        let count = replay(|frame, _, _, scaling| {
            let want = i16::try_from(scalar(frame, "Q_new")).expect("Q_new is small") - previous;
            assert_eq!(scaling.exp, want, "frame {frame}: exp");
            previous = scaling.q_new;
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
    }

    #[test]
    fn fir_phases_have_unity_dc_gain() {
        // The bank is read with stride 4 from k = 3 - frac, so table column c
        // is the phase used for frac = 3 - c. Each column must sum to 2^14
        // (give or take the one LSB the reference gives away on three of the
        // four), or a transposition slipped into the 120-entry transcription.
        let want = [16383, 16384, 16383, 16383];
        for (column, &expected) in want.iter().enumerate() {
            let sum: i32 = FIR_DOWN
                .iter()
                .skip(column)
                .step_by(4)
                .map(|c| i32::from(c.0))
                .sum();
            assert_eq!(sum, expected, "phase column {column} sums wrong");
        }
        assert_eq!(FIR_DOWN.len(), 120);
    }

    #[test]
    fn output_length_is_four_fifths_of_the_input() {
        // `DOWN_FAC` is rounded up precisely so this floors the right way; the
        // 15-sample tail call is the one that would break with 26214.
        let mut ctx = DspContext::default();
        assert_eq!(mult(&mut ctx, Word16(320), DOWN_FAC).0, 256);
        assert_eq!(mult(&mut ctx, Word16(15), DOWN_FAC).0, 12);
        assert_eq!(mult(&mut ctx, Word16(320), Word16(26214)).0, 255);
    }

    #[test]
    fn the_tail_filters_do_not_disturb_the_carried_state() {
        // The whole risk of the tail approximation is a state write-back.
        // Running `band_limit` and then a second `band_limit` on the same
        // input must give the same answer as running the decimator and
        // high-pass directly over two frames of that input.
        let input = input_frame(0);

        let mut with_tail = Preprocessor::new();
        let mut first = [Word16(0); L_FRAME + L_FILT];
        with_tail.band_limit(&input, &mut first);
        let mut second = [Word16(0); L_FRAME + L_FILT];
        with_tail.band_limit(&input, &mut second);

        let mut plain_decimator = Decimator::new();
        let mut plain_highpass = HighPass50::new();
        let mut reference = [Word16(0); L_FRAME];
        plain_decimator.decimate(&input, &mut reference);
        plain_highpass.filter(&mut reference);
        plain_decimator.decimate(&input, &mut reference);
        plain_highpass.filter(&mut reference);

        for i in 0..L_FRAME {
            assert_eq!(
                second[i].0, reference[i].0,
                "sample {i}: the tail call leaked into the carried state"
            );
        }
    }

    #[test]
    fn silence_takes_the_maximum_scaling() {
        // `tmp == 0` short-circuits to Q_MAX rather than falling through
        // `norm_s(0) - 1 = -1`, and the Q_max ceiling is 8 even though the
        // reset value is 15.
        let mut pre = Preprocessor::new();
        let mut new_speech = [Word16(0); L_FRAME + L_FILT];
        pre.band_limit(&[Word16(0); L_FRAME16K], &mut new_speech);
        let scaling = pre.preemphasise(&mut new_speech);
        assert_eq!(scaling.q_new, Q_MAX);
        assert_eq!(scaling.exp, Q_MAX - 15);
    }
}
