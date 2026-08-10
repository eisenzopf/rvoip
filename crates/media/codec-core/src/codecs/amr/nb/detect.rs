//! Source characteristic detection for the AMR-NB decoder.
//!
//! Implements TS 26.073 `Bgn_scd` (`bgnscd.c`), the `gmed_n` running median
//! (`gmed_n.c`), `lsp_avg` (`lsp_avg.c`) and `Int_lsf` (`int_lsf.c`).
//!
//! None of this shapes a sample directly. It decides *what kind of signal the
//! decoder is currently reproducing* — speech or background noise, voiced or
//! not — and those two flags then gate the pitch-gain limiter, the codebook
//! gain smoother, the phase dispersion lock and the excitation control. A
//! detector that is off by one frame produces audio that is correct in
//! structure and wrong in every gated decision.
//!
//! # The flags are read one frame late, on purpose
//!
//! `Decoder_amr` calls the detector *after* the four-subframe loop, but the
//! subframe loop reads `inBackgroundNoise` and `voicedHangover` — so what it
//! reads is the previous frame's verdict. The reference comments this as
//! "valid for use in next frame if BFI". [`SourceDetector`] therefore latches
//! both, and [`SourceDetector::background_noise`] /
//! [`SourceDetector::voiced_hangover`] are what a subframe should consult.
//! Calling [`SourceDetector::update`] before the subframe loop instead of after
//! it would advance both by a frame and change every gated decision.
//!
//! # Validated by
//!
//! `nb_stages.txt` sections `bgnscd`, `lspavg` and `intlsf`, produced by
//! `tools/amrnb_stage_oracle.c` driving the reference's own `Bgn_scd`,
//! `lsp_avg` and `Int_lsf`.

