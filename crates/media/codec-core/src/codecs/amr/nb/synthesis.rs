//! The AMR-NB decoder's time-domain back end: synthesis filtering, gain
//! control, phase dispersion and excitation control.
//!
//! Implements TS 26.073 `syn_filt.c` (`Syn_filt`), `agc.c` (`agc`, `agc2` and
//! their shared energy measure), `residu.c` (`Residu`), `weight_a.c`
//! (`Weight_Ai`), `preemph.c` (`preemphasis`), `ph_disp.c` (`ph_disp`,
//! `ph_disp_reset`, `ph_disp_lock`, `ph_disp_release`), `ex_ctrl.c` (`Ex_ctrl`)
//! and the 9-point median `gmed_n.c` that `Ex_ctrl` needs. The prose is
//! TS 26.090 §6.1 (short-term synthesis) and §6.2 (adaptive phase dispersion).
//!
//! Validated bit-exactly against the `synfilt`, `agc`, `agc2`, `weightai`,
//! `residu`, `preemph`, `phdisp` and `exctrl` sections of
//! `testdata/nb_stages.txt`, which `tools/amrnb_stage_oracle.c` produced by
//! driving the reference's own functions.
//!
//! # Q-formats
//!
//! Signals are Q0 throughout — the decoder works in integer sample units and
//! only the coefficients and gains carry a scale. LP coefficients are Q12 with
//! `a[0] == 4096`; expansion factors and the pre-emphasis and AGC coefficients
//! are Q15; the phase-dispersion impulse responses are Q15; codebook gain is
//! Q1 and LTP gain Q14. Each function restates this on its own doc.
//!
//! # `Syn_filt`'s overflow-retry contract
//!
//! [`synthesis_filter`] does not handle its own overflow. TS 26.073's
//! `dec_amr.c` wraps every decoder-side call in a retry that the caller must
//! reproduce, because the recovery touches the excitation *history*, which this
//! module does not own:
//!
//! 1. Clear `DspContext::overflow` **immediately** before the call. Anything
//!    the preceding phase dispersion, excitation control or `agc2` raised is
//!    deliberately discarded.
//! 2. Call [`synthesis_filter`] with the current memory, keeping the returned
//!    memory *unwritten* for now.
//! 3. If the flag is still clear, commit the returned memory and carry on.
//! 4. If it is set, right-shift the **entire** 194-sample excitation history by
//!    two — `PIT_MAX + L_INTERPOL + L_SUBFR`, which includes the current
//!    subframe — and the current subframe's excitation too, then call
//!    [`synthesis_filter`] again with the *unchanged* memory and commit its
//!    result. `mem_syn` is deliberately not rescaled: the retry filters a
//!    quartered excitation through an un-quartered memory.
//!
//! Two details of that retry read like bugs and are not. It always re-filters
//! the plain enhanced excitation, never the pitch-sharpened `excp` copy that
//! may have been used on the first attempt; and the retry is not itself
//! preceded by a clear, so an overflow *it* raises leaks into the next subframe
//! until that subframe's own step 1.
//!
//! # Which state persists
//!
//! [`AdaptiveGain`], [`Preemphasis`] and [`PhaseDispersion`] all carry state
//! across subframes *and* frames. The synthesis memory is returned rather than
//! stored so its owner can implement the retry above. See each type's docs for
//! its reset value — in particular [`AdaptiveGain`] resets to 1.0, not to zero.

use super::decoder_tables::{PH_IMP_LOW, PH_IMP_LOW_MR795, PH_IMP_MID, PH_IMP_MID_MR795};
use super::lsp::{M, MP1};
use super::math::inv_sqrt;
use super::L_SUBFR;
use crate::fixed_point::arith::{add, extract_h, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_deposit_l, l_mac, l_msu, l_mult, l_sub};
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::{l_shl, l_shr, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_32};

/// Scratch length inside the synthesis filter.
///
/// The reference declares a fixed `Word16 tmp[80]` rather than sizing it to
/// `lg + M`, and every call site fits: 40 + 10 in the decoder, 22 + 10 for the
/// post-filter's impulse response. Kept literal so the assertion below fails
/// on a call the reference would also have failed on.
const SYN_SCRATCH: usize = 80;

/// Mode index of 7.40 kbit/s — excluded from phase dispersion.
const MR74: u8 = 4;
/// Mode index of 7.95 kbit/s — uses its own pair of impulse responses.
const MR795: u8 = 5;
/// Mode index of 10.2 kbit/s — excluded from phase dispersion.
const MR102: u8 = 6;
/// Mode index of 12.2 kbit/s — excluded from phase dispersion.
const MR122: u8 = 7;

/// Depth of the LTP-gain history phase dispersion keeps.
const PHD_GAIN_MEM: usize = 5;

/// LTP gain below which dispersion is considered warranted, 0.6 in Q14.
const PHD_THR1_LTP: Word16 = Word16(9830);

/// LTP gain at or above which dispersion is switched off, 0.9 in Q14.
const PHD_THR2_LTP: Word16 = Word16(14746);

/// Onset detection threshold multiplier, 2.0 in Q13.
const ON_FACT_PLUS1: Word16 = Word16(16384);

/// How many subframes an onset suppresses full dispersion for.
const ON_LENGTH: Word16 = Word16(2);

/// Entries in the excitation-energy history `Ex_ctrl` reads.
pub const EXC_ENERGY_HIST: usize = 9;

// ---------------------------------------------------------------- synthesis --

/// Run the recursion into the reference's `tmp[]` layout: memory first, then
/// the `x.len()` outputs.
///
/// Filtering into scratch and copying out afterwards is not a stylistic
/// choice. Four reference call sites pass the same buffer as both input and
/// output, and one of them additionally passes a slice of that buffer as the
/// memory. Writing results into `y` inside the loop would corrupt inputs the
/// recursion has not read yet.
fn synthesis_recursion(
    ctx: &mut DspContext,
    a: &[Word16],
    x: &[Word16],
    mem: &[Word16; M],
) -> [Word16; SYN_SCRATCH] {
    let lg = x.len();
    assert!(
        lg + M <= SYN_SCRATCH,
        "synthesis filter length {lg} exceeds the reference's fixed scratch"
    );
    assert!(a.len() >= MP1, "synthesis filter needs a[0..=M]");

    let mut tmp = [Word16(0); SYN_SCRATCH];
    tmp[..M].copy_from_slice(mem);

    for i in 0..lg {
        // `a[0]` is 4096, so this is `x[i] << 13` — but the encoder passes
        // weighted coefficients whose leading term is still 4096 while the
        // rest are scaled, so the multiply must stay.
        let mut s = l_mult(ctx, x[i], a[0]);
        for j in 1..=M {
            // `L_msu`, subtract: this is 1/A(z), the sign convention opposite
            // to `Residu`'s. `tmp[M + i - j]` is the reference's `yy[-j]`, the
            // outputs this same loop produced — the recursion feeds itself.
            s = l_msu(ctx, s, a[j], tmp[M + i - j]);
        }
        // Q13 (Q0 * Q12, doubled by `L_mult`) up to Q16. By far the most
        // frequent source of the overflow the caller's retry watches for.
        s = l_shl(ctx, s, 3);
        tmp[M + i] = round(ctx, s);
    }
    tmp
}

/// Copy the filtered samples out and derive the memory an update would store.
fn commit(tmp: &[Word16; SYN_SCRATCH], out: &mut [Word16]) -> [Word16; M] {
    let lg = out.len();
    out.copy_from_slice(&tmp[M..M + lg]);

    // The reference reads the memory back out of `y[]` rather than out of its
    // scratch. Identical values, but preserved because the two differ the
    // moment a caller aliases `y` with something it mutates.
    let mut updated = [Word16(0); M];
    updated.copy_from_slice(&out[lg - M..]);
    updated
}

