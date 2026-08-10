//! What the encoder does to a frame of PCM before it is speech: the 13-bit
//! input restriction and the 80 Hz high-pass that also halves the level.
//!
//! Implements TS 26.073 `Speech_Encode_Frame`'s input masking (`sp_enc.c`
//! 137–141, active because the conformance build does not define `NO13BIT`)
//! and `Pre_Process` (`pre_proc.c` 143–173). Both mutate the caller's frame in
//! place, and `cod_amr` then copies the *processed* samples into its speech
//! history — so everything downstream, including the LP analysis window, sees
//! the output of this module and never the raw PCM.
//!
//! Validated against the `speech` rows of `testdata/nb_enc_trace.txt`, which
//! record `st->new_speech` after both steps, replayed across all three
//! committed frames so the carried filter state is exercised rather than
//! assumed. Checked once, off-tree, against the full 50-frame trace that
//! `tools/trace-amrnb-encoder.sh 4` produces: all 8000 samples matched.
//!
//! # Q-formats
//!
//! Input and output are Q0, 16-bit linear PCM. The "divide by two" the
//! reference's header promises is not a shift anywhere in the code — it is
//! folded into [`PRE_PROC_B`], whose coefficients are already halved.

// The PCM reader in `trace_support` reinterprets a `u16` bit pattern as a
// signed sample, which is exactly what the format means.
#![allow(clippy::cast_possible_wrap)]

use crate::fixed_point::arith::round;
use crate::fixed_point::arith32::{l_add, l_mac};
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::l_shl;
use crate::fixed_point::types::{DspContext, Word16, Word32};

use super::super::decoder_tables::{PRE_PROC_A, PRE_PROC_B};

/// Bits of every input sample the encoder is allowed to look at.
///
/// TS 26.073 masks the three least significant bits off every sample before
/// anything else touches it, so a 16-bit source is coded as if it were 13-bit.
/// This is not optional in the conformance build, and it is not a no-op: it
/// changes roughly seven eighths of all samples and moves the pre-processor's
/// output within the first few samples of the first frame.
const INPUT_MASK: i16 = -8; // 0xfff8 as a signed 16-bit pattern

/// Clear the three least significant bits of every sample, in place.
///
/// `frame` is Q0 PCM. Plain bitwise AND, exactly as the reference: this is one
/// of the few places where TS 26.073 uses a C operator rather than a basic
/// operator, so a basic operator here would be the wrong transcription.
///
/// The reference's encoder-homing mask, `EHF_MASK = 0x0008`, survives this
/// unchanged — which is why the homing test must run on the *raw* frame, before
/// this is applied.
pub fn restrict_to_13_bits(frame: &mut [Word16]) {
    for sample in frame {
        sample.0 &= INPUT_MASK;
    }
}

/// The encoder's input high-pass, TS 26.073 `Pre_Process`.
///
/// A second-order section at 80 Hz whose numerator carries an extra factor of
/// ½. Six words of state, all zero at reset, carried across frames — the
/// filter is never restarted at a frame boundary.
///
/// The two `y` pairs are the recursion's own history in the reference's double
/// precision format (`hi << 16 + lo << 1`), *not* the emitted samples. The
/// recursion therefore runs at higher precision than the signal it produces;
/// feeding the rounded output back would drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preprocessor {
    /// `y[i - 1]`, double precision.
    y1: (Word16, Word16),
    /// `y[i - 2]`, double precision.
    y2: (Word16, Word16),
    /// `x[i - 1]` as seen at loop entry.
    x0: Word16,
    /// `x[i - 2]` as seen at loop entry.
    x1: Word16,
}

