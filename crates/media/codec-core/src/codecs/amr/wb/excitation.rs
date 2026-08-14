//! Excitation assembly and upsampling, 3GPP TS 26.190 §6.5 and §6.9.
//!
//! Combining the adaptive and algebraic contributions into the excitation the
//! synthesis filter runs on, and resampling the result from 12.8 to 16 kHz.
//!
//! # Adaptive scaling is the whole difficulty here
//!
//! The excitation is not kept at a fixed scale. Each subframe picks a shift
//! `q_new` so the loudest sample uses the available headroom, and the *entire*
//! history buffer is rescaled whenever that shift moves. Two reasons, and both
//! are structural:
//!
//! - The synthesis filter is recursive, so the excitation needs every bit of
//!   precision it can carry; a fixed conservative scale would waste headroom on
//!   quiet passages and quantise them coarsely.
//! - The adaptive codebook reads the *history*, so history and present must
//!   share a scale. Rescaling only the new subframe would make the pitch
//!   prediction read samples at the wrong loudness.
//!
//! The shift is chosen from the smallest headroom seen across the last four
//! subframes, not just the current one — a step up in level must not clip the
//! samples the next subframe will predict from.

use super::codebook::L_SUBFR;
use super::gain_tables::FIR_UP;
use super::ltp::{HISTORY, L_INTERPOL, PIT_MAX};
use crate::fixed_point::arith::{abs_s, add, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_mult};
use crate::fixed_point::shift::{l_shl, norm_s};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Largest shift the synthesis filter can absorb.
const Q_MAX: i16 = 8;

/// Subframe length at 16 kHz.
pub const L_SUBFR16K: usize = 80;

/// Taps per phase in the upsampling filter.
const NB_COEF_UP: usize = 12;

/// Upsampling ratio numerator and denominator: 16/12.8 = 5/4.
const FAC5: usize = 5;
const FAC4: usize = 4;

/// The excitation buffer and its scaling state.
///
/// Holds [`HISTORY`] samples of past excitation ahead of the current subframe,
/// which is what the adaptive codebook reaches back into.
#[derive(Debug, Clone)]
pub struct Excitation {
    buffer: Vec<Word16>,
    /// Headroom seen in each of the last four subframes.
    q_subfr: [Word16; 4],
    /// The shift the buffer is currently held at.
    q_old: i16,
}

impl Default for Excitation {
    fn default() -> Self {
        Self::new()
    }
}

