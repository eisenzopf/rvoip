//! The AMR-NB decoder post-filter and output post-processing, TS 26.073
//! `pstfilt.c` (`Post_Filter`) and `post_pro.c` (`Post_Process`).
//!
//! This is the stage AMR-WB does not have. It runs on the synthesised speech
//! after the excitation and synthesis path have finished with it, and it is the
//! last thing between the decoder and the caller's PCM:
//!
//! 1. inverse-filter the synthesis through `A(z/g3)` to recover a residual,
//! 2. flatten the spectral tilt with `1 - mu*k*z^-1`, where `k` comes from the
//!    first two autocorrelation lags of the combined filter's impulse response,
//! 3. re-synthesise through `1/A(z/g4)`,
//! 4. restore the original subframe energy with a smoothed gain,
//! 5. high-pass at 60 Hz and double the amplitude.
//!
//! # Two signals, not one
//!
//! `Post_Filter` overwrites its `syn` argument in place, but every stage that
//! *reads* the synthesis — the residual, the AGC's reference energy, and the
//! ten-sample history carried into the next frame — reads the **unfiltered**
//! copy taken before the first subframe. Feeding the post-filtered signal back
//! into any of those produces audio that sounds fine and fails conformance,
//! because the filter then compounds on itself frame after frame.
//!
//! # Q formats
//!
//! Signals are Q0 throughout: the synthesis, the residual, the post-filtered
//! output and the post-processed output are all plain 16-bit samples. LP
//! coefficients are Q12 with `a[0] == 4096`; the expansion factors, the tilt
//! coefficient and the AGC factor are Q15; the AGC's carried gain is Q12.
//! `Residu` and `Syn_filt` are scale-preserving — their `L_shl(s, 3)` is what
//! takes the Q13 accumulator to Q16 so that `round` yields Q0 again.
//!
//! # Validation
//!
//! Bit-exact against the `postfilter` and `postproc` sections of
//! `testdata/nb_stages.txt`, which `tools/amrnb_stage_oracle.c` produced by
//! driving TS 26.073's own `Post_Filter` and `Post_Process` over four replayed
//! frames each — long enough that every carried memory has to be right, not
//! just the arithmetic.

use super::decoder_tables::{GAMMA3, GAMMA3_MR122, GAMMA4, GAMMA4_MR122};
use super::lsp::{AZ_SIZE, M, MP1};
use super::synthesis::{
    expand_bandwidth, lp_residual, synthesis_filter, synthesis_filter_in_place, AdaptiveGain,
    Preemphasis,
};
use super::{AGC_FAC, L_FRAME, L_SUBFR, MU};
use crate::fixed_point::arith::{extract_h, mult, round};
use crate::fixed_point::arith32::{l_add, l_mac, l_mult};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::l_shl;
use crate::fixed_point::types::{DspContext, Word16};

/// Length of the truncated impulse response the tilt estimate is measured on.
///
/// Twenty-two samples is a `pstfilt.c` local, not a codec-wide constant: it is
/// long enough for the two autocorrelation lags to be stable and short enough
/// that the ratio still describes the tilt rather than the whole spectrum.
const L_H: usize = 22;


/// High-pass numerator, Q13 — `0.9398` and its mirror, for a 60 Hz corner.
const HP_B: [Word16; 3] = [Word16(7699), Word16(-15398), Word16(7699)];

/// High-pass feedback coefficients `a[1]` and `a[2]`, Q13.
///
/// The reference's `a[0] = 8192` is dead — the recursion is already normalised
/// — so it is not carried here. Keeping it would only invite an off-by-one.
const HP_A: [Word16; 2] = [Word16(15836), Word16(-7667)];

/// Mode index of 10.2 kbit/s, `MR102` in the reference's enum.
const MODE_MR102: u8 = 6;

/// Mode index of 12.2 kbit/s, `MR122`.
const MODE_MR122: u8 = 7;

