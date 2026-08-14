//! Excitation enhancement, 3GPP TS 26.190 §6.1.
//!
//! Three stages between gain decoding and synthesis. TS 26.190 presents them as
//! refinements, but the reference runs all three unconditionally on the main
//! data path — and they are why **the excitation fed to the synthesis filter is
//! not the one written back to the adaptive-codebook history.**
//!
//! That split matters. The history must hold the excitation the *encoder*
//! believes it produced, or the pitch predictor at both ends drifts apart.
//! These enhancements are for the listener only, so they are applied to a copy.
//! Using one buffer for both gives audio that is recognisably speech and
//! steadily wrong.
//!
//! # What each stage is for
//!
//! - [`voice_factor`] measures how voiced the subframe is, and drives the
//!   other two.
//! - [`PhaseDispersion`] spreads each pulse's energy over time at low rates. At
//!   6.60 kbit/s there are only two pulses in 64 samples, which the ear hears
//!   as clicks rather than speech; smearing them trades peakiness for a more
//!   noise-like excitation.
//! - [`pitch_enhance`] high-pass filters the innovation on voiced frames,
//!   because the adaptive contribution already supplies the low-frequency
//!   periodic energy and doubling it up sounds boomy.

use super::codebook::L_SUBFR;
use super::gain_tables::{PH_IMP_LOW, PH_IMP_MID};
use super::math::dot_product12;
use crate::fixed_point::arith::{add, extract_h, mult, mult_r, negate, round, sub};
use crate::fixed_point::arith32::{l_add, l_deposit_h, l_mac, l_msu, l_mult};
use crate::fixed_point::div::div_s;
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Pitch gain of 0.6 in Q14 — the boundary below which a subframe counts as
/// unvoiced for dispersion purposes.
const PITCH_0_6: i16 = 9830;

/// Pitch gain of 0.9 in Q14.
const PITCH_0_9: i16 = 14746;

/// How much dispersion a rate calls for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispersionLevel {
    /// Rates at or below 6.60 kbit/s, where pulses are sparsest.
    High,
    /// Rates at or below 8.85 kbit/s.
    Low,
    /// Above 8.85 kbit/s there are enough pulses already.
    Off,
}

impl DispersionLevel {
    /// The level a frame width calls for.
    #[must_use]
    pub const fn for_frame_bits(frame_bits: usize) -> Self {
        if frame_bits <= 132 {
            Self::High
        } else if frame_bits <= 177 {
            Self::Low
        } else {
            Self::Off
        }
    }

    /// The reference's numeric level, which is added to the voicing state.
    const fn offset(self) -> i16 {
        match self {
            Self::High => 0,
            Self::Low => 1,
            Self::Off => 2,
        }
    }
}

/// How voiced a subframe is: 1.0 voiced, -1.0 unvoiced, Q15.
///
/// The ratio of adaptive to algebraic energy, which is exactly the question
/// "is this periodic or noise-like?".
#[must_use]
pub fn voice_factor(
    ctx: &mut DspContext,
    excitation: &[Word16],
    q_exc: i16,
    gain_pitch: Word16,
    code: &[Word16],
    gain_code: Word16,
) -> Word16 {
    let (energy, exp1) = dot_product12(ctx, excitation, excitation);
    let mut ener1 = extract_h(energy);
    let mut exp1 = exp1 - 2 * q_exc;

    let squared = l_mult(ctx, gain_pitch, gain_pitch);
    let exp = norm_l(squared);
    let tmp = extract_h(l_shl(ctx, squared, exp));
    ener1 = mult(ctx, ener1, tmp);
    // 10 converts the pitch gain from Q14 to Q9, matching the code's scale.
    exp1 = exp1 - exp - 10;

    let (code_energy, exp2) = dot_product12(ctx, code, code);
    let mut ener2 = extract_h(code_energy);

    let exp = norm_s(gain_code);
    let tmp = shl(ctx, gain_code, exp);
    let tmp = mult(ctx, tmp, tmp);
    ener2 = mult(ctx, ener2, tmp);
    let exp2 = exp2 - 2 * exp;

    // Bring both energies to a common exponent before comparing.
    let diff = exp1 - exp2;
    if diff >= 0 {
        ener1 = shr(ctx, ener1, 1);
        ener2 = shr(ctx, ener2, diff + 1);
    } else {
        ener1 = shr(ctx, ener1, 1 - diff);
        ener2 = shr(ctx, ener2, 1);
    }

    let difference = sub(ctx, ener1, ener2);
    let summed = add(ctx, ener1, ener2);
    let total = add(ctx, summed, Word16(1));

    if difference.0 >= 0 {
        div_s(difference, total)
    } else {
        let magnitude = negate(ctx, difference);
        negate(ctx, div_s(magnitude, total))
    }
}