/// Synthesis filtering through `1/A(z)`, TS 26.073 `Syn_filt`.
///
/// `a` is Q12 with `a[0] == 4096`; `excitation` and `out` are Q0 and must be
/// the same length. Returns the filter memory as of the last `M` output
/// samples — the caller stores it or discards it, which is how the reference's
/// `update` flag is expressed here. Discarding is what the post-filter's
/// impulse-response call and the decoder's *first* attempt per subframe do; see
/// the module header for the overflow-retry protocol that makes the difference
/// matter.
///
/// # Panics
///
/// If `out` and `excitation` differ in length, if the length plus `M` exceeds
/// the reference's fixed 80-sample scratch, if `a` is shorter than `M + 1`, or
/// if fewer than `M` samples are filtered (there would be no memory to return).
pub fn synthesis_filter(
    ctx: &mut DspContext,
    a: &[Word16],
    excitation: &[Word16],
    out: &mut [Word16],
    mem: &[Word16; M],
) -> [Word16; M] {
    assert_eq!(
        out.len(),
        excitation.len(),
        "synthesis in/out length mismatch"
    );
    assert!(out.len() >= M, "a subframe shorter than the filter memory");
    let tmp = synthesis_recursion(ctx, a, excitation, mem);
    commit(&tmp, out)
}

/// [`synthesis_filter`] where the input buffer is also the output buffer.
///
/// The reference reaches this case by passing the same pointer twice; Rust's
/// borrow rules make it a separate entry point. Both share one recursion, so
/// the aliasing behaviour cannot drift between them.
///
/// # Panics
///
/// As [`synthesis_filter`].
pub fn synthesis_filter_in_place(
    ctx: &mut DspContext,
    a: &[Word16],
    signal: &mut [Word16],
    mem: &[Word16; M],
) -> [Word16; M] {
    assert!(
        signal.len() >= M,
        "a subframe shorter than the filter memory"
    );
    let tmp = synthesis_recursion(ctx, a, signal, mem);
    commit(&tmp, signal)
}

/// LP inverse filtering `A(z)`, TS 26.073 `Residu`.
///
/// `a` is Q12; `signal` and `residual` are Q0. **`signal` must begin `M`
/// samples before the first sample to be filtered** — the recursion reads back
/// to `x[-M]`, and the reference's only decoder-side caller supplies that
/// history from the previous frame. So `signal.len() == residual.len() + M`,
/// and `residual[0]` corresponds to `signal[M]`.
///
/// # Panics
///
/// If `signal` does not carry exactly `M` samples of history ahead of the
/// output window, or if `a` is shorter than `M + 1`.
pub fn lp_residual(ctx: &mut DspContext, a: &[Word16], signal: &[Word16], residual: &mut [Word16]) {
    assert_eq!(
        signal.len(),
        residual.len() + M,
        "Residu needs exactly M samples of history before the window"
    );
    assert!(a.len() >= MP1, "Residu needs a[0..=M]");

    for (i, slot) in residual.iter_mut().enumerate() {
        let mut s = l_mult(ctx, signal[M + i], a[0]);
        for j in 1..=M {
            // `L_mac`, add — the opposite sign to `Syn_filt`'s `L_msu`, which
            // is what makes this the inverse rather than the synthesis filter.
            s = l_mac(ctx, s, a[j], signal[M + i - j]);
        }
        s = l_shl(ctx, s, 3);
        *slot = round(ctx, s);
    }
}

/// Bandwidth expansion of an LP filter, TS 26.073 `Weight_Ai`.
///
/// `a` is Q12 with `a[0] == 4096`, `factors` is Q15 and has `M` entries — one
/// per coefficient *after* the leading one. The result is Q12.
///
/// AMR-NB passes an explicit factor vector rather than a scalar `γ` raised to
/// successive powers, which is why there is no `Weight_Az` here: that is the
/// wideband function and it takes a scalar.
///
/// # Panics
///
/// If `a` is shorter than `M + 1` or `factors` shorter than `M`.
#[must_use]
pub fn expand_bandwidth(ctx: &mut DspContext, a: &[Word16], factors: &[i16]) -> [Word16; MP1] {
    assert!(a.len() >= MP1, "Weight_Ai needs a[0..=M]");
    assert!(factors.len() >= M, "Weight_Ai needs M expansion factors");

    let mut expanded = [Word16(0); MP1];
    // The leading coefficient is copied, not multiplied: it stays exactly
    // 4096 so the expanded filter is still monic in Q12.
    expanded[0] = a[0];
    for i in 1..=M {
        let scaled = l_mult(ctx, a[i], Word16(factors[i - 1]));
        expanded[i] = round(ctx, scaled);
    }
    expanded
}

// ------------------------------------------------------------- pre-emphasis --

/// The post-filter's `1 - g·z⁻¹` pre-emphasis, TS 26.073 `preemph.c`.
///
/// Carries one sample across subframes and frames; resets to zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Preemphasis {
    /// The previous block's last *input* sample.
    previous: Word16,
}

impl Preemphasis {
    /// A filter in its reset state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous: Word16(0),
        }
    }

    /// Filter `signal` in place. `signal` is Q0, `coefficient` Q15.
    ///
    /// Runs backward, which is what makes the in-place form correct: each
    /// sample needs its predecessor's *unfiltered* value, so the predecessor
    /// must not have been overwritten yet. The state saved for the next call is
    /// likewise the original last input, not the filtered output — a forward
    /// loop or a filtered carry produces plausible, wrong audio.
    ///
    /// # Panics
    ///
    /// If `signal` is empty.
    pub fn filter(&mut self, ctx: &mut DspContext, signal: &mut [Word16], coefficient: Word16) {
        let len = signal.len();
        assert!(len > 0, "pre-emphasis of an empty block");

        let carried = signal[len - 1];
        for k in (1..len).rev() {
            let feedback = mult(ctx, coefficient, signal[k - 1]);
            signal[k] = sub(ctx, signal[k], feedback);
        }
        let feedback = mult(ctx, coefficient, self.previous);
        signal[0] = sub(ctx, signal[0], feedback);

        self.previous = carried;
    }

    /// The carried sample, for tests and for state inspection.
    #[must_use]
    pub const fn memory(self) -> Word16 {
        self.previous
    }
}

// ------------------------------------------------------------ gain control --

/// Energy of a block, TS 26.073 `agc.c`'s `energy_new` with its `energy_old`
/// fallback.
///
/// Returns roughly `Σx²/8` by two different routes. The fast one accumulates at
/// full scale and probes for saturation by comparing against `MAX_32`
/// *exactly*; only if it hit the rail does it redo the sum on inputs
/// pre-shifted down by two.
///
/// The overflow flag is saved and restored **only** on the saturated branch,
/// exactly as the reference: an accumulation that did not quite reach the rail
/// can still leave the flag set on the way out. That looks like an oversight
/// and is relied upon by nothing, because the decoder clears the flag straight
/// afterwards — but it is observable, so it is reproduced rather than tidied.
fn block_energy(ctx: &mut DspContext, x: &[Word16]) -> Word32 {
    let saved_overflow = ctx.overflow;

    let mut s = l_mult(ctx, x[0], x[0]);
    for &v in &x[1..] {
        s = l_mac(ctx, s, v, v);
    }

    if l_sub(ctx, s, Word32(MAX_32)).0 == 0 {
        ctx.overflow = saved_overflow;
        let mut t = shr(ctx, x[0], 2);
        let mut s = l_mult(ctx, t, t);
        for &v in &x[1..] {
            t = shr(ctx, v, 2);
            s = l_mac(ctx, s, t, t);
        }
        s
    } else {
        l_shr(ctx, s, 4)
    }
}

/// The gain both `agc` and `agc2` derive from a pair of block energies.
///
/// `numerator` is the normalised energy of the signal being scaled and
/// `denominator` that of the reference; `exp` is the difference of their
/// normalisation shifts. The division goes one way and the reciprocal square
/// root the other, so the result is `sqrt(reference / signal)` — the factor
/// that brings the signal up to the reference.
///
/// `exp` is routinely negative, in which case `L_shr` delegates to `L_shl` and
/// may saturate. That is the specified path, not a domain error.
fn energy_ratio_root(
    ctx: &mut DspContext,
    numerator: Word16,
    denominator: Word16,
    exp: i16,
) -> Word16 {
    let mut s = l_deposit_l(div_s(numerator, denominator));
    s = l_shl(ctx, s, 7);
    s = l_shr(ctx, s, exp);
    let s = inv_sqrt(ctx, s);
    let positioned = l_shl(ctx, s, 9);
    round(ctx, positioned)
}