use super::decoder_tables::MEAN_LSF_5;
use crate::fixed_point::arith::{add, extract_h, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_msu};
use crate::fixed_point::shift::{l_shl, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// LP order — ten line frequencies per frame.
const LP_ORDER: usize = 10;

/// Frames of synthesis energy the detector remembers, one Word16 each.
const ENERGY_HISTORY: usize = 60;

/// Where the "recent energy" scan starts, `2*L_ENERGYHIST/3` in the reference.
const RECENT_START: usize = 2 * ENERGY_HISTORY / 3;

/// Frames excluded from the tail of the "overall maximum" scan.
///
/// The scan deliberately stops four short of the end, so the four *newest*
/// frames cannot by themselves lift the maximum above the silence floor.
const MAX_SCAN_SKIP: usize = 4;

/// Above this frame energy the signal is too loud to be background noise.
const FRAME_ENERGY_LIMIT: Word16 = Word16(17578);

/// Below this the frame is silence, which is not noise either.
const LOWER_NOISE_LIMIT: Word16 = Word16(20);

/// Ceiling on the recent maximum for the "quiet lately" arm of the decision.
const UPPER_NOISE_LIMIT: Word16 = Word16(1953);

/// Ceiling on the background-noise hangover counter.
const NOISE_HANGOVER_MAX: Word16 = Word16(30);

/// Ceiling on the frames-since-voiced counter.
const VOICED_HANGOVER_MAX: Word16 = Word16(10);

/// Pitch-gain thresholds for "the recent past was voiced", Q14.
///
/// 0.85, then 0.95, then 1.00: the longer the decoder has been sitting in
/// noise, the more pitch periodicity it demands before calling a frame voiced.
const LTP_LIMIT_BASE: Word16 = Word16(13926);
const LTP_LIMIT_TIGHT: Word16 = Word16(15565);
const LTP_LIMIT_TIGHTEST: Word16 = Word16(16383);

/// Longest median the reference's `gmed_n` supports (`NMAX`).
const MEDIAN_MAX: usize = 9;

/// Smoothing coefficient for the LSF average, 0.16 in Q15 (`EXPCONST`).
const LSF_SMOOTH: Word16 = Word16(5243);

/// Median of an odd-length window, by the reference's `gmed_n`.
///
/// Returns the median **value**, not its index — the reference's own comment
/// says "index of the median value" and is wrong; the code returns
/// `ind[medianIndex]`.
///
/// Q-format: none of its own. It is a selection, so the result carries whatever
/// format the input had — Q14 pitch gains in [`SourceDetector::update`], Q0
/// excitation energies in the excitation controller.
///
/// # A value of exactly −32768 can never be selected
///
/// The running maximum starts at −32767, one above the smallest Word16, so a
/// genuine `i16::MIN` input never wins a round. When every unconsumed entry is
/// `i16::MIN` the inner loop assigns nothing and the *previous* round's winning
/// index is recorded again. Both are reproduced deliberately: the reference is
/// the specification, and its callers only ever pass non-negative gains and
/// energies, so the pathology is unreachable in practice but must not be
/// "fixed" into a different answer.
///
/// # Panics
///
/// If the window is empty, longer than nine, or of even length. The reference
/// documents itself as valid only for odd `n <= NMAX`; an even length would
/// silently pick the upper of the two middle ranks.
#[must_use]
pub fn median(ctx: &mut DspContext, window: &[Word16]) -> Word16 {
    let n = window.len();
    assert!(
        n <= MEDIAN_MAX && n % 2 == 1,
        "gmed_n is defined for an odd window of at most {MEDIAN_MAX}, got {n}"
    );

    let mut pool = [Word16(0); MEDIAN_MAX];
    pool[..n].copy_from_slice(window);

    // Declared once and carried across rounds, exactly as the reference does.
    // See the note above: this is what makes an all-`i16::MIN` window repeat an
    // index instead of walking one.
    let mut winner = 0usize;
    let mut ranked = [0usize; MEDIAN_MAX];

    for slot in &mut ranked[..n] {
        let mut best = Word16(-32767);
        for (j, &candidate) in pool[..n].iter().enumerate() {
            // `>=` rather than `>`, so a tie hands the rank to the *last* index.
            if sub(ctx, candidate, best).0 >= 0 {
                best = candidate;
                winner = j;
            }
        }
        pool[winner] = Word16(i16::MIN);
        *slot = winner;
    }

    // Plain integer halving of a positive length, where the reference writes
    // `shr(n, 1)` on a Word16 loop bound rather than on signal data.
    window[ranked[n / 2]]
}

/// Interpolate LSFs for one subframe, by TS 26.073 `Int_lsf`.
///
/// Weights are 3/4, 1/2, 1/4 of the *previous* frame's fourth-subframe LSFs for
/// subframes 1 to 3, and the new vector alone for subframe 4.
///
/// Q-format: both inputs and the output are LSFs — Q15 of normalised frequency,
/// where 0 Hz is 0 and 4000 Hz is 16384.
///
/// The 3/4 weight is spelled `x - (x >> 2)` rather than as a multiply, because
/// `shr` floors toward negative infinity and a rounding multiply would land on
/// a different LSB.
///
/// # Panics
///
/// If `subframe_start` is not 0, 40, 80 or 120. The reference has no `else`
/// branch and leaves its output buffer untouched — here that would be a silent
/// zero vector, so it is a panic instead.
#[must_use]
pub fn interpolate_lsf(
    ctx: &mut DspContext,
    lsf_old: &[Word16; LP_ORDER],
    lsf_new: &[Word16; LP_ORDER],
    subframe_start: usize,
) -> [Word16; LP_ORDER] {
    let mut out = [Word16(0); LP_ORDER];

    match subframe_start {
        0 => {
            for (i, slot) in out.iter_mut().enumerate() {
                let quarter_old = shr(ctx, lsf_old[i], 2);
                let three_quarter_old = sub(ctx, lsf_old[i], quarter_old);
                let quarter_new = shr(ctx, lsf_new[i], 2);
                *slot = add(ctx, three_quarter_old, quarter_new);
            }
        }
        40 => {
            for (i, slot) in out.iter_mut().enumerate() {
                let half_old = shr(ctx, lsf_old[i], 1);
                let half_new = shr(ctx, lsf_new[i], 1);
                *slot = add(ctx, half_old, half_new);
            }
        }
        80 => {
            for (i, slot) in out.iter_mut().enumerate() {
                let quarter_old = shr(ctx, lsf_old[i], 2);
                let quarter_new = shr(ctx, lsf_new[i], 2);
                let three_quarter_new = sub(ctx, lsf_new[i], quarter_new);
                *slot = add(ctx, quarter_old, three_quarter_new);
            }
        }
        120 => out = *lsf_new,
        other => panic!("Int_lsf is defined at subframe starts 0, 40, 80 and 120, got {other}"),
    }

    out
}

/// The decoder's running LSF average, by TS 26.073 `lsp_avg`.
///
/// # It holds LSFs, not LSPs
///
/// The reference calls the state `lsp_meanSave` and the argument `lsp`, and
/// both names are wrong: `Decoder_amr` passes `lsfState->past_lsf_q`, an LSF
/// vector, at both call sites. Feeding it cosine-domain LSPs would produce a
/// mean in the wrong domain that the codebook gain smoother would then compare
/// against real LSFs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsfAverage {
    mean: [Word16; LP_ORDER],
}

impl Default for LsfAverage {
    fn default() -> Self {
        Self::new()
    }
}

impl LsfAverage {
    /// The reset state: the 5-split quantiser's long-term mean LSF.
    ///
    /// `lsp_avg.c` includes `q_plsf_5.tab`, so this seeds from [`MEAN_LSF_5`]
    /// for **every** mode, not only 12.2 kbit/s. The 3-split mean is a
    /// different vector and using it here would shift the first several frames
    /// of the codebook gain smoother in the seven other modes.
    #[must_use]
    pub fn new() -> Self {
        Self {
            mean: MEAN_LSF_5.map(Word16),
        }
    }

    /// The current average, LSF Q15.
    #[must_use]
    pub const fn mean(&self) -> &[Word16; LP_ORDER] {
        &self.mean
    }

    /// Fold one frame's decoded LSFs into the average.
    ///
    /// Q-format: `lsf` is LSF Q15; the accumulator is Q31; the stored mean is
    /// LSF Q15 again.
    ///
    /// Called **once per frame**, never per subframe, at the end of the frame.
    /// The header describes this as an eight-frame average; it is really a
    /// first-order smoother with α = 0.16, so its effective memory is about six
    /// frames. Collapsing the three steps into a single `mult(mean, 27525)`
    /// would give the same curve and a different last bit.
    pub fn update(&mut self, ctx: &mut DspContext, lsf: &[Word16; LP_ORDER]) {
        for (slot, &fresh) in self.mean.iter_mut().zip(lsf.iter()) {
            let mut acc = l_deposit_h(*slot);
            acc = l_msu(ctx, acc, LSF_SMOOTH, *slot);
            acc = l_mac(ctx, acc, LSF_SMOOTH, fresh);
            *slot = round(ctx, acc);
        }
    }
}