/// Phase dispersion, with the voicing state it carries between subframes.
///
/// Every field is previous-subframe history, which is the point: the
/// dispersion level depends on where the signal has been, not only where it is.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Default)]
pub struct PhaseDispersion {
    prev_state: i16,
    prev_gain_code: Word16,
    /// Six previous pitch gains, most recent first.
    prev_gain_pitch: [Word16; 6],
}

impl PhaseDispersion {
    /// Fresh state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            prev_state: 0,
            prev_gain_code: Word16(0),
            prev_gain_pitch: [Word16(0); 6],
        }
    }

    /// Disperse one subframe's innovation in place.
    ///
    /// The dispersion strength is chosen from the pitch gain *and* recent
    /// history: a sudden jump in code gain is treated as an onset and gets less
    /// dispersion, because smearing a transient blurs the attack.
    pub fn apply(
        &mut self,
        ctx: &mut DspContext,
        code: &mut [Word16; L_SUBFR],
        gain_code: Word16,
        gain_pitch: Word16,
        level: DispersionLevel,
    ) {
        let mut state = if gain_pitch.0 < PITCH_0_6 {
            0i16
        } else if gain_pitch.0 < PITCH_0_9 {
            1
        } else {
            2
        };

        self.prev_gain_pitch.rotate_right(1);
        self.prev_gain_pitch[0] = gain_pitch;

        let jump = sub(ctx, gain_code, self.prev_gain_code);
        let threshold = shl(ctx, self.prev_gain_code, 1);
        if jump.0 > threshold.0 {
            // An onset: keep the attack sharp.
            if state < 2 {
                state += 1;
            }
        } else {
            // Mostly unvoiced recently, so disperse hard regardless of this
            // subframe's gain.
            let unvoiced = self
                .prev_gain_pitch
                .iter()
                .filter(|g| g.0 < PITCH_0_6)
                .count();
            if unvoiced > 2 {
                state = 0;
            }
            // Rise at most one level per subframe, so the dispersion does not
            // flap between settings and modulate the noise floor.
            if state - self.prev_state > 1 {
                state -= 1;
            }
        }

        self.prev_gain_code = gain_code;
        self.prev_state = state;

        let state = state + level.offset();
        let impulse: &[i16; L_SUBFR] = match state {
            0 => &PH_IMP_LOW,
            1 => &PH_IMP_MID,
            // At level 2 and above the innovation is left alone.
            _ => return,
        };

        // Circular convolution: convolve into a double-length buffer, then fold
        // the tail back onto the head.
        let mut spread = [Word16(0); 2 * L_SUBFR];
        for i in 0..L_SUBFR {
            if code[i].0 == 0 {
                continue;
            }
            for j in 0..L_SUBFR {
                let contribution = mult_r(ctx, code[i], Word16(impulse[j]));
                spread[i + j] = add(ctx, spread[i + j], contribution);
            }
        }
        for i in 0..L_SUBFR {
            code[i] = add(ctx, spread[i], spread[i + L_SUBFR]);
        }
    }
}