impl Excitation {
    /// An empty excitation history, at the reference's reset state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            // One past the subframe: the adaptive codebook writes an extra
            // sample because the LTP low-pass filter reads one ahead.
            buffer: vec![Word16(0); HISTORY + L_SUBFR + 1],
            // Q_MAX, not zero. The reference seeds both to the maximum shift
            // so the first subframes are free to use full headroom; starting
            // at zero pins q_new to zero until four subframes have run, which
            // silently mis-scales the entire start of a stream.
            q_subfr: [Word16(Q_MAX); 4],
            q_old: Q_MAX,
        }
    }

    /// The excitation history, most recent last.
    #[must_use]
    pub fn history(&self) -> &[Word16] {
        &self.buffer[..HISTORY]
    }

    /// Mutable access to the whole buffer, for [`super::ltp::predict`].
    ///
    /// The adaptive codebook writes into the current subframe while reading the
    /// history, so it needs one contiguous buffer rather than two slices.
    #[must_use]
    pub fn buffer_mut(&mut self) -> (&mut [Word16], usize) {
        (&mut self.buffer, HISTORY)
    }

    /// The shift the buffer is currently scaled by.
    #[must_use]
    pub const fn q_new(&self) -> i16 {
        self.q_old
    }

    /// Choose this subframe's scaling from the code gain and recent headroom.
    ///
    /// Returns the shift and the code gain rounded into it.
    fn choose_scaling(&self, ctx: &mut DspContext, gain_code: Word32) -> (i16, Word16) {
        // The tightest headroom across the last four subframes, so a step up in
        // level cannot clip what the next subframe predicts from.
        let mut limit = self.q_subfr.iter().map(|q| q.0).min().unwrap_or(0);
        limit = limit.min(Q_MAX);

        let mut q_new = 0i16;
        let mut scaled = gain_code;
        while scaled.0 < 0x0800_0000 && q_new < limit {
            scaled = l_shl(ctx, scaled, 1);
            q_new += 1;
        }
        (q_new, round(ctx, scaled))
    }

    /// Rescale the whole buffer to a new shift.
    fn rescale(&mut self, ctx: &mut DspContext, shift: i16) {
        if shift == 0 {
            return;
        }
        for sample in &mut self.buffer {
            // Saturating rather than wrapping: a loud transient should clip,
            // not invert.
            let widened = l_shl(ctx, l_deposit_h(*sample), shift);
            *sample = round(ctx, widened);
        }
    }

    /// Choose this subframe's scaling and rescale the buffer to it.
    ///
    /// Separate from [`Self::assemble`] because the reference copies the
    /// adaptive-only excitation *after* the rescale and before the total is
    /// built over it — capturing it either side of this call gives different
    /// answers, and the enhanced path needs the post-rescale one.
    ///
    /// Returns the shift and the code gain rounded into it.
    pub fn rescale_to(&mut self, gain_code: Word32) -> (i16, Word16) {
        let mut ctx = DspContext::default();
        let (q_new, gain_word) = self.choose_scaling(&mut ctx, gain_code);
        let shift = q_new - self.q_old;
        self.rescale(&mut ctx, shift);
        self.q_old = q_new;
        (q_new, gain_word)
    }

    /// Build the total excitation over an already-rescaled buffer.
    ///
    /// `gain_code` is the value [`Self::rescale_to`] returned.
    pub fn build(
        &mut self,
        code: &[Word16; L_SUBFR],
        gain_pitch: Word16,
        gain_code: Word16,
        q_new: i16,
    ) -> [Word16; L_SUBFR] {
        let mut ctx = DspContext::default();
        let offset = HISTORY;
        let mut out = [Word16(0); L_SUBFR];
        for i in 0..L_SUBFR {
            let mut acc = l_mult(&mut ctx, code[i], gain_code);
            acc = l_shl(&mut ctx, acc, 5);
            acc = l_mac(&mut ctx, acc, self.buffer[offset + i], gain_pitch);
            let acc = l_shl(&mut ctx, acc, 1);
            let sample = round(&mut ctx, acc);
            self.buffer[offset + i] = sample;
            out[i] = sample;
        }

        let mut max = Word16(1);
        for sample in &out {
            let magnitude = abs_s(&mut ctx, *sample);
            if magnitude.0 > max.0 {
                max = magnitude;
            }
        }
        let raised = add(&mut ctx, Word16(norm_s(max)), Word16(q_new));
        let headroom = sub(&mut ctx, raised, Word16(1));
        self.q_subfr.rotate_right(1);
        self.q_subfr[0] = headroom;

        out
    }

    /// Assemble one subframe's excitation.
    ///
    /// The adaptive contribution must already be in the current subframe — that
    /// is what [`super::ltp::predict`] writes there. `code` is the innovation,
    /// `gain_pitch` Q14, `gain_code` Q16.
    ///
    /// Returns the assembled subframe and the shift it is scaled by.
    pub fn assemble(
        &mut self,
        code: &[Word16; L_SUBFR],
        gain_pitch: Word16,
        gain_code: Word32,
    ) -> ([Word16; L_SUBFR], i16) {
        let mut ctx = DspContext::default();

        let (q_new, gain_code) = self.choose_scaling(&mut ctx, gain_code);
        let shift = q_new - self.q_old;
        self.rescale(&mut ctx, shift);
        self.q_old = q_new;

        let offset = HISTORY;
        let mut out = [Word16(0); L_SUBFR];
        for i in 0..L_SUBFR {
            let mut acc = l_mult(&mut ctx, code[i], gain_code);
            acc = l_shl(&mut ctx, acc, 5);
            acc = l_mac(&mut ctx, acc, self.buffer[offset + i], gain_pitch);
            let acc = l_shl(&mut ctx, acc, 1);
            let sample = round(&mut ctx, acc);
            self.buffer[offset + i] = sample;
            out[i] = sample;
        }

        // Record the headroom this subframe leaves, for the next choice.
        let mut max = Word16(1);
        for sample in &out {
            let magnitude = abs_s(&mut ctx, *sample);
            if magnitude.0 > max.0 {
                max = magnitude;
            }
        }
        let raised = add(&mut ctx, Word16(norm_s(max)), Word16(q_new));
        let headroom = sub(&mut ctx, raised, Word16(1));
        self.q_subfr.rotate_right(1);
        self.q_subfr[0] = headroom;

        (out, q_new)
    }

    /// Slide the buffer forward by one subframe, discarding the oldest history.
    pub fn advance(&mut self) {
        self.buffer.copy_within(L_SUBFR..HISTORY + L_SUBFR, 0);
        self.buffer[HISTORY..].fill(Word16(0));
    }

    /// The headroom recorded for the last four subframes, oldest last.
    #[must_use]
    pub const fn scaling_history(&self) -> &[Word16; 4] {
        &self.q_subfr
    }
}