/// Numerator and denominator bandwidth-expansion factors for a rate, Q15.
///
/// The only mode dependency in the whole post-filter. The two high rates
/// widen the numerator from `0.55^n` to `0.7^n` and the denominator from
/// `0.7^n` to `0.75^n`; everything else shares one pair.
///
/// [`GAMMA4`] and [`GAMMA3_MR122`] hold identical numbers — both are `0.7^n` —
/// which is why picking the wrong denominator is invisible at 12.2 and 10.2
/// and detunes every other rate by a factor that is still perfectly stable.
const fn expansion_factors(mode_index: u8) -> (&'static [i16; M], &'static [i16; M]) {
    match mode_index {
        MODE_MR102 | MODE_MR122 => (&GAMMA3_MR122, &GAMMA4_MR122),
        _ => (&GAMMA3, &GAMMA4),
    }
}

/// The formant post-filter and its carried state, TS 26.073 `Post_Filter`.
///
/// Four memories survive from one frame to the next, and all four are
/// load-bearing: the ten unfiltered synthesis samples the residual filter needs
/// before the start of a frame, the synthesis memory of `1/A(z/g4)`, the
/// pre-emphasis sample, and the AGC's smoothed gain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostFilter {
    /// Ten samples of previous-frame history followed by the current frame's
    /// **unfiltered** synthesis.
    ///
    /// One buffer rather than two because the residual filter's window
    /// straddles the frame boundary at the first subframe and lies wholly
    /// inside the frame afterwards; splitting it would need a special case for
    /// exactly that subframe.
    synthesis: [Word16; M + L_FRAME],
    /// Synthesis memory of `1/A(z/g4)` — the last ten *unscaled* outputs.
    ///
    /// Captured before the AGC rescales the subframe, because the reference
    /// updates it inside `Syn_filt`. Capturing it after would feed the gain
    /// back into the filter and make it drift.
    denominator_memory: [Word16; M],
    /// The tilt-compensation filter's one-sample memory, in the residual
    /// domain.
    tilt: Preemphasis,
    /// The output gain control's smoothed gain.
    gain: AdaptiveGain,
}

impl Default for PostFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl PostFilter {
    /// A post-filter in its reset state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            synthesis: [Word16(0); M + L_FRAME],
            denominator_memory: [Word16(0); M],
            tilt: Preemphasis::new(),
            gain: AdaptiveGain::new(),
        }
    }

    /// Return to the reset state, TS 26.073 `Post_Filter_reset`.
    ///
    /// Every buffer zeroes except the AGC's gain, which resets to unity — 4096
    /// in Q12, not 0. A zeroed gain would silence the first subframe after
    /// every homing frame.
    pub const fn reset(&mut self) {
        *self = Self::new();
    }

    /// Post-filter one frame of synthesised speech, in place.
    ///
    /// `synthesis` is 160 Q0 samples in and the post-filtered 160 out. `az` is
    /// the four subframes' interpolated LP coefficients, Q12, eleven per
    /// subframe with `az[11 * k] == 4096`. `mode_index` is the *speech* mode,
    /// 0..=7 — comfort-noise frames are post-filtered with the speech mode
    /// they would have used, never with a DTX pseudo-mode.
    pub fn process(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        synthesis: &mut [Word16; L_FRAME],
        az: &[Word16; AZ_SIZE],
    ) {
        // Snapshot the whole unfiltered frame before any subframe is touched.
        // From here on `self.synthesis` is the unfiltered signal and the
        // caller's buffer is progressively overwritten with the filtered one.
        self.synthesis[M..].copy_from_slice(synthesis);

        let (gamma3, gamma4) = expansion_factors(mode_index);

        // Field-by-field so the residual can read `unfiltered` while the
        // tilt and gain memories are borrowed mutably.
        let Self {
            synthesis: unfiltered,
            denominator_memory,
            tilt,
            gain,
        } = self;

        let mut residual = [Word16(0); L_SUBFR];

        for (subframe, coefficients) in az.chunks_exact(MP1).enumerate() {
            let start = subframe * L_SUBFR;
            let numerator = expand_bandwidth(ctx, coefficients, gamma3);
            let denominator = expand_bandwidth(ctx, coefficients, gamma4);

            // The window reaches `M` samples before the subframe. At subframe
            // zero those samples are the previous frame's unfiltered tail.
            lp_residual(
                ctx,
                &numerator,
                &unfiltered[start..start + M + L_SUBFR],
                &mut residual,
            );

            let coefficient = tilt_coefficient(ctx, &numerator, &denominator);
            tilt.filter(ctx, &mut residual, coefficient);

            *denominator_memory = synthesis_filter(
                ctx,
                &denominator,
                &residual,
                &mut synthesis[start..start + L_SUBFR],
                denominator_memory,
            );

            // The AGC's reference is the unfiltered synthesis, which lives
            // `M` samples further into the buffer than the residual window did.
            gain.scale(
                ctx,
                &unfiltered[M + start..M + start + L_SUBFR],
                &mut synthesis[start..start + L_SUBFR],
                Word16(AGC_FAC),
            );
        }

        // Carry the unfiltered tail, not the filtered one, into the history
        // the next frame's first residual window will read.
        unfiltered.copy_within(L_FRAME.., 0);
    }
}