impl Preprocessor {
    /// A preprocessor in the state `Pre_Process_reset` leaves behind: silent.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            y1: (Word16(0), Word16(0)),
            y2: (Word16(0), Word16(0)),
            x0: Word16(0),
            x1: Word16(0),
        }
    }

    /// High-pass and halve `signal` in place, Q0 in and Q0 out.
    ///
    /// Any length; the reference calls this with a whole 160-sample frame on
    /// the live path and with a 40-sample lookahead on the priming path that
    /// its own driver never takes.
    pub fn filter(&mut self, ctx: &mut DspContext, signal: &mut [Word16]) {
        let b = PRE_PROC_B.map(Word16);
        let a = PRE_PROC_A.map(Word16);

        for sample in signal {
            // `x2` is snapshotted before the two state writes below. Reading
            // `self.x1` after updating it would give x[i-1] where x[i-2] is
            // wanted, which is a one-sample error in one of three numerator
            // taps: audible only as a slightly different spectrum, and fatal.
            let x2 = self.x1;
            self.x1 = self.x0;
            self.x0 = *sample;

            let mut acc = mpy_32_16(self.y1.0, self.y1.1, a[1]);
            acc = l_add(ctx, acc, mpy_32_16(self.y2.0, self.y2.1, a[2]));
            acc = l_mac(ctx, acc, self.x0, b[0]);
            acc = l_mac(ctx, acc, self.x1, b[1]);
            acc = l_mac(ctx, acc, x2, b[2]);
            // Q12 coefficients doubled by `L_mult` leave the accumulator at
            // Q13; three more bits put the result in the high half. Saturating.
            acc = l_shl(ctx, acc, 3);
            *sample = round(ctx, acc);

            self.y2 = self.y1;
            // Stored pre-`round`: the state is the unrounded accumulator, not
            // the sample just emitted.
            self.y1 = l_extract(acc);
        }
    }

    /// Mask to 13 bits and high-pass, the pair of steps `Speech_Encode_Frame`
    /// runs before `cod_amr` sees anything.
    ///
    /// Order matters: the mask applies to the raw PCM and the filter to the
    /// masked samples.
    pub fn condition(&mut self, ctx: &mut DspContext, frame: &mut [Word16]) {
        restrict_to_13_bits(frame);
        self.filter(ctx, frame);
    }

    /// The recursion's `y[i - 1]` and `y[i - 2]`, double precision, and the two
    /// carried input samples — for tests that need to assert the state rather
    /// than infer it from the output.
    #[must_use]
    pub const fn state(&self) -> ((Word16, Word16), (Word16, Word16), Word16, Word16) {
        (self.y1, self.y2, self.x0, self.x1)
    }
}

/// Recompose one of the preprocessor's double-precision state pairs.
///
/// Exposed because the pair is meaningless as two independent numbers: the
/// reference's format is `hi << 16 + lo << 1`, so the low word carries 15 bits
/// and not 16.
#[must_use]
pub fn compose(hi: Word16, lo: Word16) -> Word32 {
    crate::fixed_point::oper32::l_comp(hi, lo)
}

/// Reader for `testdata/nb_enc_trace.txt` and the PCM it was produced from.
///
/// Test-only, and shared with the sibling encoder modules, which is why it
/// lives at module scope rather than inside `mod tests`.
///
/// # A passing test that compares nothing
///
/// Every accessor panics rather than returning an `Option`. A trace row that
/// silently yields nothing makes a bit-exact test pass vacuously, and a green
/// suite is otherwise indistinguishable from a suite that compared nothing.
#[cfg(test)]
pub(crate) mod trace_support {
    use crate::fixed_point::types::Word16;

    use super::super::super::L_FRAME;

    /// The committed trace: `T <frame> <subframe> <name> <values...>`, with
    /// `-1` as the subframe of a frame-level row.
    pub const TRACE: &str = include_str!("../../testdata/nb_enc_trace.txt");

    /// The 8 kHz input the reference encoder was driven with.
    const INPUT: &[u8] = include_bytes!("../../testdata/amrnb_enc_input.pcm");

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

    /// One labelled row, as raw integers. `subframe` is -1 for frame-level rows.
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