/// The post-filter's automatic gain control, TS 26.073 `agc`.
///
/// Carries the smoothed gain across subframes and frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveGain {
    /// Gain after the last sample of the previous block, Q12.
    past_gain: Word16,
}

impl Default for AdaptiveGain {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveGain {
    /// A gain control in its reset state.
    ///
    /// The reset value is **1.0 in Q12**, not zero: the post-filter's first
    /// subframe must pass its input through at unit gain rather than ramping up
    /// from silence.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            past_gain: Word16(4096),
        }
    }

    /// The carried gain, Q12.
    #[must_use]
    pub const fn past_gain(self) -> Word16 {
        self.past_gain
    }

    /// Rescale `signal` in place so its energy tracks `reference`'s.
    ///
    /// `reference` is the post-filter's input and `signal` its output, both Q0
    /// and the same length; `factor` is the Q15 smoothing coefficient
    /// (`AGC_FAC`). The gain is smoothed *per sample*, so the loop is
    /// inherently sequential and its final value becomes the carried state.
    ///
    /// A silent output short-circuits with the carried gain set to zero, which
    /// makes the next block ramp up from nothing — deliberate, and the only
    /// path that leaves `signal` untouched.
    ///
    /// # Panics
    ///
    /// If the two blocks differ in length or either is empty.
    pub fn scale(
        &mut self,
        ctx: &mut DspContext,
        reference: &[Word16],
        signal: &mut [Word16],
        factor: Word16,
    ) {
        assert_eq!(reference.len(), signal.len(), "AGC block length mismatch");
        assert!(!signal.is_empty(), "AGC of an empty block");

        let energy_out = block_energy(ctx, signal);
        // A plain comparison, as in the reference: not `L_sub`, so it cannot
        // disturb the overflow flag.
        if energy_out.0 == 0 {
            self.past_gain = Word16(0);
            return;
        }

        // The `- 1` is what keeps the normalised output energy in
        // [8192, 16384] while the input's lands in [16384, 32767], so that
        // `div_s`'s numerator never exceeds its denominator.
        let mut exp = sub(ctx, Word16(norm_l(energy_out)), Word16(1));
        let positioned = l_shl(ctx, energy_out, exp.0);
        let normalised_out = round(ctx, positioned);

        let energy_in = block_energy(ctx, reference);
        let step = if energy_in.0 == 0 {
            Word16(0)
        } else {
            let shift = norm_l(energy_in);
            let positioned = l_shl(ctx, energy_in, shift);
            let normalised_in = round(ctx, positioned);
            exp = sub(ctx, exp, Word16(shift));
            let root = energy_ratio_root(ctx, normalised_out, normalised_in, exp.0);
            // 32767, not 32768: the complement of the smoothing factor is one
            // LSB short, deliberately.
            let complement = sub(ctx, Word16(32767), factor);
            mult(ctx, root, complement)
        };

        let mut gain = self.past_gain;
        for slot in signal.iter_mut() {
            gain = mult(ctx, gain, factor);
            gain = add(ctx, gain, step);
            // `extract_h`, i.e. truncation. Rounding here is the single most
            // tempting "improvement" in this module and it is wrong.
            let product = l_mult(ctx, *slot, gain);
            *slot = extract_h(l_shl(ctx, product, 3));
        }
        self.past_gain = gain;
    }
}

/// Rescale `signal` in place to `reference`'s energy, TS 26.073 `agc2`.
///
/// Both blocks are Q0 and the same length. Unlike [`AdaptiveGain::scale`] this
/// has no state and no smoothing: one constant gain is applied to the whole
/// block, and there is no `1 - factor` term. The decoder uses it to match the
/// pitch-sharpened excitation copy to the enhanced excitation's energy.
///
/// A silent `signal` is left exactly as it was.
///
/// # Panics
///
/// If the two blocks differ in length or either is empty.
pub fn match_energy(ctx: &mut DspContext, reference: &[Word16], signal: &mut [Word16]) {
    assert_eq!(reference.len(), signal.len(), "agc2 block length mismatch");
    assert!(!signal.is_empty(), "agc2 of an empty block");

    let energy_out = block_energy(ctx, signal);
    if energy_out.0 == 0 {
        return;
    }

    let mut exp = sub(ctx, Word16(norm_l(energy_out)), Word16(1));
    let positioned = l_shl(ctx, energy_out, exp.0);
    let normalised_out = round(ctx, positioned);

    let energy_in = block_energy(ctx, reference);
    let gain = if energy_in.0 == 0 {
        Word16(0)
    } else {
        let shift = norm_l(energy_in);
        let positioned = l_shl(ctx, energy_in, shift);
        let normalised_in = round(ctx, positioned);
        exp = sub(ctx, exp, Word16(shift));
        energy_ratio_root(ctx, normalised_out, normalised_in, exp.0)
    };

    for slot in signal.iter_mut() {
        let product = l_mult(ctx, *slot, gain);
        *slot = extract_h(l_shl(ctx, product, 3));
    }
}

// -------------------------------------------------------- phase dispersion --

/// Adaptive phase dispersion and total-excitation formation, TS 26.073
/// `ph_disp.c`.
///
/// **Not the wideband algorithm.** AMR-NB selects among four 40-tap Q15
/// impulse responses using a three-level indicator derived from the LTP-gain
/// history and an onset detector, and it forms the total excitation as its last
/// step. The wideband routine of the same name shares neither the tables, the
/// rate mapping, nor the state.
///
/// Every field persists across subframes and frames. All of them are cleared by
/// [`PhaseDispersion::new`], which the decoder's reset calls *including* on the
/// path taken for comfort-noise frames — unlike the synthesis memory and the
/// excitation-energy history, which that path skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhaseDispersion {
    /// The last five LTP gains, Q14, newest first.
    gain_memory: [Word16; PHD_GAIN_MEM],
    /// The previous subframe's dispersion level, 0..=2.
    previous_level: Word16,
    /// The previous subframe's codebook gain, Q1.
    previous_cb_gain: Word16,
    /// Forces maximum dispersion while set.
    locked: bool,
    /// Subframes remaining in the current onset hold.
    onset: Word16,
}