/// Background-noise source characteristic detector, by TS 26.073 `Bgn_scd`.
///
/// An energy detector floating on top of a 60-frame history rather than a real
/// voice activity detector, as the reference's own comment concedes. It is
/// looking for a frame that is quiet but not silent, over a stretch that has
/// been quiet but not silent, and it requires two consecutive such frames
/// before it will say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDetector {
    /// Synthesis frame energies, oldest first; the newest lands at the end.
    energy_history: [Word16; ENERGY_HISTORY],
    /// Consecutive noise-like frames, capped at 30.
    noise_hangover: Word16,
    /// Frames since the last voiced one, capped at 10.
    voiced_hangover: Word16,
    /// Last verdict, latched for the next frame's subframe loop.
    background_noise: bool,
}

impl Default for SourceDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceDetector {
    /// The reset state: no history, no hangover, not in noise.
    ///
    /// This covers `Bgn_scd_reset` *and* the two fields `Decoder_amr_reset`
    /// zeroes on the detector's behalf. They always reset together — both are
    /// unconditional in `Decoder_amr_reset` — so keeping them in one struct
    /// cannot desynchronise them.
    ///
    /// Note that a single SID frame destroys all 60 frames of history:
    /// `Decoder_amr` calls `Decoder_amr_reset(st, MRDTX)` on every non-speech
    /// frame, and `Bgn_scd_reset` inside it is not mode-gated. A decoder that
    /// resets only at start-up will diverge on the first speech frame after any
    /// DTX period.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            energy_history: [Word16(0); ENERGY_HISTORY],
            noise_hangover: Word16(0),
            voiced_hangover: Word16(0),
            background_noise: false,
        }
    }

    /// The latched decision: `true` when the last frame was judged background
    /// noise.
    ///
    /// The reference's header comments the polarity backwards; the code and
    /// every caller agree that 1 means background noise.
    #[must_use]
    pub const fn background_noise(&self) -> bool {
        self.background_noise
    }

    /// Frames since the last voiced frame, 0 to 10.
    #[must_use]
    pub const fn voiced_hangover(&self) -> Word16 {
        self.voiced_hangover
    }

    /// Classify one synthesised frame and advance both hangover counters.
    ///
    /// Q-format: `ltp_gains` are Q14 pitch gains, newest last, frozen by the
    /// caller across bad frames; `synthesis` is the Q0 synthesis frame.
    ///
    /// Returns the new decision, which is also latched — see the module header
    /// for why callers should read the latched value rather than this one
    /// during the frame that produced it.
    pub fn update(
        &mut self,
        ctx: &mut DspContext,
        ltp_gains: &[Word16; MEDIAN_MAX],
        synthesis: &[Word16; super::L_FRAME],
    ) -> bool {
        let current = Self::frame_energy(ctx, synthesis);

        let mut quietest = Word16(i16::MAX);
        for &past in &self.energy_history {
            if sub(ctx, past, quietest).0 < 0 {
                quietest = past;
            }
        }
        // A margin of 16 over the quietest frame ever seen. Saturating: any
        // floor above 2047 pins this at 32767, which makes the first arm of the
        // decision below always true.
        let noise_floor = shl(ctx, quietest, 4);

        // Deliberately asymmetric scans. The overall maximum ignores the four
        // newest frames; the recent maximum looks at the newest twenty. Neither
        // bound is a tidy-up candidate.
        let mut loudest = self.energy_history[0];
        for &past in &self.energy_history[1..ENERGY_HISTORY - MAX_SCAN_SKIP] {
            if sub(ctx, loudest, past).0 < 0 {
                loudest = past;
            }
        }

        let mut loudest_recent = self.energy_history[RECENT_START];
        for &past in &self.energy_history[RECENT_START + 1..] {
            if sub(ctx, loudest_recent, past).0 < 0 {
                loudest_recent = past;
            }
        }

        // Not silence, not sustained volume, and either below the floor we have
        // learned or quiet for the last twenty frames. Rust's `&&`/`||`
        // short-circuit exactly as the C does.
        let noise_like = sub(ctx, loudest, LOWER_NOISE_LIMIT).0 > 0
            && sub(ctx, current, FRAME_ENERGY_LIMIT).0 < 0
            && sub(ctx, current, LOWER_NOISE_LIMIT).0 > 0
            && (sub(ctx, current, noise_floor).0 < 0
                || sub(ctx, loudest_recent, UPPER_NOISE_LIMIT).0 < 0);

        if noise_like {
            let bumped = add(ctx, self.noise_hangover, Word16(1));
            self.noise_hangover = if sub(ctx, bumped, NOISE_HANGOVER_MAX).0 > 0 {
                NOISE_HANGOVER_MAX
            } else {
                bumped
            };
        } else {
            self.noise_hangover = Word16(0);
        }

        // Two consecutive noise-like frames, not one — the reference calls this
        // acting "somewhat cautiously".
        let inbg = sub(ctx, self.noise_hangover, Word16(1)).0 > 0;

        // Push only now: every scan above saw frames k-60 through k-1.
        self.energy_history.copy_within(1.., 0);
        self.energy_history[ENERGY_HISTORY - 1] = current;

        // The reference writes two sequential `if`s, where the second
        // overwrites the first — an `else if` chain in *that* order would be
        // wrong. Reversed, it is exactly equivalent, because `> 15` implies
        // `> 8` for a counter that only ever holds 0..=30. Both comparisons are
        // still evaluated, and in the reference's order, so the operator flags
        // land the same way.
        let past_first_threshold = sub(ctx, self.noise_hangover, Word16(8)).0 > 0;
        let past_second_threshold = sub(ctx, self.noise_hangover, Word16(15)).0 > 0;
        let ltp_limit = if past_second_threshold {
            LTP_LIMIT_TIGHTEST
        } else if past_first_threshold {
            LTP_LIMIT_TIGHT
        } else {
            LTP_LIMIT_BASE
        };

        // The five-tap verdict is always computed, even when the nine-tap one
        // replaces it — both medians consume the same operator context.
        let recent_voiced = {
            let recent_median = median(ctx, &ltp_gains[4..]);
            sub(ctx, recent_median, ltp_limit).0 > 0
        };

        // Deep in noise the decision is re-taken over the whole history. The
        // reference assigns in *both* arms, so this can clear a positive as
        // well as set one; an override that only ever sets would hold
        // `voiced_hangover` at zero through arbitrarily long noise.
        let voiced = if sub(ctx, self.noise_hangover, Word16(20)).0 > 0 {
            let full_median = median(ctx, ltp_gains);
            sub(ctx, full_median, ltp_limit).0 > 0
        } else {
            recent_voiced
        };

        if voiced {
            self.voiced_hangover = Word16(0);
        } else {
            let bumped = add(ctx, self.voiced_hangover, Word16(1));
            self.voiced_hangover = if sub(ctx, bumped, VOICED_HANGOVER_MAX).0 > 0 {
                VOICED_HANGOVER_MAX
            } else {
                bumped
            };
        }

        self.background_noise = inbg;
        inbg
    }

    /// Frame energy as the detector measures it: `2*Σx²`, quadrupled, top half.
    ///
    /// Q-format: Q0 samples in, a Word16 energy index out.
    ///
    /// Both saturations are load-bearing and neither is rare: the accumulator
    /// pins at `i32::MAX` once `Σx²` passes 2³⁰ (frame RMS around 2591), and the
    /// `<< 2` pins it once `Σx²` passes 2²⁸ (RMS around 1295). A full-scale
    /// frame sums to 3.4e11, which does not fit a Word32 at all.
    ///
    /// What matters is that the clamp happens rather than *where*. Every term
    /// is non-negative, so the running sum only climbs — an accumulator that
    /// widened to `i64` and clamped once at the end would give the identical
    /// Word16, and the frequently repeated claim that it would not is wrong.
    /// An accumulator that *wraps* is a different matter: it turns the loudest
    /// possible frame into an arbitrary small energy, and the detector then
    /// treats a shout as background noise.
    fn frame_energy(ctx: &mut DspContext, synthesis: &[Word16; super::L_FRAME]) -> Word16 {
        let mut acc = Word32(0);
        for &sample in synthesis {
            acc = l_mac(ctx, acc, sample, sample);
        }
        extract_h(l_shl(ctx, acc, 2))
    }
}