    /// One row as `Word16`s, with its length asserted.
    pub fn words(frame: usize, subframe: i32, name: &str, expected: usize) -> Vec<Word16> {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), expected, "frame {frame}/{subframe}: {name} length");
        values
            .into_iter()
            .map(|v| Word16(i16::try_from(v).expect("row holds Word16 values")))
            .collect()
    }

    /// One scalar row.
    pub fn scalar(frame: usize, subframe: i32, name: &str) -> i32 {
        let values = row(frame, subframe, name);
        assert_eq!(values.len(), 1, "frame {frame}/{subframe}: {name} is a scalar");
        values[0]
    }

    /// One 160-sample input frame, decoded from the little-endian PCM.
    ///
    /// Raw: the caller applies [`super::restrict_to_13_bits`] and the
    /// pre-processor to get what `cod_amr` was handed.
    pub fn input_frame(frame: usize) -> [Word16; L_FRAME] {
        let mut samples = [Word16(0); L_FRAME];
        let base = frame * L_FRAME * 2;
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
    use super::trace_support::{frames, input_frame, words};
    use super::*;
    use crate::codecs::amr::nb::L_FRAME;

    /// Replay the whole preprocessor across every committed frame, keeping its
    /// state, and hand each conditioned frame to `check`.
    ///
    /// Returns the number of frames replayed so callers can assert it.
    fn replay(mut check: impl FnMut(usize, &[Word16; L_FRAME])) -> usize {
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let total = frames();
        for frame in 0..total {
            let mut samples = input_frame(frame);
            pre.condition(&mut ctx, &mut samples);
            check(frame, &samples);
        }
        total
    }

    #[test]
    fn conditioned_input_is_bit_exact_against_ts26073() {
        let mut compared = 0usize;
        let count = replay(|frame, got| {
            let want = words(frame, -1, "speech", L_FRAME);
            for (i, &sample) in got.iter().enumerate() {
                assert_eq!(
                    sample.0, want[i].0,
                    "frame {frame}: conditioned sample {i} differs from the reference"
                );
                compared += 1;
            }
        });
        assert_eq!(count, 3, "the committed trace covers three frames");
        assert_eq!(compared, 3 * L_FRAME, "480 samples compared");
    }

    #[test]
    fn the_thirteen_bit_mask_is_not_a_no_op() {
        // If the mask were skipped the filter would still produce plausible
        // speech, so this asserts the mask actually bites on the committed
        // input rather than trusting that it does.
        let mut changed = 0usize;
        let mut total = 0usize;
        for frame in 0..frames() {
            let raw = input_frame(frame);
            let mut masked = raw;
            restrict_to_13_bits(&mut masked);
            for (r, m) in raw.iter().zip(masked.iter()) {
                assert_eq!(m.0 & 7, 0, "masked sample still has low bits set");
                assert_eq!(m.0, r.0 & !7, "mask is not a plain bitwise AND");
                total += 1;
                changed += usize::from(r.0 != m.0);
            }
        }
        assert_eq!(total, 3 * L_FRAME, "480 samples inspected");
        assert!(
            changed > total / 2,
            "the mask changed only {changed} of {total} samples; the fixture is \
             not exercising it"
        );
    }

    #[test]
    fn skipping_the_mask_changes_the_conditioned_frame() {
        // The complement of the test above: masking is load-bearing all the way
        // through the filter, not merely visible in the input.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut unmasked = input_frame(0);
        pre.filter(&mut ctx, &mut unmasked);
        let want = words(0, -1, "speech", L_FRAME);
        let differences = unmasked
            .iter()
            .zip(want.iter())
            .filter(|(a, b)| a.0 != b.0)
            .count();
        assert!(
            differences > 0,
            "the unmasked frame matched the reference, so this fixture cannot \
             tell the two paths apart"
        );
    }

    #[test]
    fn filter_state_carries_across_frames() {
        // Restarting the filter each frame is the obvious wrong port and it
        // only shows up at the frame boundary, so pin it: frame 1 conditioned
        // from a fresh preprocessor must differ from frame 1 in the replay.
        let mut ctx = DspContext::default();
        let mut fresh = Preprocessor::new();
        let mut samples = input_frame(1);
        fresh.condition(&mut ctx, &mut samples);
        let want = words(1, -1, "speech", L_FRAME);
        assert_ne!(
            samples[0].0, want[0].0,
            "a fresh preprocessor reproduced frame 1's first sample, so this \
             test cannot detect a dropped state"
        );

        let mut carried = Preprocessor::new();
        let mut frame0 = input_frame(0);
        carried.condition(&mut ctx, &mut frame0);
        let mut frame1 = input_frame(1);
        carried.condition(&mut ctx, &mut frame1);
        assert_eq!(frame1[0].0, want[0].0, "carried state reproduces frame 1");
    }

    #[test]
    fn reset_state_is_silent_and_stays_silent() {
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut silence = [Word16(0); L_FRAME];
        pre.condition(&mut ctx, &mut silence);
        assert!(silence.iter().all(|s| s.0 == 0), "silence in, silence out");
        assert_eq!(pre, Preprocessor::new(), "silence left the state untouched");
    }

    #[test]
    fn recursion_state_is_the_unrounded_accumulator() {
        // The state update stores the pre-`round` value. Feeding back the
        // emitted sample instead is a one-LSB difference per sample that
        // compounds through the recursion, so check the low word carries the
        // bits `round` would have discarded on at least one sample.
        let mut ctx = DspContext::default();
        let mut pre = Preprocessor::new();
        let mut samples = input_frame(0);
        restrict_to_13_bits(&mut samples);
        let mut sub_lsb_state = 0usize;
        for &sample in &samples {
            let mut one = [sample];
            pre.filter(&mut ctx, &mut one);
            let (y1, _, _, _) = pre.state();
            let composed = compose(y1.0, y1.1);
            // `round` would have produced `extract_h(composed + 0x8000)`; a
            // non-zero low word is precision the output does not carry.
            sub_lsb_state += usize::from(y1.1.0 != 0);
            assert_eq!(
                crate::fixed_point::arith::extract_h(composed).0,
                y1.0.0,
                "the double-precision pair does not recompose"
            );
        }
        assert!(
            sub_lsb_state > L_FRAME / 2,
            "only {sub_lsb_state} of {L_FRAME} state words carried sub-LSB \
             precision; the recursion is running at output precision"
        );
    }
}