/// Resampler state for 12.8 to 16 kHz.
#[derive(Debug, Clone)]
pub struct Upsampler {
    memory: [Word16; 2 * NB_COEF_UP],
}

impl Default for Upsampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Upsampler {
    /// A resampler with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            memory: [Word16(0); 2 * NB_COEF_UP],
        }
    }

    /// Resample one subframe: 64 samples in, 80 out.
    ///
    /// Ratio 5/4, realised by walking the input at 4/5 of a sample per output
    /// and interpolating with a five-phase filter — the same
    /// polyphase trick as the pitch predictor's quarter-sample lags, at a
    /// different resolution.
    #[must_use]
    pub fn process(&mut self, input: &[Word16; L_SUBFR]) -> [Word16; L_SUBFR16K] {
        let mut ctx = DspContext::default();

        // History in front, so the filter can reach back across the boundary.
        let mut signal = [Word16(0); L_SUBFR + 2 * NB_COEF_UP];
        signal[..2 * NB_COEF_UP].copy_from_slice(&self.memory);
        signal[2 * NB_COEF_UP..].copy_from_slice(input);

        let mut out = [Word16(0); L_SUBFR16K];
        let mut pos = 0usize;
        for slot in &mut out {
            // pos counts in fifths of an input sample.
            let i = pos / FAC5;
            let frac = pos % FAC5;
            // The filter is centred on sample i, so it starts NB_COEF_UP - 1
            // taps earlier: signal[NB_COEF_UP + i - (NB_COEF_UP - 1)].
            *slot = interpolate(&mut ctx, &signal[i + 1..], frac);
            pos += FAC4;
        }

        self.memory.copy_from_slice(&signal[L_SUBFR..]);
        out
    }
}

/// One output sample: the 24-tap filter at the phase `frac` selects.
fn interpolate(ctx: &mut DspContext, x: &[Word16], frac: usize) -> Word16 {
    let mut sum = Word32(0);
    let mut k = FAC5 - 1 - frac;
    for &sample in x.iter().take(2 * NB_COEF_UP) {
        sum = l_mac(ctx, sum, sample, Word16(FIR_UP[k]));
        k += FAC5;
    }
    // Saturation can occur here, and clipping beats wrapping.
    let scaled = l_shl(ctx, sum, 1);
    round(ctx, scaled)
}