#[cfg(test)]
mod tests {
    use super::super::vectors::{rows, Row};
    use super::super::L_FRAME;
    use super::*;

    /// Read a fixture row of exactly `len` `Word16`s.
    fn words(row: &Row, label: &str, len: usize) -> Vec<Word16> {
        assert_eq!(row.label, label, "expected a {label:?} row, got {:?}", row.label);
        let v = row.words();
        assert_eq!(v.len(), len, "{label} row should carry {len} values");
        v
    }

    fn array<const N: usize>(v: &[Word16]) -> [Word16; N] {
        let mut out = [Word16(0); N];
        out.copy_from_slice(v);
        out
    }

    #[test]
    fn background_noise_detection_is_bit_exact_against_ts26073() {
        let rows = rows("bgnscd");
        assert_eq!(rows[0].label, "seed", "the bgnscd section should open with its seed");

        let mut detector = SourceDetector::new();
        let mut ctx = DspContext::default();
        let mut compared = 0;

        // One replay: the detector's 60-frame history and both hangover
        // counters carry from case to case, so the rows only mean anything in
        // order.
        for (n, frame) in rows[1..].chunks(3).enumerate() {
            let gains = words(&frame[0], "ltp", MEDIAN_MAX);
            let synth = words(&frame[1], "syn", L_FRAME);
            assert_eq!(frame[2].label, "step", "each bgnscd case ends with a step row");

            let want = frame[2].i16s();
            assert_eq!(want.len(), 2, "a step row is `bgn hangover`");

            let bgn = detector.update(&mut ctx, &array(&gains), &array(&synth));

            assert_eq!(
                i16::from(bgn),
                want[0],
                "bgnscd case {n}: decision {} but the reference gives {}",
                i16::from(bgn),
                want[0]
            );
            assert_eq!(
                detector.voiced_hangover().0,
                want[1],
                "bgnscd case {n}: voiced hangover {} but the reference gives {}",
                detector.voiced_hangover().0,
                want[1]
            );
            assert_eq!(
                i16::from(detector.background_noise()),
                want[0],
                "bgnscd case {n}: the latched decision disagrees with the returned one"
            );
            compared += 1;
        }

        assert_eq!(compared, 10, "compared {compared} bgnscd cases, expected 10");
    }