/// High-pass the innovation with an explicit strength.
///
/// `strength` is `voice_factor/8 + 0.25` in Q12, which is what the reference
/// computes from the voicing measure.
#[must_use]
pub fn pitch_enhance_with(
    ctx: &mut DspContext,
    code: &[Word16; L_SUBFR],
    strength: Word16,
) -> [Word16; L_SUBFR] {
    let mut out = [Word16(0); L_SUBFR];

    // The ends have only one neighbour, so they get a one-sided filter.
    let acc = l_msu(ctx, l_deposit_h(code[0]), code[1], strength);
    out[0] = round(ctx, acc);

    for i in 1..L_SUBFR - 1 {
        let mut acc = l_msu(ctx, l_deposit_h(code[i]), code[i + 1], strength);
        acc = l_msu(ctx, acc, code[i - 1], strength);
        out[i] = round(ctx, acc);
    }

    let acc = l_msu(
        ctx,
        l_deposit_h(code[L_SUBFR - 1]),
        code[L_SUBFR - 2],
        strength,
    );
    out[L_SUBFR - 1] = round(ctx, acc);

    out
}

/// The pitch-enhancer strength a voicing measure calls for, Q12.
#[must_use]
pub fn enhance_strength(ctx: &mut DspContext, voice_fac: Word16) -> Word16 {
    let scaled = shr(ctx, voice_fac, 3);
    add(ctx, scaled, Word16(4096))
}

/// Scale `output` so its energy matches `input`'s.
///
/// Used at the low rates, where an extra pitch-sharpened excitation is blended
/// in: the blend changes the waveform's shape deliberately but must not change
/// its loudness, or the sharpening would be heard as a level jump rather than
/// as added periodicity.
///
/// Both signals are pre-shifted down two bits before the energy sums, which is
/// what keeps a loud subframe from overflowing the accumulator.
pub fn agc2(ctx: &mut DspContext, input: &[Word16], output: &mut [Word16]) {
    let energy = |ctx: &mut DspContext, v: &[Word16]| -> Word32 {
        let first = shr(ctx, v[0], 2);
        let mut s = l_mult(ctx, first, first);
        for &x in &v[1..] {
            let t = shr(ctx, x, 2);
            s = l_mac(ctx, s, t, t);
        }
        s
    };

    let out_energy = energy(ctx, output);
    if out_energy.0 == 0 {
        return;
    }
    let mut exp = norm_l(out_energy) - 1;
    let normalised = l_shl(ctx, out_energy, exp);
    let gain_out = round(ctx, normalised);

    let in_energy = energy(ctx, input);
    let g0 = if in_energy.0 == 0 {
        Word16(0)
    } else {
        let i = norm_l(in_energy);
        let normalised = l_shl(ctx, in_energy, i);
        let gain_in = round(ctx, normalised);
        exp -= i;

        // g0 = sqrt(gain_in / gain_out), via the reciprocal square root of the
        // ratio the other way up.
        let ratio = Word32(i32::from(div_s(gain_out, gain_in).0));
        let ratio = l_shl(ctx, ratio, 7);
        let ratio = crate::fixed_point::shift::l_shr(ctx, ratio, exp);
        let root = super::math::isqrt(ctx, ratio);
        let lifted = l_shl(ctx, root, 9);
        round(ctx, lifted)
    };

    for sample in output.iter_mut() {
        let scaled = l_mult(ctx, *sample, g0);
        let lifted = l_shl(ctx, scaled, 2);
        *sample = extract_h(lifted);
    }
}

/// How stable the LP filter is between frames: 1.0 stable, 0 unstable, Q15.
///
/// Measured as the squared distance between this frame's ISFs and the last
/// frame's. A filter that moves a long way between frames is not describing a
/// steady spectrum, and the noise enhancer only acts when it is safe to assume
/// the signal is stationary noise rather than a transition.
#[must_use]
pub fn stability_factor(ctx: &mut DspContext, isf: &[Word16], isf_old: &[Word16]) -> Word16 {
    let mut acc = Word32(0);
    // The last ISF is excluded: it is the trailing predictor coefficient
    // rather than a line frequency, so its motion says nothing about stability.
    for i in 0..isf.len() - 1 {
        let d = sub(ctx, isf[i], isf_old[i]);
        acc = l_mac(ctx, acc, d, d);
    }
    let scaled = extract_h(l_shl(ctx, acc, 8));
    let scaled = mult(ctx, scaled, Word16(26214));
    // 1.25 - 0.8·distance, then Q14 to Q15 with saturation, floored at zero.
    let tmp = sub(ctx, Word16(20480), scaled);
    let stab = shl(ctx, tmp, 1);
    if stab.0 < 0 {
        Word16(0)
    } else {
        stab
    }
}