impl PhaseDispersion {
    /// A disperser in its reset state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            gain_memory: [Word16(0); PHD_GAIN_MEM],
            previous_level: Word16(0),
            previous_cb_gain: Word16(0),
            locked: false,
            onset: Word16(0),
        }
    }

    /// Force maximum dispersion, TS 26.073 `ph_disp_lock`.
    ///
    /// The decoder applies this on a bad frame in background noise at the three
    /// lowest rates. It is transient in practice — [`Self::release`] runs at the
    /// top of every subframe.
    pub const fn lock(&mut self) {
        self.locked = true;
    }

    /// Undo [`Self::lock`], TS 26.073 `ph_disp_release`.
    pub const fn release(&mut self) {
        self.locked = false;
    }

    /// Choose this subframe's dispersion level and update the carried state.
    ///
    /// Split out from [`Self::apply`] only for readability: it runs for every
    /// mode, including the three that never disperse anything, because the
    /// history it maintains outlives the mode.
    fn choose_level(&mut self, ctx: &mut DspContext, cb_gain: Word16, ltp_gain: Word16) -> Word16 {
        for i in (1..PHD_GAIN_MEM).rev() {
            self.gain_memory[i] = self.gain_memory[i - 1];
        }
        self.gain_memory[0] = ltp_gain;

        // Strict inequalities both ways: a gain exactly on the low threshold
        // disperses maximally, one exactly on the high threshold not at all.
        let mut level = if sub(ctx, ltp_gain, PHD_THR2_LTP).0 < 0 {
            if sub(ctx, ltp_gain, PHD_THR1_LTP).0 > 0 {
                Word16(1)
            } else {
                Word16(0)
            }
        } else {
            Word16(2)
        };

        // Arithmetically twice the previous codebook gain, but written as this
        // exact chain because the `L_shl` saturates once the previous gain
        // reaches 16384 and the saturation changes the comparison.
        let doubled = l_mult(ctx, self.previous_cb_gain, ON_FACT_PLUS1);
        let positioned = l_shl(ctx, doubled, 2);
        let onset_threshold = round(ctx, positioned);
        if sub(ctx, cb_gain, onset_threshold).0 > 0 {
            self.onset = ON_LENGTH;
        } else if self.onset.0 > 0 {
            self.onset = sub(ctx, self.onset, Word16(1));
        }

        if self.onset.0 == 0 {
            let mut weak = Word16(0);
            for &gain in &self.gain_memory {
                if sub(ctx, gain, PHD_THR1_LTP).0 < 0 {
                    weak = add(ctx, weak, Word16(1));
                }
            }
            // Strictly more than two of five, i.e. a majority — not "two or
            // more".
            if sub(ctx, weak, Word16(2)).0 > 0 {
                level = Word16(0);
            }
        }

        // Dispersion may weaken by at most one step per subframe, so a single
        // strongly-voiced subframe cannot switch it off outright.
        let one_step_up = add(ctx, self.previous_level, Word16(1));
        if sub(ctx, level, one_step_up).0 > 0 && self.onset.0 == 0 {
            level = sub(ctx, level, Word16(1));
        }
        if sub(ctx, level, Word16(2)).0 < 0 && self.onset.0 > 0 {
            level = add(ctx, level, Word16(1));
        }

        if sub(ctx, cb_gain, Word16(10)).0 < 0 {
            level = Word16(2);
        }
        // The lock overrides even the very-low-level cut-out above.
        if self.locked {
            level = Word16(0);
        }

        // Both carried values take the *final* level, after every override.
        self.previous_level = level;
        self.previous_cb_gain = cb_gain;
        level
    }

    /// Disperse the innovation and form the total excitation.
    ///
    /// `excitation` arrives as the unscaled adaptive-codebook vector, Q0, and
    /// leaves as the total excitation, Q0. `innovation` is Q13 (Q12 at
    /// 12.2 kbit/s) and is **destroyed** — the dispersed copy is written back
    /// over it, so a caller that still needs the algebraic codevector (the LTP
    /// history does) must have taken its own copy first.
    ///
    /// Three of the eight rates never disperse: 7.40, 10.2 and 12.2. The source
    /// comment claims only two, and the code is what is normative. The state
    /// update and the total-excitation formation still run for them.
    ///
    /// # Panics
    ///
    /// If either block is not exactly one subframe long, or `mode_index` is not
    /// a speech mode.
    pub fn apply(
        &mut self,
        ctx: &mut DspContext,
        mode_index: u8,
        excitation: &mut [Word16],
        innovation: &mut [Word16],
        gains: ExcitationGains,
    ) {
        assert_eq!(
            excitation.len(),
            L_SUBFR,
            "phase dispersion takes a subframe"
        );
        assert_eq!(
            innovation.len(),
            L_SUBFR,
            "phase dispersion takes a subframe"
        );
        assert!(
            mode_index <= MR122,
            "mode {mode_index} is not a speech mode"
        );

        let level = self.choose_level(ctx, gains.codebook, gains.pitch);

        // Plain comparisons on the mode index: the reference writes these as
        // `sub(mode, MR122) != 0`, but the enum spans 0..8 so the saturating
        // subtraction can only ever agree with a direct comparison.
        let disperse =
            mode_index != MR122 && mode_index != MR102 && mode_index != MR74 && level.0 < 2;
        if disperse {
            disperse_innovation(ctx, mode_index, level, innovation);
        }

        for i in 0..L_SUBFR {
            let mut acc = l_mult(ctx, excitation[i], gains.pitch_factor);
            acc = l_mac(ctx, acc, innovation[i], gains.codebook);
            acc = l_shl(ctx, acc, gains.shift);
            excitation[i] = round(ctx, acc);
        }
    }
}

/// The per-subframe scalars phase dispersion needs, kept together because
/// crossing two of them is the mistake the decoder invites.
///
/// The codebook gain here is the *averaged* one (`Cb_gain_average`'s output),
/// which for five of the eight rates differs from the raw decoded gain that the
/// adaptive-codebook history is updated with. The pitch gain, by contrast, is
/// the limited one — the background-noise limiter runs before this point but
/// after the pitch-sharpened excitation copy is built, so that copy alone sees
/// the unlimited value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExcitationGains {
    /// Codebook gain, Q1.
    pub codebook: Word16,
    /// LTP gain, Q14 — the dispersion level's only input.
    pub pitch: Word16,
    /// Factor scaling the LTP excitation, Q14 (Q13 at 12.2 kbit/s).
    pub pitch_factor: Word16,
    /// Left shift aligning the weighted sum to Q16: 1 for every rate but 12.2,
    /// which uses 2 because both of its inputs carry one bit less.
    pub shift: i16,
}

/// Circular convolution of the innovation with the selected impulse response.
fn disperse_innovation(
    ctx: &mut DspContext,
    mode_index: u8,
    level: Word16,
    innovation: &mut [Word16],
) {
    // Pulse positions, the original amplitudes, and the cleared accumulator
    // are all produced in one pass — the zeroing cannot disturb the scan
    // because each sample is read before it is cleared.
    let mut original = [Word16(0); L_SUBFR];
    let mut positions = [0usize; L_SUBFR];
    let mut pulses = 0usize;
    for (i, slot) in innovation.iter_mut().enumerate() {
        if slot.0 != 0 {
            positions[pulses] = i;
            pulses += 1;
        }
        original[i] = *slot;
        *slot = Word16(0);
    }

    // 7.95 kbit/s has its own pair of responses; every other dispersing rate
    // shares one pair. `level` is 0 or 1 here, 2 having been excluded.
    let response: &[i16; L_SUBFR] = if mode_index == MR795 {
        if level.0 == 0 {
            &PH_IMP_LOW_MR795
        } else {
            &PH_IMP_MID_MR795
        }
    } else if level.0 == 0 {
        &PH_IMP_LOW
    } else {
        &PH_IMP_MID
    };

    for &position in &positions[..pulses] {
        let amplitude = original[position];
        // One tap index sweeps 0..L_SUBFR across both halves: the split is the
        // wrap-around of a circular convolution, not two filters.
        let mut tap = 0usize;
        for slot in &mut innovation[position..] {
            let contribution = mult(ctx, amplitude, Word16(response[tap]));
            tap += 1;
            // Saturating `add`: on a dense codebook the accumulator really
            // does reach the rail, and clamping there is the specified
            // behaviour.
            *slot = add(ctx, *slot, contribution);
        }
        for slot in &mut innovation[..position] {
            let contribution = mult(ctx, amplitude, Word16(response[tap]));
            tap += 1;
            *slot = add(ctx, *slot, contribution);
        }
    }
}

// ------------------------------------------------------ excitation control --

/// Nine-point median, TS 26.073 `gmed_n`.
///
/// A selection sort that repeatedly extracts the maximum and records where it
/// came from, then returns the value at the middle-ranked position. `Bgn_scd`
/// needs the same routine; it lives here because [`control_excitation`] is its
/// only caller in this module's scope.
///
/// Two oddities are reproduced rather than tidied: the running maximum starts
/// at −32767, one short of the minimum, and the comparison is `>=`, so the
/// *last* index holding the maximum wins. Neither changes the value returned
/// for the non-degenerate inputs the decoder produces, but both change which
/// index is chosen among equals, and a future caller might care.
#[must_use]
pub fn median_of_nine(ctx: &mut DspContext, values: &[Word16; EXC_ENERGY_HIST]) -> Word16 {
    let mut remaining = *values;
    let mut rank = [0usize; EXC_ENERGY_HIST];
    let mut chosen = 0usize;

    for slot in &mut rank {
        let mut largest = Word16(-32767);
        for (j, &candidate) in remaining.iter().enumerate() {
            if sub(ctx, candidate, largest).0 >= 0 {
                largest = candidate;
                chosen = j;
            }
        }
        remaining[chosen] = Word16(-32768);
        *slot = chosen;
    }

    // Plain integer halving of the count, as in the reference's `shr(n, 1)`
    // on a compile-time-constant 9.
    values[rank[EXC_ENERGY_HIST / 2]]
}