    #[test]
    fn the_lsf_average_is_bit_exact_against_ts26073() {
        let rows = rows("lspavg");
        assert_eq!(rows[0].label, "seed", "the lspavg section should open with its seed");

        let mut average = LsfAverage::new();
        let mut ctx = DspContext::default();
        let mut compared = 0;

        // Also a replay: each `mean` row is the state after folding in every
        // preceding `lsf` row.
        for (n, case) in rows[1..].chunks(2).enumerate() {
            let lsf = words(&case[0], "lsf", LP_ORDER);
            let want = words(&case[1], "mean", LP_ORDER);

            average.update(&mut ctx, &array(&lsf));

            for (i, (&got, &expected)) in average.mean().iter().zip(want.iter()).enumerate() {
                assert_eq!(
                    got.0, expected.0,
                    "lspavg case {n}: mean[{i}] = {} but the reference gives {}",
                    got.0, expected.0
                );
            }
            compared += 1;
        }

        assert_eq!(compared, 10, "compared {compared} lspavg cases, expected 10");
    }

    #[test]
    fn lsf_interpolation_is_bit_exact_against_ts26073() {
        let rows = rows("intlsf");
        assert_eq!(rows[0].label, "seed", "the intlsf section should open with its seed");

        let lsf_old: [Word16; LP_ORDER] = array(&words(&rows[1], "old", LP_ORDER));
        let lsf_new: [Word16; LP_ORDER] = array(&words(&rows[2], "new", LP_ORDER));

        let mut ctx = DspContext::default();
        let mut compared = 0;

        for case in rows[3..].chunks(2) {
            assert_eq!(case[0].label, "case", "each intlsf case opens with its subframe start");
            let start = usize::try_from(case[0].ints()[0]).expect("a subframe start is non-negative");
            let want = words(&case[1], "out", LP_ORDER);

            let got = interpolate_lsf(&mut ctx, &lsf_old, &lsf_new, start);

            for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                assert_eq!(
                    g.0, w.0,
                    "intlsf at i_subfr {start}: out[{i}] = {} but the reference gives {}",
                    g.0, w.0
                );
            }
            compared += 1;
        }