/// Tilt-compensation coefficient for one subframe, Q15.
///
/// The impulse response of `A(z/g3) / A(z/g4)` is truncated to [`L_H`] samples
/// and its first two autocorrelation lags taken; the coefficient is
/// `mu * r1 / r0`, or zero when `r1` is non-positive — a non-positive lag-1
/// correlation means the response has no tilt worth removing.
fn tilt_coefficient(
    ctx: &mut DspContext,
    numerator: &[Word16; MP1],
    denominator: &[Word16; MP1],
) -> Word16 {
    let mut response = [Word16(0); L_H];
    response[..MP1].copy_from_slice(numerator);

    // The reference filters `h` into itself with `&h[M+1]` as the filter
    // memory, which is legal there only because `Syn_filt` copies the memory
    // out and runs the recursion on scratch before writing any output. The
    // memory is the eleven zeros just written, so the equivalent — and the only
    // form Rust's borrow rules admit — is a zeroed memory that is not updated.
    let zero_memory = [Word16(0); M];
    synthesis_filter_in_place(ctx, denominator, &mut response, &zero_memory);

    // `extract_h` truncates; the reference does not round either lag, and
    // rounding one of them shifts the tilt by a hair on every subframe.
    let mut acc = l_mult(ctx, response[0], response[0]);
    for &h in &response[1..] {
        acc = l_mac(ctx, acc, h, h);
    }
    let lag0 = extract_h(acc);

    // Twenty-one products, not twenty-two: the leading term is h[0]*h[1] and
    // the loop stops one short of the end.
    let mut acc = l_mult(ctx, response[0], response[1]);
    for pair in response[1..].windows(2) {
        acc = l_mac(ctx, acc, pair[0], pair[1]);
    }
    let lag1 = extract_h(acc);

    if lag1.0 <= 0 {
        Word16(0)
    } else {
        // `div_s` is only defined for a dividend no larger than its divisor.
        // That holds here because `mu` is 0.8: scaling a positive `lag1` down
        // by 0.8 keeps it strictly under `lag0`, which dominates every other
        // lag of an autocorrelation.
        let damped = mult(ctx, lag1, Word16(MU));
        div_s(damped, lag0)
    }
}

/// Output post-processing, TS 26.073 `Post_Process`.
///
/// A second-order 60 Hz high-pass followed by a doubling of the amplitude. The
/// two are one filter in the reference and stay one here, because the doubling
/// happens *after* the recursion state is taken: the feedback carries the
/// undoubled value, and storing the doubled one instead gives a filter with
/// twice the intended feedback — stable, plausible, and wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostProcessor {
    /// Previous filter output *before* the final doubling, split into
    /// `L_Extract` high/low halves. Q16 against the sample scale, which is what
    /// makes the Q13 feedback coefficients land back on Q16.
    previous: (Word16, Word16),
    /// The output before that, same form.
    second_previous: (Word16, Word16),
    /// Previous *input* sample, Q0.
    last_input: Word16,
    /// The input before that.
    second_last_input: Word16,
}