/// Restrain the excitation level in background noise, TS 26.073 `Ex_ctrl`.
///
/// Scales `excitation` (Q0, one subframe) up toward a target derived from the
/// energy history, but only when this subframe is quieter than the historical
/// median and not near-silent. `energy` is the Q0 square root of the subframe's
/// excitation energy; `history` runs oldest-first with the previous subframe at
/// index 8; `hangover` counts subframes since the last voiced one.
///
/// `energy` is by value and the caller's copy is untouched, which matters: the
/// reference overwrites its local with the *reciprocal* partway through, and it
/// is the original that the decoder pushes into `history` after this returns.
///
/// The decoder gates this to 4.75, 5.15 and 5.90 kbit/s only, and passes the
/// *previous* frame's bad-frame flag as `prev_bfi`, not the current one.
///
/// # Panics
///
/// If `excitation` is not exactly one subframe long.
pub fn control_excitation(
    ctx: &mut DspContext,
    excitation: &mut [Word16],
    energy: Word16,
    history: &[Word16; EXC_ENERGY_HIST],
    hangover: Word16,
    prev_bfi: bool,
    careful: bool,
) {
    assert_eq!(excitation.len(), L_SUBFR, "Ex_ctrl takes a subframe");

    let mut target = median_of_nine(ctx, history);

    let recent_sum = add(ctx, history[7], history[8]);
    let mut previous = shr(ctx, recent_sum, 1);
    if sub(ctx, history[8], previous).0 < 0 {
        previous = history[8];
    }

    // Everything below runs only for 5 < energy < target: a subframe that is
    // already at or above the historical level is left alone, and one that is
    // essentially silent is not amplified.
    if !(sub(ctx, energy, target).0 < 0 && sub(ctx, energy, Word16(5)).0 > 0) {
        return;
    }

    let mut ceiling = shl(ctx, previous, 2);
    if sub(ctx, hangover, Word16(7)).0 < 0 || prev_bfi {
        ceiling = sub(ctx, ceiling, previous);
    }
    if sub(ctx, target, ceiling).0 > 0 {
        target = ceiling;
    }

    let exp = norm_s(energy);
    let normalised = shl(ctx, energy, exp);
    // 16383, not 16384 — and the numerator being one below the normalised
    // minimum is what keeps `div_s` inside its domain.
    let reciprocal = div_s(Word16(16383), normalised);

    let mut acc = l_mult(ctx, target, reciprocal);
    // 20 rather than 30, so the scale factor lands in Q10: 1024 is unity.
    let denormalise = sub(ctx, Word16(20), Word16(exp));
    acc = l_shr(ctx, acc, denormalise.0);
    if l_sub(ctx, acc, Word32(32767)).0 > 0 {
        acc = Word32(32767);
    }
    let mut scale = extract_l(acc);

    if careful && sub(ctx, scale, Word16(3072)).0 > 0 {
        scale = Word16(3072);
    }

    for slot in excitation.iter_mut() {
        let mut product = l_mult(ctx, scale, *slot);
        product = l_shr(ctx, product, 11);
        // `extract_l`, which keeps the low sixteen bits and *wraps*. With a
        // scale factor near 32767 the shifted product genuinely exceeds 16
        // bits and genuinely wraps; saturating here would be a different codec.
        *slot = extract_l(product);
    }
}

#[cfg(test)]
mod tests {
    use super::super::lsp::AZ_SIZE;
    use super::super::vectors::{next_noise, noise, rows, Row};
    use super::*;

    fn ctx() -> DspContext {
        DspContext::default()
    }

    /// Gains for the property tests: a codebook gain well clear of the
    /// very-low-level cut-out, and a zero pitch factor so part C cannot fold
    /// the dispersed innovation back into the excitation being inspected.
    const PROBE_GAINS: ExcitationGains = ExcitationGains {
        codebook: Word16(1000),
        pitch: Word16(0),
        pitch_factor: Word16(0),
        shift: 1,
    };