/// The noise enhancer's carried threshold.
///
/// Tracks the code gain slowly, so a gain that is merely fluctuating around a
/// steady level gets pulled back toward it.
#[derive(Debug, Clone, Default)]
pub struct NoiseEnhancer {
    threshold: Word32,
}

impl NoiseEnhancer {
    /// Fresh state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            threshold: Word32(0),
        }
    }

    /// Move the code gain toward the running threshold, Q16 in and out.
    ///
    /// Only acts when the frame is both **noisy** and **stable** — the blend
    /// weight is the product of the two measures. On speech, or during a
    /// transition, it does nothing, because smoothing gain there would blur
    /// attacks. On stationary noise it halves the gain variation, which is
    /// what stops a constant background sounding like it is breathing.
    pub fn apply(
        &mut self,
        ctx: &mut DspContext,
        gain_code: Word32,
        voice_fac: Word16,
        stab_fac: Word16,
    ) -> Word32 {
        // 1 when unvoiced, 0 when voiced.
        let halved = shr(ctx, voice_fac, 1);
        let noisiness = sub(ctx, Word16(16384), halved);
        let fac = mult(ctx, stab_fac, noisiness);

        let (hi, lo) = l_extract(gain_code);

        // Step the threshold toward the gain by about 1.5 dB, never past it.
        // The two constants are +1.19 and -0.84 in Q15/Q14 terms.
        let stepped = if gain_code.0 < self.threshold.0 {
            let raised = l_add(ctx, gain_code, mpy_32_16(hi, lo, Word16(6226)));
            if raised.0 > self.threshold.0 {
                self.threshold
            } else {
                raised
            }
        } else {
            let lowered = mpy_32_16(hi, lo, Word16(27536));
            if lowered.0 < self.threshold.0 {
                self.threshold
            } else {
                lowered
            }
        };
        self.threshold = stepped;

        // Blend the original gain with the threshold, weighted by fac.
        let kept = mpy_32_16(hi, lo, sub(ctx, Word16(32767), fac));
        let (t_hi, t_lo) = l_extract(stepped);
        l_add(ctx, kept, mpy_32_16(t_hi, t_lo, fac))
    }
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_row, block_row_i32, has_block};
    use super::*;

    fn expect(label: &str, got: &[Word16], blk: usize) {
        let want = block_row("enhance", &format!("{label}{blk}"));
        assert_eq!(want.len(), got.len(), "{label}{blk}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w,
                "{label}{blk}: sample {i} = {} but the reference gives {w}",
                g.0
            );
        }
    }

    fn excitation(blk: usize) -> [Word16; L_SUBFR] {
        let mut v = [Word16(0); L_SUBFR];
        for (n, slot) in v.iter_mut().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let t = (blk * L_SUBFR + n) as f64;
            #[allow(clippy::cast_possible_truncation)]
            {
                *slot = Word16((2500.0 * (2.0 * std::f64::consts::PI * t / 31.0).sin()) as i16);
            }
        }
        v
    }

    fn innovation(blk: usize) -> [Word16; L_SUBFR] {
        let mut code = [Word16(0); L_SUBFR];
        for (n, slot) in code.iter_mut().enumerate() {
            if n % 9 == blk % 9 {
                *slot = Word16(if n % 2 == 1 { -512 } else { 512 });
            }
        }
        code
    }

    const LEVELS: [DispersionLevel; 3] = [
        DispersionLevel::High,
        DispersionLevel::Low,
        DispersionLevel::Off,
    ];

    #[test]
    fn the_enhancement_chain_is_bit_exact_against_ts26173() {
        assert!(has_block("enhance"), "fixture block enhance missing");
        let mut ctx = DspContext::default();
        let mut disp = PhaseDispersion::new();

        for blk in 0..5 {
            let meta = block_row("enhance", &format!("emeta{blk}"));
            let (gain_pitch, gain_code) = (Word16(meta[0]), Word16(meta[1]));

            let exc = excitation(blk);
            let mut code = innovation(blk);
            expect("eexc", &exc, blk);
            expect("ecode", &code, blk);

            let voice_fac = voice_factor(&mut ctx, &exc, -3, gain_pitch, &code, gain_code);
            let want_vfac = block_row("enhance", &format!("vfac{blk}"));
            assert_eq!(
                i32::from(voice_fac.0),
                i32::from(want_vfac[0]),
                "block {blk}: voice factor"
            );

            disp.apply(&mut ctx, &mut code, gain_code, gain_pitch, LEVELS[blk % 3]);
            expect("disp", &code, blk);

            // The carried state matters as much as the output: a wrong update
            // only shows up subframes later.
            let want_mem = block_row("enhance", &format!("dmem{blk}"));
            assert_eq!(
                i32::from(disp.prev_state),
                i32::from(want_mem[0]),
                "block {blk}: dispersion state"
            );
            assert_eq!(
                i32::from(disp.prev_gain_code.0),
                i32::from(want_mem[1]),
                "block {blk}: previous code gain"
            );
            for (i, &w) in want_mem[2..8].iter().enumerate() {
                assert_eq!(
                    i32::from(disp.prev_gain_pitch[i].0),
                    i32::from(w),
                    "block {blk}: pitch gain history {i}"
                );
            }

            let strength = enhance_strength(&mut ctx, voice_fac);
            let enhanced = pitch_enhance_with(&mut ctx, &code, strength);
            expect("pitchenh", &enhanced, blk);
        }
    }

    #[test]
    fn the_noise_enhancer_is_bit_exact_against_ts26173() {
        let mut ctx = DspContext::default();
        let mut enhancer = NoiseEnhancer::new();

        for blk in 0..5usize {
            // The same ISF pair the oracle builds.
            let mut isf = [Word16(0); 16];
            let mut isf_old = [Word16(0); 16];
            for i in 0..16usize {
                let base = 1000 + i16::try_from(i).expect("index") * 900;
                isf[i] = Word16(base);
                let offset = i16::try_from(blk * blk * 220 * (i % 3)).expect("perturbation fits");
                isf_old[i] = Word16(base + offset);
            }

            let want = block_row_i32("enhance", &format!("nenh{blk}"));
            let stab = stability_factor(&mut ctx, &isf, &isf_old);
            assert_eq!(i32::from(stab.0), want[0], "block {blk}: stability factor");

            let exc = excitation(blk);
            let code = innovation(blk);
            let meta = block_row("enhance", &format!("emeta{blk}"));
            let voice_fac =
                voice_factor(&mut ctx, &exc, -3, Word16(meta[0]), &code, Word16(meta[1]));

            let gain_code = Word32(want[2]);
            let out = enhancer.apply(&mut ctx, gain_code, voice_fac, stab);

            assert_eq!(enhancer.threshold.0, want[3], "block {blk}: threshold");
            assert_eq!(out.0, want[4], "block {blk}: enhanced gain");
        }
    }

    #[test]
    fn the_enhancer_does_nothing_on_a_voiced_stable_frame() {
        // The blend weight is noisiness x stability, so a strongly voiced
        // frame must pass its gain through untouched -- smoothing gain during
        // speech would blur attacks.
        let mut ctx = DspContext::default();
        let mut enhancer = NoiseEnhancer::new();
        let gain = Word32(1_000_000);
        let out = enhancer.apply(&mut ctx, gain, Word16(32767), Word16(32767));
        let drift = (out.0 - gain.0).abs();
        assert!(
            drift * 200 < gain.0,
            "a voiced frame's gain moved by {drift} of {}",
            gain.0
        );
    }

    #[test]
    fn an_unstable_filter_scores_zero_stability() {
        // A filter whose ISFs jump a long way between frames is in transition,
        // and the enhancer must stand down.
        let mut ctx = DspContext::default();
        let line = |i: usize| i16::try_from(i).expect("index") * 900 + 1000;
        let isf: [Word16; 16] = std::array::from_fn(|i| Word16(line(i)));
        let far: [Word16; 16] = std::array::from_fn(|i| Word16(line(i) + 4000));
        assert_eq!(stability_factor(&mut ctx, &isf, &far).0, 0);
        assert!(stability_factor(&mut ctx, &isf, &isf).0 > 30000);
    }

    #[test]
    fn dispersion_is_off_above_885_kbit_s() {
        // Above 8.85 there are enough pulses already; dispersing them would
        // only blur the excitation for nothing.
        assert_eq!(DispersionLevel::for_frame_bits(132), DispersionLevel::High);
        assert_eq!(DispersionLevel::for_frame_bits(177), DispersionLevel::Low);
        for bits in [253usize, 285, 317, 365, 397, 461, 477] {
            assert_eq!(
                DispersionLevel::for_frame_bits(bits),
                DispersionLevel::Off,
                "{bits} bits should not disperse"
            );
        }
    }

    #[test]
    fn dispersion_spreads_energy_without_creating_it() {
        // A convolution with a normalised impulse response redistributes
        // energy; a large change either way would mean the response or the
        // circular fold is wrong.
        let mut ctx = DspContext::default();
        let mut disp = PhaseDispersion::new();
        let mut code = innovation(0);

        let before: i64 = code.iter().map(|c| i64::from(c.0).pow(2)).sum();
        let nonzero_before = code.iter().filter(|c| c.0 != 0).count();

        disp.apply(
            &mut ctx,
            &mut code,
            Word16(500),
            Word16(4000),
            DispersionLevel::High,
        );

        let after: i64 = code.iter().map(|c| i64::from(c.0).pow(2)).sum();
        let nonzero_after = code.iter().filter(|c| c.0 != 0).count();

        assert!(
            nonzero_after > nonzero_before * 2,
            "dispersion left {nonzero_after} non-zero samples against {nonzero_before}"
        );
        assert!(
            after > before / 4 && after < before * 4,
            "energy moved from {before} to {after}"
        );
    }

    #[test]
    fn a_periodic_excitation_reads_as_voiced() {
        // voice_factor compares adaptive against algebraic energy, so a strong
        // pitch contribution and a weak innovation must come out positive.
        let mut ctx = DspContext::default();
        let exc = excitation(0);
        let quiet_code = [Word16(0); L_SUBFR];
        let voiced = voice_factor(&mut ctx, &exc, -3, Word16(15000), &quiet_code, Word16(1));

        let mut loud_code = [Word16(0); L_SUBFR];
        for (i, slot) in loud_code.iter_mut().enumerate() {
            *slot = Word16(if i % 2 == 0 { 512 } else { -512 });
        }
        let unvoiced = voice_factor(&mut ctx, &exc, -3, Word16(2000), &loud_code, Word16(8000));

        assert!(
            voiced.0 > unvoiced.0,
            "voiced measured {} against unvoiced {}",
            voiced.0,
            unvoiced.0
        );
    }

    #[test]
    fn the_pitch_enhancer_leaves_an_unvoiced_frame_nearly_alone() {
        // At voice_fac = 0 the strength is 0.25 in Q12, which is a gentle
        // filter; at full voicing it is far stronger.
        let mut ctx = DspContext::default();
        let code = innovation(0);

        let weak = enhance_strength(&mut ctx, Word16(0));
        let gentle = pitch_enhance_with(&mut ctx, &code, weak);
        let full = enhance_strength(&mut ctx, Word16(32767));
        let strong = pitch_enhance_with(&mut ctx, &code, full);

        let change = |v: &[Word16; L_SUBFR]| -> i64 {
            v.iter()
                .zip(code.iter())
                .map(|(a, b)| i64::from(a.0 - b.0).abs())
                .sum()
        };
        assert!(
            change(&gentle) < change(&strong),
            "unvoiced was changed {} against voiced {}",
            change(&gentle),
            change(&strong)
        );
    }
}