impl PostProcessor {
    /// A post-processor in its reset state — all six memories zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous: (Word16(0), Word16(0)),
            second_previous: (Word16(0), Word16(0)),
            last_input: Word16(0),
            second_last_input: Word16(0),
        }
    }

    /// Return to the reset state, TS 26.073 `Post_Process_reset`.
    pub const fn reset(&mut self) {
        *self = Self::new();
    }

    /// High-pass and double `signal` in place, Q0 in and Q0 out.
    ///
    /// Called once per frame with all 160 samples; the memories carry across
    /// the call, so splitting a frame into two calls gives the same result and
    /// dropping the state does not.
    pub fn process(&mut self, ctx: &mut DspContext, signal: &mut [Word16]) {
        for sample in signal.iter_mut() {
            let third_last_input = self.second_last_input;
            self.second_last_input = self.last_input;
            // Capture the input before the output overwrites it.
            self.last_input = *sample;

            let mut acc = mpy_32_16(self.previous.0, self.previous.1, HP_A[0]);
            acc = l_add(
                ctx,
                acc,
                mpy_32_16(self.second_previous.0, self.second_previous.1, HP_A[1]),
            );
            acc = l_mac(ctx, acc, self.last_input, HP_B[0]);
            acc = l_mac(ctx, acc, self.second_last_input, HP_B[1]);
            acc = l_mac(ctx, acc, third_last_input, HP_B[2]);
            acc = l_shl(ctx, acc, 2);

            // The doubling saturates, and it is deliberately not folded back
            // into the recursion below.
            let doubled = l_shl(ctx, acc, 1);
            *sample = round(ctx, doubled);

            self.second_previous = self.previous;
            self.previous = l_extract(acc);
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::nb::vectors;

    /// 7.40 kbit/s — the mode the post-filter oracle ran at.
    const MODE_MR74: u8 = 4;

    fn ctx() -> DspContext {
        DspContext::default()
    }

    fn az_of(section: &str) -> [Word16; AZ_SIZE] {
        let rows = vectors::rows(section);
        let row = rows
            .iter()
            .find(|r| r.label == "az")
            .unwrap_or_else(|| panic!("{section} has no `az` row"));
        row.words()
            .try_into()
            .expect("an interpolated LP set is 4 x MP1 coefficients")
    }

    /// The `in`/`out` pairs of a replayed section, in file order.
    fn frames(section: &str) -> Vec<(Vec<Word16>, Vec<Word16>)> {
        let rows = vectors::rows(section);
        let mut pairs = Vec::new();
        let mut pending: Option<Vec<Word16>> = None;
        for row in &rows {
            match row.label {
                "in" => {
                    assert!(pending.is_none(), "{section}: two `in` rows in a row");
                    pending = Some(row.words());
                }
                "out" => {
                    let input = pending
                        .take()
                        .unwrap_or_else(|| panic!("{section}: an `out` row with no `in`"));
                    pairs.push((input, row.words()));
                }
                "az" | "seed" => {}
                other => panic!("{section}: unexpected row {other:?}"),
            }
        }
        assert!(pending.is_none(), "{section}: a trailing `in` with no `out`");
        pairs
    }

    /// Compare one case and return how many samples were actually looked at.
    ///
    /// The count is returned rather than discarded because a fixture row that
    /// parsed as empty would otherwise make the loop below agree with itself.
    fn compare(label: &str, frame: usize, got: &[Word16], want: &[Word16]) -> usize {
        assert!(!want.is_empty(), "{label} frame {frame}: the fixture row is empty");
        assert_eq!(got.len(), want.len(), "{label} frame {frame}: length");
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            assert_eq!(g, w, "{label} frame {frame} sample {i}");
        }
        got.len()
    }

    // ── the two sections this module owns ────────────────────────────────────

    #[test]
    fn post_filter_matches_the_reference_over_four_replayed_frames() {
        let az = az_of("postfilter");
        let pairs = frames("postfilter");
        assert_eq!(pairs.len(), 4, "the oracle replays four frames");

        let mut ctx = ctx();
        let mut filter = PostFilter::new();
        let mut samples = 0;

        for (frame, (input, want)) in pairs.iter().enumerate() {
            let mut syn: [Word16; L_FRAME] = input
                .clone()
                .try_into()
                .expect("a post-filter frame is L_FRAME samples");
            filter.process(&mut ctx, MODE_MR74, &mut syn, &az);
            samples += compare("postfilter", frame, &syn, want);
        }

        assert_eq!(samples, 4 * L_FRAME, "640 samples compared");
    }

    #[test]
    fn post_process_matches_the_reference_over_four_replayed_frames() {
        let pairs = frames("postproc");
        assert_eq!(pairs.len(), 4, "the oracle replays four frames");

        let mut ctx = ctx();
        let mut post = PostProcessor::new();
        let mut samples = 0;

        for (frame, (input, want)) in pairs.iter().enumerate() {
            let mut signal = input.clone();
            post.process(&mut ctx, &mut signal);
            samples += compare("postproc", frame, &signal, want);
        }

        assert_eq!(samples, 4 * L_FRAME, "640 samples compared");
    }

    #[test]
    fn the_replayed_inputs_are_the_stream_the_oracle_drew() {
        // The fixture states both the seed and the samples it produced. If the
        // two ever disagree, the committed inputs are not what the reference
        // was fed, and every comparison above is against the wrong signal.
        for (section, seed) in [("postfilter", 13579i16), ("postproc", 2222)] {
            let drawn = vectors::noise(seed, 4 * L_FRAME, 4);
            let dumped: Vec<Word16> = frames(section)
                .into_iter()
                .flat_map(|(input, _)| input)
                .collect();
            assert_eq!(dumped.len(), 4 * L_FRAME, "{section}: input length");
            assert_eq!(dumped, drawn, "{section}: dumped inputs vs. the seed");
        }
    }

    // ── properties a shared-assumption oracle cannot reach ───────────────────

    #[test]
    fn the_post_filter_carries_state_across_frames() {
        // The oracle and this module could agree on the arithmetic and still
        // both drop a memory only if they shared the bug — but a filter that
        // carries nothing produces the *same* output from a fresh instance,
        // which this catches independently of the fixture.
        let az = az_of("postfilter");
        let pairs = frames("postfilter");

        let mut ctx = ctx();
        let mut stateful = PostFilter::new();
        let mut differs = 0;

        for (frame, (input, _)) in pairs.iter().enumerate() {
            let mut carried: [Word16; L_FRAME] = input.clone().try_into().expect("frame");
            stateful.process(&mut ctx, MODE_MR74, &mut carried, &az);

            let mut fresh: [Word16; L_FRAME] = input.clone().try_into().expect("frame");
            PostFilter::new().process(&mut ctx, MODE_MR74, &mut fresh, &az);

            if frame > 0 && carried != fresh {
                differs += 1;
            }
        }

        assert_eq!(differs, 3, "every frame after the first depends on its predecessor");
    }

    #[test]
    fn resetting_returns_both_filters_to_their_first_frame_behaviour() {
        let az = az_of("postfilter");
        let pairs = frames("postfilter");

        let mut ctx = ctx();
        let mut filter = PostFilter::new();
        let mut first: [Word16; L_FRAME] = pairs[0].0.clone().try_into().expect("frame");
        filter.process(&mut ctx, MODE_MR74, &mut first, &az);
        for (input, _) in &pairs[1..] {
            let mut syn: [Word16; L_FRAME] = input.clone().try_into().expect("frame");
            filter.process(&mut ctx, MODE_MR74, &mut syn, &az);
        }
        filter.reset();
        let mut again: [Word16; L_FRAME] = pairs[0].0.clone().try_into().expect("frame");
        filter.process(&mut ctx, MODE_MR74, &mut again, &az);
        assert_eq!(first, again, "Post_Filter_reset is not complete");

        let proc_pairs = frames("postproc");
        let mut post = PostProcessor::new();
        let mut first = proc_pairs[0].0.clone();
        post.process(&mut ctx, &mut first);
        for (input, _) in &proc_pairs[1..] {
            let mut signal = input.clone();
            post.process(&mut ctx, &mut signal);
        }
        post.reset();
        let mut again = proc_pairs[0].0.clone();
        post.process(&mut ctx, &mut again);
        assert_eq!(first, again, "Post_Process_reset is not complete");
    }

    #[test]
    fn the_two_high_rates_select_a_different_pair_of_expansion_factors() {
        // Getting the gamma selection wrong detunes a rate without breaking
        // anything visible, and the fixture only covers 7.40 — so the branch
        // itself is asserted here rather than assumed.
        assert_eq!(expansion_factors(MODE_MR102), expansion_factors(MODE_MR122));
        for low in 0..6u8 {
            assert_eq!(expansion_factors(low), expansion_factors(0), "mode {low}");
            assert_ne!(
                expansion_factors(low),
                expansion_factors(MODE_MR122),
                "mode {low} must not use the 12.2 factors"
            );
        }

        let az = az_of("postfilter");
        let pairs = frames("postfilter");
        let mut ctx = ctx();

        let mut at_74: [Word16; L_FRAME] = pairs[0].0.clone().try_into().expect("frame");
        PostFilter::new().process(&mut ctx, MODE_MR74, &mut at_74, &az);
        let mut at_122: [Word16; L_FRAME] = pairs[0].0.clone().try_into().expect("frame");
        PostFilter::new().process(&mut ctx, MODE_MR122, &mut at_122, &az);
        assert_ne!(at_74, at_122, "the mode branch has no effect on the output");
    }

    #[test]
    fn the_denominator_tables_are_the_pair_the_reference_defines() {
        // `gamma4` and `gamma3_MR122` are numerically identical in the
        // reference and `gamma4_MR122` is not. A port that collapses the first
        // pair into one symbol is fine; one that collapses all three is the
        // silent detuning this asserts against.
        assert_eq!(GAMMA4, GAMMA3_MR122, "both are 0.7^n");
        assert_ne!(GAMMA4_MR122, GAMMA3_MR122, "0.75^n is not 0.7^n");
        for table in [&GAMMA3, &GAMMA3_MR122, &GAMMA4, &GAMMA4_MR122] {
            assert!(
                table.windows(2).all(|w| w[0] > w[1]) && table[9] > 0,
                "an expansion factor table decays strictly and stays positive"
            );
        }
    }

    #[test]
    fn post_processing_rejects_a_constant_input() {
        // A 60 Hz high-pass must drive a DC input to nothing. This catches a
        // sign flipped in the feedback, which leaves a stable filter with the
        // wrong passband and output that still looks like speech.
        let mut ctx = ctx();
        let mut post = PostProcessor::new();
        let mut signal = [Word16(4000); 8 * L_FRAME];
        post.process(&mut ctx, &mut signal);
        let tail = signal[7 * L_FRAME..].iter().map(|v| i32::from(v.0).abs()).max();
        assert!(
            tail.expect("non-empty tail") < 40,
            "DC survived the high-pass: {tail:?}"
        );
    }

    #[test]
    fn post_processing_is_independent_of_where_a_frame_is_split() {
        // The state is per sample, not per call, so a frame processed in two
        // halves must give the same samples as one call. A memory updated once
        // per call instead of once per sample passes the fixture and fails here.
        let mut ctx = ctx();
        let input = frames("postproc")[0].0.clone();

        let mut whole = input.clone();
        PostProcessor::new().process(&mut ctx, &mut whole);

        let mut split = input;
        let mut post = PostProcessor::new();
        let (head, tail) = split.split_at_mut(37);
        post.process(&mut ctx, head);
        post.process(&mut ctx, tail);

        assert_eq!(whole, split);
    }

    // ── the temporary private copies of the synthesis primitives ─────────────
    //
    // Delete these with the TODO block above. They exist so that a failure in
    // the two sections this module owns localises to the post-filter's own
    // ordering and state rather than to a borrowed primitive.

    #[test]
    fn a_flat_filter_leaves_the_residual_and_synthesis_untouched() {
        // `Residu` and `Syn_filt` are scale-preserving: with A(z) = 1 both are
        // the identity. A Q-format slip in either shows up here as a signal
        // scaled by a power of two, which no amount of listening would catch.
        let mut ctx = ctx();
        let flat = {
            let mut a = [Word16(0); MP1];
            a[0] = Word16(4096);
            a
        };
        let signal: Vec<Word16> = (0..L_SUBFR + M)
            .map(|i| Word16(i16::try_from(i).expect("small") * 137 - 2000))
            .collect();

        let mut residual = vec![Word16(0); L_SUBFR];
        lp_residual(&mut ctx, &flat, &signal, &mut residual);
        assert_eq!(residual, signal[M..], "Residu is not the identity at A(z) = 1");

        let mut synthesised = vec![Word16(0); L_SUBFR];
        let memory = synthesis_filter(
            &mut ctx,
            &flat,
            &residual,
            &mut synthesised,
            &[Word16(0); M],
        );
        assert_eq!(synthesised, residual, "Syn_filt is not the identity at A(z) = 1");
        assert_eq!(
            memory.as_slice(),
            &synthesised[L_SUBFR - M..],
            "the returned memory is not the last M outputs"
        );
    }
}