    /// The first row of a section, which every section here uses for its
    /// header (`az`, `seed` or `seq`).
    fn labelled(section: &str, label: &str) -> Row {
        *rows(section)
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("{section} has no {label} row"))
    }

    /// The four subframes' LP coefficient sets, as the oracle derived them
    /// once and used for `synfilt`, `weightai` and `residu` alike.
    fn oracle_az() -> Vec<Word16> {
        let az = labelled("synfilt", "az").words();
        assert_eq!(az.len(), AZ_SIZE, "the az row is a whole frame of filters");
        az
    }

    #[test]
    fn synthesis_filtering_is_bit_exact_against_ts26073() {
        let az = oracle_az();
        let rows = rows("synfilt");
        let mut c = ctx();
        let mut mem = [Word16(0); M];
        let mut compared = 0;
        let mut subframe = 0usize;
        let mut pending: Option<Vec<Word16>> = None;

        for row in &rows {
            match row.label {
                "exc" => pending = Some(row.words()),
                "out" => {
                    let exc = pending.take().expect("an exc row precedes every out row");
                    let want = row.words();
                    assert_eq!(exc.len(), L_SUBFR);
                    let mut got = vec![Word16(0); L_SUBFR];
                    let a = &az[subframe * MP1..(subframe + 1) * MP1];
                    mem = synthesis_filter(&mut c, a, &exc, &mut got, &mem);
                    for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0, w.0,
                            "synfilt subframe {subframe}: y[{i}] = {} but the reference gives {}",
                            g.0, w.0
                        );
                    }
                    subframe += 1;
                    compared += 1;
                }
                _ => {}
            }
        }

        assert_eq!(
            compared, 4,
            "synfilt replays four subframes, compared {compared}"
        );
    }

    #[test]
    fn the_synthesis_inputs_come_from_the_oracles_own_generator() {
        // Cross-check on the shared pseudo-noise recurrence. If the two
        // generators ever drift, every section that regenerates its inputs
        // rather than reading them would compare against data the reference
        // never saw, and the failures would look like DSP bugs.
        let seed = i16::try_from(labelled("synfilt", "seed").ints()[0]).expect("seed fits");
        let drawn = noise(seed, 4 * L_SUBFR, 5);
        let dumped: Vec<Word16> = rows("synfilt")
            .iter()
            .filter(|r| r.label == "exc")
            .flat_map(Row::words)
            .collect();
        assert_eq!(dumped.len(), 4 * L_SUBFR);
        assert_eq!(
            drawn, dumped,
            "the regenerated excitation is not the oracle's"
        );
    }

    #[test]
    fn agc_is_bit_exact_against_ts26073() {
        let rows = rows("agc");
        let seed = i16::try_from(labelled("agc", "seed").ints()[0]).expect("seed fits");
        // Each replay draws a reference block then a target block from one
        // stream, so the whole run is a single contiguous draw.
        let cases = rows.iter().filter(|r| r.label == "out").count();
        let drawn = noise(seed, cases * 2 * L_SUBFR, 4);

        let mut c = ctx();
        let mut agc = AdaptiveGain::new();
        assert_eq!(
            agc.past_gain().0,
            4096,
            "AGC resets to unity, not to silence"
        );

        let mut compared = 0usize;
        let mut n = 0usize;
        let mut produced: Option<Vec<Word16>> = None;

        for row in &rows {
            match row.label {
                "out" => {
                    let base = n * 2 * L_SUBFR;
                    let reference = &drawn[base..base + L_SUBFR];
                    let mut signal = drawn[base + L_SUBFR..base + 2 * L_SUBFR].to_vec();
                    agc.scale(
                        &mut c,
                        reference,
                        &mut signal,
                        Word16(super::super::AGC_FAC),
                    );
                    let want = row.words();
                    for (i, (&g, &w)) in signal.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0, w.0,
                            "agc case {n}: out[{i}] = {} but the reference gives {}",
                            g.0, w.0
                        );
                    }
                    produced = Some(signal);
                    n += 1;
                    compared += 1;
                }
                "mem" => {
                    assert!(produced.take().is_some(), "a mem row follows every out row");
                    let want = i16::try_from(row.ints()[0]).expect("gain fits");
                    assert_eq!(agc.past_gain().0, want, "agc case {}: carried gain", n - 1);
                }
                _ => {}
            }
        }

        assert_eq!(compared, 6, "agc replays six blocks, compared {compared}");
    }

    #[test]
    fn agc2_is_bit_exact_against_ts26073() {
        let rows = rows("agc2");
        let seed = i16::try_from(labelled("agc2", "seed").ints()[0]).expect("seed fits");
        let cases = rows.iter().filter(|r| r.label == "out").count();
        let drawn = noise(seed, cases * 2 * L_SUBFR, 4);

        let mut c = ctx();
        let mut compared = 0usize;

        for (n, row) in rows.iter().filter(|r| r.label == "out").enumerate() {
            let base = n * 2 * L_SUBFR;
            let reference = &drawn[base..base + L_SUBFR];
            let mut signal = drawn[base + L_SUBFR..base + 2 * L_SUBFR].to_vec();
            match_energy(&mut c, reference, &mut signal);
            let want = row.words();
            for (i, (&g, &w)) in signal.iter().zip(want.iter()).enumerate() {
                assert_eq!(
                    g.0, w.0,
                    "agc2 case {n}: out[{i}] = {} but the reference gives {}",
                    g.0, w.0
                );
            }
            compared += 1;
        }

        assert_eq!(compared, 6, "agc2 replays six blocks, compared {compared}");
    }

    #[test]
    fn bandwidth_expansion_is_bit_exact_against_ts26073() {
        use super::super::decoder_tables::{GAMMA3, GAMMA3_MR122, GAMMA4, GAMMA4_MR122};

        // The oracle sweeps the four post-filter factor sets in this order.
        let sets: [&[i16; M]; 4] = [&GAMMA3_MR122, &GAMMA3, &GAMMA4_MR122, &GAMMA4];
        let az = oracle_az();
        let mut c = ctx();
        let mut compared = 0usize;
        let mut which: Option<usize> = None;

        for row in &rows("weightai") {
            match row.label {
                "case" => {
                    let index = usize::try_from(row.ints()[0]).expect("case index");
                    assert_eq!(index, compared, "weightai cases arrive in order");
                    which = Some(index);
                }
                "out" => {
                    let index = which.take().expect("a case row precedes every out row");
                    let got = expand_bandwidth(&mut c, &az[..MP1], sets[index]);
                    let want = row.words();
                    assert_eq!(want.len(), MP1);
                    for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0, w.0,
                            "weightai case {index}: a[{i}] = {} but the reference gives {}",
                            g.0, w.0
                        );
                    }
                    compared += 1;
                }
                _ => {}
            }
        }

        assert_eq!(
            compared, 4,
            "weightai sweeps four factor sets, compared {compared}"
        );
    }

    #[test]
    fn lp_inverse_filtering_is_bit_exact_against_ts26073() {
        let az = oracle_az();
        let rows = rows("residu");
        let signal = labelled("residu", "in").words();
        let want = rows
            .iter()
            .find(|r| r.label == "out")
            .expect("residu has an out row")
            .words();

        assert_eq!(
            signal.len(),
            L_SUBFR + M,
            "residu supplies M samples of history"
        );
        assert_eq!(want.len(), L_SUBFR);

        let mut c = ctx();
        let mut got = vec![Word16(0); L_SUBFR];
        lp_residual(&mut c, &az[..MP1], &signal, &mut got);

        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w.0,
                "residu: r[{i}] = {} but the reference gives {}",
                g.0, w.0
            );
        }
        assert_eq!(got.len(), L_SUBFR, "compared a whole subframe");
    }

    #[test]
    fn preemphasis_is_bit_exact_against_ts26073() {
        let rows = rows("preemph");
        let mut seed = i16::try_from(labelled("preemph", "seed").ints()[0]).expect("seed fits");

        let mut c = ctx();
        let mut filter = Preemphasis::new();
        let mut compared = 0usize;
        let mut block: Option<(Word16, Vec<Word16>)> = None;

        for row in &rows {
            match row.label {
                "case" => {
                    // The oracle interleaves one scalar draw with forty vector
                    // draws from a single stream, so the coefficient must be
                    // drawn before the block or everything after it shifts.
                    let raw = next_noise(&mut seed);
                    // Plain C arithmetic in the oracle, so plain Rust here:
                    // masking to twelve bits makes the arithmetic and logical
                    // shifts agree, which is why this needs no unsigned cast.
                    let coefficient = Word16((raw >> 3) & 0x0FFF);
                    let want = i16::try_from(row.ints()[0]).expect("coefficient fits");
                    assert_eq!(
                        coefficient.0, want,
                        "preemph case {compared}: regenerated coefficient"
                    );
                    let signal = (0..L_SUBFR)
                        .map(|_| Word16(next_noise(&mut seed) >> 4))
                        .collect();
                    block = Some((coefficient, signal));
                }
                "out" => {
                    let (coefficient, mut signal) =
                        block.take().expect("a case row precedes every out row");
                    filter.filter(&mut c, &mut signal, coefficient);
                    let want = row.words();
                    for (i, (&g, &w)) in signal.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0, w.0,
                            "preemph case {compared}: y[{i}] = {} but the reference gives {}",
                            g.0, w.0
                        );
                    }
                    compared += 1;
                }
                "mem" => {
                    let want = i16::try_from(row.ints()[0]).expect("memory fits");
                    assert_eq!(
                        filter.memory().0,
                        want,
                        "preemph case {}: carried sample",
                        compared - 1
                    );
                }
                _ => {}
            }
        }

        assert_eq!(
            compared, 4,
            "preemph replays four blocks, compared {compared}"
        );
    }

    #[test]
    fn phase_dispersion_is_bit_exact_against_ts26073() {
        let mut c = ctx();
        let mut state = PhaseDispersion::new();
        let mut mode_index = 0u8;
        let mut sequences = 0usize;
        let mut compared = 0usize;
        let mut step: Option<(Word16, Word16, i16)> = None;
        let mut excitation: Option<Vec<Word16>> = None;
        let mut innovation: Option<Vec<Word16>> = None;
        // Which dispersion levels the sweep actually reached, and whether any
        // innovation was genuinely rewritten. Forty matching vectors would
        // prove very little if every one of them had taken the no-dispersion
        // branch, so the coverage is asserted rather than hoped for.
        let mut levels_seen = [false; 3];
        let mut dispersed = 0usize;

        for row in &rows("phdisp") {
            match row.label {
                "seq" => {
                    let head = row.ints();
                    mode_index = u8::try_from(head[0]).expect("mode index");
                    // The oracle resets the state at the top of every mode.
                    state = PhaseDispersion::new();
                    sequences += 1;
                }
                "step" => {
                    let v = row.ints();
                    step = Some((
                        Word16(i16::try_from(v[0]).expect("cbGain fits")),
                        Word16(i16::try_from(v[1]).expect("ltpGain fits")),
                        i16::try_from(v[2]).expect("shift fits"),
                    ));
                }
                "x" => excitation = Some(row.words()),
                "inno" => innovation = Some(row.words()),
                "out" => {
                    let (cb_gain, ltp_gain, shift) = step.take().expect("a step row precedes out");
                    let mut x = excitation.take().expect("an x row precedes out");
                    let mut inno = innovation.take().expect("an inno row precedes out");
                    let inno_before = inno.clone();
                    // The oracle releases the lock before every call, and
                    // passes the LTP gain again as the pitch factor.
                    state.release();
                    state.apply(
                        &mut c,
                        mode_index,
                        &mut x,
                        &mut inno,
                        ExcitationGains {
                            codebook: cb_gain,
                            pitch: ltp_gain,
                            pitch_factor: ltp_gain,
                            shift,
                        },
                    );
                    let want = row.words();
                    for (i, (&g, &w)) in x.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0,
                            w.0,
                            "phdisp mode {mode_index} case {}: x[{i}] = {} but the reference \
                             gives {}",
                            compared % 5,
                            g.0,
                            w.0
                        );
                    }
                    let level = usize::try_from(state.previous_level.0).expect("level is 0..=2");
                    levels_seen[level] = true;
                    if inno != inno_before {
                        dispersed += 1;
                    }
                    compared += 1;
                }
                _ => {}
            }
        }

        assert_eq!(sequences, 8, "phdisp sweeps all eight rates");
        assert_eq!(
            compared, 40,
            "phdisp replays five subframes per rate, compared {compared}"
        );
        assert_eq!(
            levels_seen, [true; 3],
            "the sweep did not reach all three dispersion levels"
        );
        assert!(
            dispersed >= 8,
            "only {dispersed} of {compared} cases actually dispersed anything"
        );
    }

    #[test]
    fn excitation_control_is_bit_exact_against_ts26073() {
        let mut c = ctx();
        let mut compared = 0usize;
        let mut scaled = 0usize;
        let mut excitation: Option<Vec<Word16>> = None;
        let mut history: Option<Vec<Word16>> = None;
        let mut step: Option<(Word16, Word16, bool, bool)> = None;

        for row in &rows("exctrl") {
            match row.label {
                "exc" => excitation = Some(row.words()),
                "hist" => history = Some(row.words()),
                "step" => {
                    let v = row.ints();
                    step = Some((
                        Word16(i16::try_from(v[0]).expect("energy fits")),
                        Word16(i16::try_from(v[1]).expect("hangover fits")),
                        v[2] != 0,
                        v[3] != 0,
                    ));
                }
                "out" => {
                    let mut exc = excitation.take().expect("an exc row precedes out");
                    let hist_row = history.take().expect("a hist row precedes out");
                    let (energy, hangover, prev_bfi, careful) =
                        step.take().expect("a step row precedes out");
                    let hist: [Word16; EXC_ENERGY_HIST] =
                        hist_row.try_into().expect("nine history entries");
                    let before = exc.clone();

                    control_excitation(
                        &mut c, &mut exc, energy, &hist, hangover, prev_bfi, careful,
                    );

                    let want = row.words();
                    for (i, (&g, &w)) in exc.iter().zip(want.iter()).enumerate() {
                        assert_eq!(
                            g.0, w.0,
                            "exctrl case {compared}: exc[{i}] = {} but the reference gives {}",
                            g.0, w.0
                        );
                    }
                    if before != exc {
                        scaled += 1;
                    }
                    compared += 1;
                }
                _ => {}
            }
        }

        assert_eq!(
            compared, 8,
            "exctrl replays eight subframes, compared {compared}"
        );
        // Both branches must appear: the gate is what most of these cases
        // exercise, and matching eight untouched vectors would say nothing
        // about the scaling itself.
        assert!(
            scaled >= 1,
            "no exctrl case actually rescaled the excitation"
        );
        assert!(
            scaled < compared,
            "no exctrl case exercised the pass-through gate"
        );
    }

    // --------------------------------------------------------- properties --
    //
    // These cannot replace the comparisons above: the oracle and this module
    // could share a wrong assumption and still agree. What they catch is the
    // class of mistake a fixture that was generated from the same misreading
    // would not — aliasing, conservation, permutation and table identity.

    #[test]
    fn the_two_synthesis_entry_points_agree() {
        // The in-place form exists only because Rust cannot alias `x` and `y`;
        // if it ever diverged, four reference call sites would be wrong.
        let az = oracle_az();
        let exc = noise(4321, L_SUBFR, 5);
        let mem = [Word16(7); M];

        let mut c = ctx();
        let mut separate = vec![Word16(0); L_SUBFR];
        let mem_a = synthesis_filter(&mut c, &az[..MP1], &exc, &mut separate, &mem);

        let mut c2 = ctx();
        let mut aliased = exc.clone();
        let mem_b = synthesis_filter_in_place(&mut c2, &az[..MP1], &mut aliased, &mem);

        assert_eq!(
            separate, aliased,
            "in-place filtering diverged from the copying form"
        );
        assert_eq!(mem_a, mem_b);
        assert_eq!(mem_a.to_vec(), separate[L_SUBFR - M..].to_vec());
    }

    #[test]
    fn the_inverse_filter_undoes_the_synthesis_filter() {
        // A(z) and 1/A(z) are exact inverses in real arithmetic; in fixed
        // point the round-trip error is the rounding of two `round` calls per
        // sample. A sign flip or an off-by-one in either recursion blows this
        // up immediately, where a bit-exact fixture alone would not say which
        // of the two is wrong.
        let az = oracle_az();
        let a = &az[..MP1];
        let speech = noise(1379, L_SUBFR + M, 6);

        let mut c = ctx();
        let mut residual = vec![Word16(0); L_SUBFR];
        lp_residual(&mut c, a, &speech, &mut residual);

        // The synthesis memory is the same history the residual filter read.
        let mut mem = [Word16(0); M];
        mem.copy_from_slice(&speech[..M]);
        let mut recovered = vec![Word16(0); L_SUBFR];
        synthesis_filter(&mut c, a, &residual, &mut recovered, &mem);

        for (i, (&r, &s)) in recovered.iter().zip(speech[M..].iter()).enumerate() {
            let error = i32::from(r.0) - i32::from(s.0);
            assert!(
                error.abs() <= 4,
                "round trip sample {i}: {} against {}, error {error}",
                r.0,
                s.0
            );
        }
    }

    #[test]
    fn expansion_preserves_the_leading_coefficient_and_shrinks_the_rest() {
        use super::super::decoder_tables::GAMMA3;

        let az = oracle_az();
        let mut c = ctx();
        let expanded = expand_bandwidth(&mut c, &az[..MP1], &GAMMA3);

        assert_eq!(expanded[0].0, 4096, "Weight_Ai multiplied a[0]");
        for i in 1..=M {
            assert!(
                i32::from(expanded[i].0).abs() <= i32::from(az[i].0).abs(),
                "coefficient {i} grew under bandwidth expansion"
            );
        }
    }

    #[test]
    fn the_two_identical_factor_sets_really_are_identical() {
        use super::super::decoder_tables::{GAMMA3_MR122, GAMMA4};

        // `GAMMA4` and `GAMMA3_MR122` are both 0.7^n and coincide entry for
        // entry. The fixture's cases 0 and 3 therefore have equal outputs,
        // which would also be true if the test indexed the wrong table — so
        // assert the coincidence directly rather than reading it off.
        let az = oracle_az();
        let mut c = ctx();
        let a = expand_bandwidth(&mut c, &az[..MP1], &GAMMA3_MR122);
        let b = expand_bandwidth(&mut c, &az[..MP1], &GAMMA4);
        assert_eq!(a, b);
        assert_ne!(GAMMA3_MR122, super::super::decoder_tables::GAMMA4_MR122);
    }

    #[test]
    fn preemphasis_with_a_zero_coefficient_is_the_identity() {
        let mut c = ctx();
        let mut filter = Preemphasis::new();
        let original = noise(999, L_SUBFR, 4);
        let mut signal = original.clone();
        filter.filter(&mut c, &mut signal, Word16(0));
        assert_eq!(
            signal, original,
            "a zero coefficient must not alter the block"
        );
        assert_eq!(
            filter.memory(),
            original[L_SUBFR - 1],
            "the carried sample is the last input, not the last output"
        );
    }

    #[test]
    fn preemphasis_carries_the_input_sample_not_the_filtered_one() {
        // With a non-zero coefficient the two differ, which is exactly the
        // mistake this guards: storing signal[L-1] after filtering.
        let mut c = ctx();
        let mut filter = Preemphasis::new();
        let original = noise(2024, L_SUBFR, 4);
        let mut signal = original.clone();
        filter.filter(&mut c, &mut signal, Word16(16384));
        assert_ne!(
            signal[L_SUBFR - 1],
            original[L_SUBFR - 1],
            "the block was not filtered"
        );
        assert_eq!(filter.memory(), original[L_SUBFR - 1]);
    }

    #[test]
    fn agc2_brings_the_two_energies_together() {
        // The conservation law the function exists to enforce. A fixture can
        // only say the numbers match; this says they mean what they should.
        let mut c = ctx();
        let reference = noise(5150, L_SUBFR, 3);
        let mut signal: Vec<Word16> = noise(5900, L_SUBFR, 6)
            .iter()
            .map(|w| Word16(w.0))
            .collect();

        let energy = |v: &[Word16]| -> f64 { v.iter().map(|w| f64::from(w.0).powi(2)).sum() };
        let target = energy(&reference);
        let before = energy(&signal);
        assert!(before > 0.0 && target > 0.0);

        match_energy(&mut c, &reference, &mut signal);
        let after = energy(&signal);

        assert!(
            (after / target).log2().abs() < (before / target).log2().abs(),
            "agc2 moved the energy away from its target: {before} -> {after}, target {target}"
        );
        assert!(
            (after / target).log2().abs() < 0.25,
            "agc2 left the energies {} dB apart",
            10.0 * (after / target).log10()
        );
    }

    #[test]
    fn phase_dispersion_of_a_single_pulse_rotates_the_impulse_response() {
        // The dispersion is a circular convolution, so a unit pulse at
        // position p must reproduce the response rotated by p. This catches a
        // tap index that fails to wrap — which a fixture generated from the
        // same misreading would not.
        //
        // The lock pins the level at maximum, which is also the only way to
        // reach it on a first subframe: the onset detector fires whenever the
        // codebook gain exceeds twice its predecessor, and the predecessor
        // starts at zero.
        for (mode_index, response) in [(0u8, &PH_IMP_LOW), (MR795, &PH_IMP_LOW_MR795)] {
            for position in [0usize, 1, 17, 39] {
                let mut c = ctx();
                let mut state = PhaseDispersion::new();
                state.lock();
                let mut innovation = vec![Word16(0); L_SUBFR];
                innovation[position] = Word16(16384);
                // A zero pitch factor and gain keep part C from folding the
                // dispersed innovation back into the excitation being read.
                let mut excitation = vec![Word16(0); L_SUBFR];

                state.apply(
                    &mut c,
                    mode_index,
                    &mut excitation,
                    &mut innovation,
                    PROBE_GAINS,
                );

                for (i, got) in innovation.iter().enumerate() {
                    let tap = (i + L_SUBFR - position) % L_SUBFR;
                    let want = mult(&mut c, Word16(16384), Word16(response[tap]));
                    assert_eq!(
                        got.0, want.0,
                        "mode {mode_index}, pulse at {position}: innovation[{i}] is not tap {tap}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_onset_holds_dispersion_one_step_below_maximum() {
        // The onset detector fires when the codebook gain more than doubles,
        // and for the next two subframes it forbids full dispersion however
        // weak the LTP gain is. A port that skips part A on the non-dispersing
        // rates, or that decrements the counter in the wrong branch, loses
        // this and over-disperses every attack.
        let mut c = ctx();
        let mut state = PhaseDispersion::new();
        let mut levels = Vec::new();

        for _ in 0..4 {
            let mut excitation = vec![Word16(0); L_SUBFR];
            let mut innovation = vec![Word16(0); L_SUBFR];
            state.apply(&mut c, 0, &mut excitation, &mut innovation, PROBE_GAINS);
            levels.push(state.previous_level.0);
        }

        // Subframe 0 fires the onset (gain 1000 against a remembered 0) and
        // subframes 1 and 2 run the hold down; only by 3 is the LTP-gain
        // history allowed to force maximum dispersion.
        assert_eq!(
            levels,
            vec![1, 1, 0, 0],
            "onset hold did not run for two subframes"
        );
    }

    #[test]
    fn the_rates_that_never_disperse_leave_the_innovation_alone() {
        for mode_index in [MR74, MR102, MR122] {
            let mut c = ctx();
            let mut state = PhaseDispersion::new();
            let original = noise(777, L_SUBFR, 4);
            let mut innovation = original.clone();
            let mut excitation = vec![Word16(0); L_SUBFR];
            state.apply(
                &mut c,
                mode_index,
                &mut excitation,
                &mut innovation,
                PROBE_GAINS,
            );
            assert_eq!(
                innovation, original,
                "mode {mode_index} dispersed an innovation it must not touch"
            );
        }
    }

    #[test]
    fn the_gain_history_is_a_shift_register_in_every_mode() {
        // Part A of the algorithm runs even for the three rates that never
        // disperse; if it did not, their state would be stale the moment the
        // decoder switched rate mid-call.
        for mode_index in 0..=MR122 {
            let mut c = ctx();
            let mut state = PhaseDispersion::new();
            let gains = [
                Word16(100),
                Word16(200),
                Word16(300),
                Word16(400),
                Word16(500),
            ];
            for &g in &gains {
                let mut excitation = vec![Word16(0); L_SUBFR];
                let mut innovation = vec![Word16(0); L_SUBFR];
                state.apply(
                    &mut c,
                    mode_index,
                    &mut excitation,
                    &mut innovation,
                    ExcitationGains {
                        pitch: g,
                        ..PROBE_GAINS
                    },
                );
            }
            let mut want: Vec<Word16> = gains.to_vec();
            want.reverse();
            assert_eq!(
                state.gain_memory.to_vec(),
                want,
                "mode {mode_index} did not maintain the LTP gain history"
            );
        }
    }

    #[test]
    fn locking_overrides_the_low_level_cutout() {
        // cbGain below 10 normally disables dispersion outright, but the lock
        // is applied afterwards and wins. Getting the order wrong makes error
        // concealment silently stop dispersing.
        let mut c = ctx();
        let mut state = PhaseDispersion::new();
        state.lock();
        let mut innovation = vec![Word16(0); L_SUBFR];
        innovation[3] = Word16(8192);
        let mut excitation = vec![Word16(0); L_SUBFR];
        state.apply(
            &mut c,
            0,
            &mut excitation,
            &mut innovation,
            ExcitationGains {
                codebook: Word16(5),
                pitch: Word16(16000),
                ..PROBE_GAINS
            },
        );
        assert_eq!(
            state.previous_level.0, 0,
            "the lock did not force full dispersion"
        );
        assert_ne!(
            innovation[0].0, 0,
            "a locked disperser left the innovation alone"
        );
    }

    #[test]
    fn the_median_is_a_genuine_median() {
        // A permutation check: the value returned must be the fifth smallest
        // of the nine, whatever the tie-breaking does to the index.
        let mut c = ctx();
        let mut seed = 4242i16;
        for _ in 0..64 {
            let mut values = [Word16(0); EXC_ENERGY_HIST];
            for slot in &mut values {
                *slot = Word16(next_noise(&mut seed) >> 4);
            }
            let got = median_of_nine(&mut c, &values);
            let mut sorted: Vec<i16> = values.iter().map(|w| w.0).collect();
            sorted.sort_unstable();
            assert_eq!(got.0, sorted[4], "median of {sorted:?}");
        }
    }

    #[test]
    fn excitation_control_is_a_no_op_outside_its_window() {
        // Only 5 < energy < median does anything; a subframe already at or
        // above the historical level must pass through untouched.
        let mut c = ctx();
        let history = [Word16(100); EXC_ENERGY_HIST];
        let original = noise(313, L_SUBFR, 5);

        for energy in [Word16(0), Word16(5), Word16(100), Word16(1000)] {
            let mut exc = original.clone();
            control_excitation(&mut c, &mut exc, energy, &history, Word16(4), false, false);
            assert_eq!(
                exc, original,
                "energy {} is outside the scaling window but the excitation moved",
                energy.0
            );
        }
    }

    #[test]
    fn the_careful_flag_caps_the_scale_factor_at_three() {
        // Without the cap a near-silent subframe in a loud history gets
        // amplified without limit; the flag is what error concealment relies on.
        let history = [Word16(1000); EXC_ENERGY_HIST];
        let original = noise(818, L_SUBFR, 8);

        let mut careful = original.clone();
        let mut c = ctx();
        control_excitation(
            &mut c,
            &mut careful,
            Word16(6),
            &history,
            Word16(9),
            false,
            true,
        );

        let mut free = original.clone();
        control_excitation(
            &mut c,
            &mut free,
            Word16(6),
            &history,
            Word16(9),
            false,
            false,
        );

        let peak = |v: &[Word16]| v.iter().map(|w| i32::from(w.0).abs()).max().unwrap_or(0);
        assert!(
            peak(&careful) < peak(&free),
            "the careful flag did not restrain the gain"
        );
        assert!(
            peak(&careful) <= 3 * peak(&original) + 1,
            "the careful cap let the excitation past three times its input"
        );
    }
}