        assert_eq!(compared, 4, "compared {compared} intlsf cases, expected 4");
    }

    /// A small deterministic pseudo-random source, so the property tests below
    /// sweep more than one shape without depending on a crate.
    fn splitmix(state: &mut u64) -> u32 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z >> 32) as u32
    }

    #[test]
    fn the_median_is_a_real_order_statistic() {
        // The oracle only ever feeds `median` non-negative gains, so it cannot
        // tell a correct selection from one that is merely consistent. Sorting
        // is an independent definition: for any window that avoids the
        // `i16::MIN` pathology, the two must agree.
        let mut ctx = DspContext::default();
        let mut state = 0x5EED_1234_u64;
        let mut compared = 0;

        for n in [1usize, 3, 5, 7, 9] {
            for _ in 0..400 {
                let window: Vec<Word16> = (0..n)
                    .map(|_| {
                        let v = i32::try_from(splitmix(&mut state) % 65535).expect("small") - 32767;
                        Word16(i16::try_from(v).expect("in range by construction"))
                    })
                    .collect();

                let mut sorted: Vec<i16> = window.iter().map(|w| w.0).collect();
                sorted.sort_unstable();

                assert_eq!(
                    median(&mut ctx, &window).0,
                    sorted[n / 2],
                    "median of {window:?} disagrees with the sorted middle"
                );
                compared += 1;
            }
        }

        assert_eq!(compared, 2000, "compared {compared} windows, expected 2000");
    }

    #[test]
    fn the_median_reproduces_the_reference_i16_min_pathology() {
        // `max` starting at -32767 means an `i16::MIN` entry never wins a
        // round. Once every unconsumed entry is `i16::MIN` the inner loop
        // assigns nothing, the carried index repeats, and the ranking stops
        // being a permutation — so the "median" is not the sorted middle.
        //
        // A uniform window cannot show this, because every candidate answer is
        // the same number. These mixed windows can, and all three expected
        // values come from running TS 26.073's own `gmed_n`.
        let mut ctx = DspContext::default();

        // Sorted middle is `i16::MIN`; the reference returns 5, because after
        // round 0 the index stops moving and every later rank repeats index 2.
        let window = [Word16(i16::MIN), Word16(i16::MIN), Word16(5), Word16(i16::MIN), Word16(i16::MIN)];
        assert_eq!(median(&mut ctx, &window).0, 5);

        // -32767 *is* selectable, so it takes rank 1 and the repeat starts one
        // round later — landing on a different wrong answer.
        let window = [Word16(i16::MIN), Word16(-32767), Word16(5), Word16(i16::MIN), Word16(i16::MIN)];
        assert_eq!(median(&mut ctx, &window).0, -32767);

        let window = [
            Word16(i16::MIN),
            Word16(i16::MIN),
            Word16(i16::MIN),
            Word16(i16::MIN),
            Word16(7),
            Word16(9),
            Word16(i16::MIN),
            Word16(i16::MIN),
            Word16(i16::MIN),
        ];
        assert_eq!(median(&mut ctx, &window).0, 7);

        // A uniform floor still terminates and still answers in-range.
        let floor = [Word16(i16::MIN); MEDIAN_MAX];
        assert_eq!(median(&mut ctx, &floor).0, i16::MIN);

        // With one genuine `i16::MIN` among ordinary values the ranking is
        // still a permutation, so the answer agrees with a sort.
        let window = [Word16(i16::MIN), Word16(5), Word16(-9), Word16(400), Word16(7)];
        let mut sorted: Vec<i16> = window.iter().map(|w| w.0).collect();
        sorted.sort_unstable();
        assert_eq!(median(&mut ctx, &window).0, sorted[2]);
    }

    #[test]
    fn a_tie_hands_the_rank_to_the_last_index() {
        // `>=` rather than `>` in the inner loop, so duplicates are consumed in
        // a definite order rather than looping forever on the same index. Two
        // maxima and a distinct middle: whichever duplicate is ranked first,
        // rank 2 must still be 7.
        let mut ctx = DspContext::default();
        let window = [Word16(7), Word16(9), Word16(1), Word16(9), Word16(3)];
        assert_eq!(median(&mut ctx, &window).0, 7);

        let window = [Word16(100); 5];
        assert_eq!(median(&mut ctx, &window).0, 100);
    }

    #[test]
    fn lsf_interpolation_carries_the_weights_it_claims() {
        // The oracle's four cases use one fixed pair of endpoints, so a swapped
        // subframe 1 and 3 — which is exactly the mistake the reference's
        // asymmetric `x - (x>>2)` form invites — would only show up as two
        // wrong rows. This checks the weights themselves, over many endpoint
        // pairs, against exact rational arithmetic.
        //
        // Each subframe is `((4-k)*old + k*new) / 4` for k = 1, 2, 3, 4, and
        // every `shr` floors. Working in quarters, the flooring costs at most
        // one whole LSB (subframe 2 drops the low bit of both endpoints, so it
        // can be four quarters low) and can gain at most three quarters where
        // the two floors pull in opposite directions.
        let mut ctx = DspContext::default();
        let mut state = 0xC0FF_EE00_u64;
        let mut compared = 0;

        for _ in 0..500 {
            let mut lsf_old = [Word16(0); LP_ORDER];
            let mut lsf_new = [Word16(0); LP_ORDER];
            for i in 0..LP_ORDER {
                lsf_old[i] = Word16(i16::try_from(splitmix(&mut state) % 16385).expect("in range"));
                lsf_new[i] = Word16(i16::try_from(splitmix(&mut state) % 16385).expect("in range"));
            }

            for (k, start) in [0usize, 40, 80, 120].into_iter().enumerate() {
                let got = interpolate_lsf(&mut ctx, &lsf_old, &lsf_new, start);
                let weight = i32::try_from(k).expect("small") + 1;

                for i in 0..LP_ORDER {
                    let old = i32::from(lsf_old[i].0);
                    let new = i32::from(lsf_new[i].0);
                    let exact_quarters = (4 - weight) * old + weight * new;
                    let deviation = i32::from(got[i].0) * 4 - exact_quarters;
                    assert!(
                        (-4..=3).contains(&deviation),
                        "i_subfr {start}: coefficient {i} is {deviation} quarters off \
                         {exact_quarters}/4 — the subframe weight is wrong, not just rounded"
                    );

                    // Containment: an interpolant cannot exceed either
                    // endpoint, and can only undershoot by the flooring LSB.
                    let (lo, hi) = (old.min(new), old.max(new));
                    assert!(
                        (lo - 1..=hi).contains(&i32::from(got[i].0)),
                        "i_subfr {start}: coefficient {i} left [{lo}, {hi}]"
                    );
                }
                compared += 1;
            }

            assert_eq!(
                interpolate_lsf(&mut ctx, &lsf_old, &lsf_new, 120),
                lsf_new,
                "the fourth subframe is a plain copy of the new vector"
            );
        }

        assert_eq!(compared, 2000, "compared {compared} interpolations, expected 2000");
    }

    #[test]
    fn the_lsf_average_contracts_toward_its_input_without_overshooting() {
        // The oracle's ten steps all move the mean the same direction, so they
        // cannot show that the smoother is stable. This can: feeding a constant
        // vector must never step past it, and the distance must never grow.
        let mut ctx = DspContext::default();
        let mut state = 0xBEEF_0007_u64;

        for _ in 0..40 {
            let mut average = LsfAverage::new();
            let mut target = [Word16(0); LP_ORDER];
            for slot in &mut target {
                *slot = Word16(i16::try_from(splitmix(&mut state) % 16385).expect("in range"));
            }

            let mut previous = *average.mean();
            for step in 0..200 {
                average.update(&mut ctx, &target);
                for i in 0..LP_ORDER {
                    let was = i32::from(previous[i].0) - i32::from(target[i].0);
                    let now = i32::from(average.mean()[i].0) - i32::from(target[i].0);
                    assert!(
                        now.abs() <= was.abs(),
                        "step {step}: coefficient {i} moved away from its target"
                    );
                    assert!(
                        was.signum() * now.signum() >= 0,
                        "step {step}: coefficient {i} overshot its target"
                    );
                }
                previous = *average.mean();
            }

            // 0.16 per step leaves a dead zone three LSBs wide, because the
            // increment floors to zero once the gap is small enough. Converging
            // to *exactly* the target is not something to assert.
            for (i, (&settled, &want)) in average.mean().iter().zip(target.iter()).enumerate() {
                let gap = i32::from(settled.0) - i32::from(want.0);
                assert!(gap.abs() <= 3, "coefficient {i} settled {gap} away from its target");
            }
        }
    }

    /// A square wave, the simplest frame with a flat, exactly known energy.
    fn square(amplitude: i16) -> [Word16; L_FRAME] {
        let mut synth = [Word16(0); L_FRAME];
        for (i, slot) in synth.iter_mut().enumerate() {
            *slot = Word16(if i % 2 == 0 { amplitude } else { -amplitude });
        }
        synth
    }

    /// A frame quiet enough to be noise but not silence: energy 1757, between
    /// [`LOWER_NOISE_LIMIT`] and [`UPPER_NOISE_LIMIT`].
    fn quiet_frame() -> [Word16; L_FRAME] {
        square(300)
    }

    /// The frame index, 1-based, at which the detector first reports noise.
    fn first_noise_frame(frames: &[[Word16; L_FRAME]], gain: i16) -> Option<usize> {
        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();
        let gains = [Word16(gain); MEDIAN_MAX];
        frames
            .iter()
            .position(|f| detector.update(&mut ctx, &gains, f))
            .map(|i| i + 1)
    }

    #[test]
    fn frame_energy_saturates_where_the_reference_says_it_does() {
        // Four points on the energy curve, taken from TS 26.073's own
        // accumulator. The last one is the whole reason `L_mac` and `L_shl`
        // must stay saturating: a full-scale frame sums to 3.4e11, which does
        // not fit a Word32 at all, and any accumulator that does not clamp
        // produces a small number for the loudest possible input.
        let mut ctx = DspContext::default();
        for (amplitude, expected) in [(300i16, 1757i16), (392, 3001), (700, 9570), (32767, 32767)] {
            let got = SourceDetector::frame_energy(&mut ctx, &square(amplitude));
            assert_eq!(
                got.0, expected,
                "amplitude {amplitude}: energy {} but the reference gives {expected}",
                got.0
            );
        }
    }

    #[test]
    fn a_loud_frame_can_never_be_background_noise() {
        // Every synthesis frame in the fixture is far above FRAMEENERGYLIMIT,
        // so the committed vectors only ever exercise the hangover-zero path.
        // Assert the invariant that makes that so, independently of them.
        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();
        let mut state = 0x1234_ABCD_u64;

        for frame in 0..80 {
            let mut synth = [Word16(0); L_FRAME];
            for slot in &mut synth {
                let v = i16::try_from(splitmix(&mut state) % 4096).expect("in range") - 2048;
                *slot = Word16(v);
            }
            let gains = [Word16(0); MEDIAN_MAX];
            assert!(
                !detector.update(&mut ctx, &gains, &synth),
                "frame {frame}: a full-band frame was called background noise"
            );
            assert!(detector.voiced_hangover().0 <= VOICED_HANGOVER_MAX.0);
        }
        // Zero pitch gain for 80 frames: the counter pins at its ceiling rather
        // than running on.
        assert_eq!(detector.voiced_hangover().0, VOICED_HANGOVER_MAX.0);
    }

    #[test]
    fn the_detector_needs_two_frames_and_the_history_fills_from_the_top() {
        // Start-up is subtle: `loudest` scans slots 0..=55 while the push lands
        // in slot 59, so the gate lifts after five frames rather than after
        // sixty, and the first possible positive is the seventh frame. A port
        // that waits for a full history would silence the detector for over a
        // second of audio; one that scanned all sixty slots would open it four
        // frames early.
        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();
        let gains = [Word16(0); MEDIAN_MAX];
        let synth = quiet_frame();

        let mut first_positive = None;
        for frame in 0..12 {
            if detector.update(&mut ctx, &gains, &synth) && first_positive.is_none() {
                first_positive = Some(frame);
            }
        }
        assert_eq!(
            first_positive,
            Some(6),
            "the first background-noise verdict should be the seventh frame"
        );

        // And the counter climbs to its ceiling rather than past it.
        for _ in 0..40 {
            detector.update(&mut ctx, &gains, &synth);
        }
        assert_eq!(detector.noise_hangover, NOISE_HANGOVER_MAX);
    }

    #[test]
    fn the_voicing_threshold_tightens_once_the_decoder_settles_into_noise() {
        let limits = [LTP_LIMIT_BASE.0, LTP_LIMIT_TIGHT.0, LTP_LIMIT_TIGHTEST.0];
        assert!(
            limits.windows(2).all(|w| w[0] < w[1]),
            "the voicing limits must tighten monotonically, got {limits:?}"
        );

        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();

        // 0.885 in Q14: above the 0.85 limit, below the 0.95 one. The same
        // gains therefore read as voiced early and unvoiced later, which is the
        // whole point of the two sequential threshold tests.
        let gains = [Word16(14500); MEDIAN_MAX];
        let synth = quiet_frame();

        // Slots 0..=55 are still zero for the first five frames, so the noise
        // hangover does not start climbing until frame 6 and reaches 8 at
        // frame 13.
        for frame in 1..=13 {
            detector.update(&mut ctx, &gains, &synth);
            assert_eq!(
                detector.voiced_hangover().0,
                0,
                "frame {frame}: 0.885 clears the 0.85 limit and should read as voiced"
            );
        }
        assert_eq!(detector.noise_hangover.0, 8, "hangover should sit at the first threshold");

        detector.update(&mut ctx, &gains, &synth);
        assert_eq!(detector.noise_hangover.0, 9, "hangover should have crossed 8");
        assert_eq!(
            detector.voiced_hangover().0,
            1,
            "past hangover 8 the limit is 0.95, which 0.885 does not clear"
        );
    }

    #[test]
    fn the_second_voicing_threshold_is_reachable_too() {
        // The two threshold tests are sequential `if`s, so the second silently
        // stops mattering if it is turned into an `else if` — or dropped. A
        // gain of 0.977 in Q14 sits between the 0.95 and 1.00 limits, so it
        // reads as voiced right up to hangover 15 and not after. The reference
        // puts the transition at frame 21.
        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();
        let gains = [Word16(16000); MEDIAN_MAX];
        let synth = quiet_frame();

        for frame in 1..=20 {
            detector.update(&mut ctx, &gains, &synth);
            assert_eq!(
                detector.voiced_hangover().0,
                0,
                "frame {frame}: 0.977 still clears the 0.95 limit"
            );
        }
        assert_eq!(detector.noise_hangover.0, 15, "hangover should sit on the second threshold");

        detector.update(&mut ctx, &gains, &synth);
        assert_eq!(detector.noise_hangover.0, 16);
        assert_eq!(
            detector.voiced_hangover().0,
            1,
            "past hangover 15 the limit is 1.00, which 0.977 does not clear"
        );
    }

    #[test]
    fn the_noise_floor_saturates_once_the_history_fills() {
        // Sustained energy 3001 is above the 2047 at which `shl(_, 4)` clamps.
        // Until the history fills, one slot is still zero, so the floor is zero
        // and the "below the learned floor" arm is dead; the "quiet lately" arm
        // cannot fire either, because 3001 exceeds UPPERNOISELIMIT. The moment
        // the sixtieth slot is written the floor pins at 32767 and the first
        // arm becomes unconditionally true.
        //
        // A non-saturating `<< 4` wraps 3001 to a negative Word16 here and the
        // detector never latches at all. The reference latches at frame 62.
        let frames = vec![square(392); 66];
        assert_eq!(
            first_noise_frame(&frames, 100),
            Some(62),
            "the saturating noise floor should open the detector at frame 62"
        );
    }

    #[test]
    fn a_quiet_recent_window_latches_before_the_loud_frames_leave_history() {
        // This is what the asymmetric scan bounds are for. `maxEnergyLastPart`
        // looks only at the newest twenty frames, so twenty-one quiet frames
        // after a loud stretch are enough — the detector does not wait for all
        // sixty slots to turn over. Widening that scan to the whole history
        // (or narrowing `maxEnergy`'s) delays the latch by forty frames.
        let mut frames = vec![square(700); 25];
        frames.extend(std::iter::repeat_n(square(300), 25));
        assert_eq!(
            first_noise_frame(&frames, 100),
            Some(47),
            "the recent-energy window should open the detector at frame 47"
        );
    }

    #[test]
    fn deep_in_noise_the_nine_tap_median_can_clear_a_voiced_verdict() {
        // The last branch the fixture never reaches. Past hangover 20 the
        // decision is re-taken over all nine gains, and the reference writes it
        // as an if/else that assigns *both* ways — so it can overturn a
        // positive, not merely add one. An `if` without the `else` compiles,
        // passes every committed vector, and holds `voicedHangover` at zero
        // through arbitrarily long stretches of noise.
        //
        // Making that visible needs care: past hangover 20 the limit is already
        // 1.00, so the five-tap median has to clear 16383 while the nine-tap one
        // does not. Three high gains among the newest five carry the five-tap
        // median; four zeros in the older half sink the nine-tap one.
        let mut ctx = DspContext::default();
        let mut detector = SourceDetector::new();
        let synth = quiet_frame();
        let mut gains = [Word16(0); MEDIAN_MAX];
        gains[4] = Word16(17000);
        gains[5] = Word16(17000);
        gains[6] = Word16(17000);

        // Hangover starts climbing at frame 6, so it is 20 at frame 25.
        for frame in 1..=25 {
            detector.update(&mut ctx, &gains, &synth);
            assert_eq!(
                detector.voiced_hangover().0,
                0,
                "frame {frame}: the five-tap median alone should still read voiced"
            );
        }
        assert_eq!(detector.noise_hangover.0, 20, "hangover should sit exactly on the threshold");

        detector.update(&mut ctx, &gains, &synth);
        assert_eq!(detector.noise_hangover.0, 21, "hangover should have crossed 20");
        assert_eq!(
            detector.voiced_hangover().0,
            1,
            "the nine-tap median should have overturned the five-tap verdict"
        );
    }
}