/// Total excitation history the buffer holds, for callers sizing their own.
pub const EXCITATION_HISTORY: usize = PIT_MAX + L_INTERPOL;

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_row, block_row_i32, has_block};
    use super::*;

    /// The oracle's deterministic adaptive contribution for one subframe.
    fn adaptive(sf: usize) -> [Word16; L_SUBFR] {
        let mut v = [Word16(0); L_SUBFR];
        for (n, slot) in v.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let t = (sf * L_SUBFR + n) as f64;
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = Word16((2000.0 * (2.0 * std::f64::consts::PI * t / 37.0).sin()) as i16);
            }
        }
        v
    }

    /// The oracle's deterministic innovation for one subframe.
    fn innovation(sf: usize) -> [Word16; L_SUBFR] {
        let mut code = [Word16(0); L_SUBFR];
        for (n, slot) in code.iter_mut().enumerate() {
            if n % 11 == sf % 11 {
                *slot = Word16(if n % 2 == 1 { -512 } else { 512 });
            }
        }
        code
    }

    fn expect(label: &str, got: &[Word16], sf: usize) {
        let want = block_row("excasm", &format!("{label}{sf}"));
        assert_eq!(want.len(), got.len(), "{label}{sf}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w,
                "{label}{sf}: sample {i} = {} but the reference gives {w}",
                g.0
            );
        }
    }

    #[test]
    fn excitation_assembly_and_upsampling_are_bit_exact_against_ts26173() {
        assert!(has_block("excasm"), "fixture block excasm missing");

        let mut exc = Excitation::new();
        let mut up = Upsampler::new();

        for sf in 0..4 {
            let meta = block_row_i32("excasm", &format!("meta{sf}"));
            let gain_pitch = Word16(i16::try_from(meta[0]).expect("pitch gain is a Word16"));
            let gain_code = Word32(meta[2]);

            // The adaptive contribution is written into the subframe first,
            // exactly as the pitch predictor would.
            let (buffer, offset) = exc.buffer_mut();
            buffer[offset..offset + L_SUBFR].copy_from_slice(&adaptive(sf));

            let code = innovation(sf);
            expect("code", &code, sf);

            let (assembled, _) = exc.assemble(&code, gain_pitch, gain_code);
            expect("exc", &assembled, sf);
            expect("q", exc.scaling_history(), sf);

            let upsampled = up.process(&assembled);
            expect("up", &upsampled, sf);

            exc.advance();
        }
    }

    #[test]
    fn rescaling_moves_the_history_with_the_present() {
        // The adaptive codebook reads the history, so both must share a scale.
        // Rescaling only the new subframe would make pitch prediction read at
        // the wrong loudness -- audible as a level jump whenever the shift
        // changes.
        let mut exc = Excitation::new();

        // The limit is the *minimum* headroom across all four recorded
        // subframes, and a fresh decoder has four zeroes -- so no shift is
        // possible until four subframes have run. That is deliberate: the
        // scaling must not open up on the strength of one quiet subframe.
        for _ in 0..4 {
            let (buffer, offset) = exc.buffer_mut();
            buffer[offset..offset + L_SUBFR].fill(Word16(100));
            exc.assemble(&[Word16(0); L_SUBFR], Word16(4096), Word32(1 << 20));
            exc.advance();
        }
        assert!(
            exc.scaling_history().iter().all(|q| q.0 > 0),
            "four quiet subframes left no headroom to shift into"
        );

        {
            let (buffer, _) = exc.buffer_mut();
            buffer.fill(Word16(1000));
        }
        let before = exc.history()[0];

        // A tiny code gain drives the shift to the recorded limit.
        exc.assemble(&[Word16(0); L_SUBFR], Word16(0), Word32(1));
        let after = exc.history()[0];

        assert_ne!(
            before.0, after.0,
            "the history was left at the old scale when the shift moved"
        );
    }

    #[test]
    fn the_shift_never_exceeds_what_the_synthesis_filter_absorbs() {
        // Q_MAX is set by the synthesis filter's headroom; exceeding it would
        // overflow there rather than here, which is much harder to diagnose.
        let mut exc = Excitation::new();
        for sf in 0..20 {
            let (buffer, offset) = exc.buffer_mut();
            buffer[offset..offset + L_SUBFR].copy_from_slice(&adaptive(sf));
            let (_, q) = exc.assemble(&innovation(sf), Word16(8000), Word32(1));
            assert!(q <= Q_MAX, "subframe {sf} chose a shift of {q}");
            exc.advance();
        }
    }

    #[test]
    fn upsampling_produces_five_samples_for_every_four() {
        let mut up = Upsampler::new();
        let input = [Word16(0); L_SUBFR];
        assert_eq!(up.process(&input).len() * FAC4, L_SUBFR * FAC5);
    }

    #[test]
    fn upsampling_preserves_a_tone_in_the_passband() {
        // A 1 kHz tone is well inside the filter's flat region, so it must
        // come through at roughly its input amplitude -- not attenuated, and
        // not aliased into something else.
        let mut up = Upsampler::new();
        let mut peak = 0i32;
        for block in 0..6 {
            let mut input = [Word16(0); L_SUBFR];
            for (n, slot) in input.iter_mut().enumerate() {
                #[allow(clippy::cast_precision_loss)]
                let t = (block * L_SUBFR + n) as f64;
                #[allow(clippy::cast_possible_truncation)]
                {
                    *slot = Word16(
                        (8000.0 * (2.0 * std::f64::consts::PI * 1000.0 * t / 12800.0).sin()) as i16,
                    );
                }
            }
            let out = up.process(&input);
            // Skip the blocks while the filter's history fills.
            if block >= 2 {
                peak = peak.max(out.iter().map(|s| i32::from(s.0).abs()).max().unwrap_or(0));
            }
        }
        assert!(
            (7000..=9000).contains(&peak),
            "a 1 kHz tone came out at {peak} against an input of 8000"
        );
    }

    #[test]
    fn the_upsampler_carries_state_across_subframes() {
        // Without the carried history each block would start from silence, and
        // the seams would click every 5 ms.
        let input = [Word16(4000); L_SUBFR];

        let mut continuous = Upsampler::new();
        let _ = continuous.process(&input);
        let carried = continuous.process(&input);

        let mut fresh = Upsampler::new();
        let isolated = fresh.process(&input);

        assert_ne!(
            carried, isolated,
            "the resampler produced the same block with and without history"
        );
    }
}
