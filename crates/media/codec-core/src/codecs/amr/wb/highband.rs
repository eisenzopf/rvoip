//! High-band synthesis, 3GPP TS 26.190 §6.10.
//!
//! **This is what makes the codec wideband.** The 12.8 kHz core carries content
//! to 6.4 kHz; everything above is generated here, not transmitted. Below 23.85
//! kbit/s not one bit describes the high band — it is noise, shaped and
//! levelled from what the low band happens to be doing.
//!
//! That sounds like a cheat and is not. Above about 6 kHz speech is mostly
//! fricative energy with little perceptually relevant fine structure; the ear
//! cares that the band is *present* and at the right level far more than what
//! is in it. Spending bits there would buy much less than spending them on the
//! core.
//!
//! # The chain
//!
//! 1. White noise from the reference's own generator — the decoder must
//!    reproduce it exactly, so it is a defined sequence, not real randomness.
//! 2. Scale it to the excitation's energy.
//! 3. Scale again by the low band's **spectral tilt**: voiced speech (falling
//!    spectrum) gets up to 14 dB less high-band noise than unvoiced. Injecting
//!    a flat noise level would make vowels sound hissy.
//! 4. Shape it with a bandwidth-expanded copy of the LP filter, so the noise
//!    follows the formant structure rather than sitting flat under it.
//! 5. Band-limit to roughly 6–7 kHz, and at 23.85 kbit/s low-pass at 7 kHz too.
//!
//! At 23.85 kbit/s the encoder transmits a gain index per subframe and step 3
//! is replaced by a table lookup — the only mode where the high band is
//! measured rather than guessed.

use super::excitation::L_SUBFR16K;
use super::gain_tables::{FIR_6K_7K, FIR_7K, HP_GAIN};
use super::math::{dot_product12, isqrt_n};
use super::synthesis::M;
use crate::fixed_point::arith::{add, extract_h, mult, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_mult};
use crate::fixed_point::div::div_s;
use crate::fixed_point::shift::{l_shl, norm_l, norm_s, shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Order of the high-band filter memory the reference reserves.
const M16K: usize = 20;

/// Length of the band-shaping FIR filters.
const L_FIR: usize = 31;

/// Bandwidth expansion for the noise-shaping filter, 0.6 in Q15.
const GAMMA_HF: Word16 = Word16(19661);

/// 400 Hz high-pass numerator, Q12 and quartered.
const HP400_B: [Word16; 3] = [Word16(915), Word16(-1830), Word16(915)];

/// 400 Hz high-pass denominator, Q12 and quadrupled.
const HP400_A: [Word16; 3] = [Word16(16384), Word16(29280), Word16(-14160)];

/// The reference's noise generator.
///
/// A defined sequence rather than real randomness: encoder and decoder must
/// produce identical noise, so "random" here means "unpredictable to the ear,
/// exactly reproducible to the codec".
#[derive(Debug, Clone)]
pub struct NoiseGenerator {
    seed: Word16,
}

impl Default for NoiseGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl NoiseGenerator {
    /// A generator at the decoder's documented initial seed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            seed: Word16(21845),
        }
    }

    /// The next sample of the sequence.
    pub fn next(&mut self, ctx: &mut DspContext) -> Word16 {
        // seed = (seed * 31821) / 2 + 13849, in 16 bits.
        let product = l_mult(ctx, self.seed, Word16(31821));
        let halved = crate::fixed_point::shift::l_shr(ctx, product, 1);
        let advanced = crate::fixed_point::arith32::l_add(ctx, halved, Word32(13849));
        self.seed = crate::fixed_point::arith::extract_l(advanced);
        self.seed
    }

    /// A subframe of noise, scaled down to leave headroom.
    #[must_use]
    pub fn fill(&mut self, ctx: &mut DspContext) -> [Word16; L_SUBFR16K] {
        let mut out = [Word16(0); L_SUBFR16K];
        for slot in &mut out {
            let sample = self.next(ctx);
            *slot = shr(ctx, sample, 3);
        }
        out
    }
}

/// The 400 Hz high-pass used to measure spectral tilt.
///
/// Separate from the 50 Hz output filter: this one exists only to strip
/// low-frequency energy before the tilt is measured, because a strong first
/// formant would otherwise dominate the correlation and make every frame look
/// voiced.
#[derive(Debug, Clone, Default)]
pub struct TiltFilter {
    y1_hi: Word16,
    y1_lo: Word16,
    y2_hi: Word16,
    y2_lo: Word16,
    x0: Word16,
    x1: Word16,
}

impl TiltFilter {
    /// A filter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            y1_hi: Word16(0),
            y1_lo: Word16(0),
            y2_hi: Word16(0),
            y2_lo: Word16(0),
            x0: Word16(0),
            x1: Word16(0),
        }
    }

    /// Filter one subframe in place. Output is divided by 16, which is what
    /// keeps the energy sum below that follows from overflowing.
    pub fn filter(&mut self, ctx: &mut DspContext, signal: &mut [Word16]) {
        for sample in signal.iter_mut() {
            let x2 = self.x1;
            self.x1 = self.x0;
            self.x0 = *sample;

            let mut acc = Word32(16384);
            acc = l_mac(ctx, acc, self.y1_lo, HP400_A[1]);
            acc = l_mac(ctx, acc, self.y2_lo, HP400_A[2]);
            acc = crate::fixed_point::shift::l_shr(ctx, acc, 15);
            acc = l_mac(ctx, acc, self.y1_hi, HP400_A[1]);
            acc = l_mac(ctx, acc, self.y2_hi, HP400_A[2]);
            acc = l_mac(ctx, acc, self.x0, HP400_B[0]);
            acc = l_mac(ctx, acc, self.x1, HP400_B[1]);
            acc = l_mac(ctx, acc, x2, HP400_B[2]);

            let acc = l_shl(ctx, acc, 1);

            self.y2_hi = self.y1_hi;
            self.y2_lo = self.y1_lo;
            let (hi, lo) = crate::fixed_point::oper32::l_extract(acc);
            self.y1_hi = hi;
            self.y1_lo = lo;

            *sample = round(ctx, acc);
        }
    }
}

/// A 31-tap FIR with carried history, used for both band-shaping filters.
#[derive(Debug, Clone)]
pub struct BandFilter {
    memory: [Word16; L_FIR - 1],
    taps: &'static [i16; L_FIR],
    /// Pre-scaling of the input, to leave room for the filter's own gain.
    prescale: i16,
}

impl BandFilter {
    /// The 6–7 kHz band-pass that confines the synthesised noise.
    #[must_use]
    pub const fn band_pass() -> Self {
        Self {
            memory: [Word16(0); L_FIR - 1],
            taps: &FIR_6K_7K,
            // The filter has a gain of 4, so the input is quartered first.
            prescale: 2,
        }
    }

    /// The 7 kHz low-pass, applied only at 23.85 kbit/s.
    #[must_use]
    pub const fn low_pass_7k() -> Self {
        Self {
            memory: [Word16(0); L_FIR - 1],
            taps: &FIR_7K,
            prescale: 0,
        }
    }

    /// Filter one subframe in place.
    pub fn filter(&mut self, ctx: &mut DspContext, signal: &mut [Word16]) {
        let mut x = [Word16(0); L_SUBFR16K + L_FIR - 1];
        x[..L_FIR - 1].copy_from_slice(&self.memory);
        for (i, &s) in signal.iter().enumerate() {
            x[i + L_FIR - 1] = shr(ctx, s, self.prescale);
        }

        for (i, slot) in signal.iter_mut().enumerate() {
            let mut acc = Word32(0);
            for (j, &tap) in self.taps.iter().enumerate() {
                acc = l_mac(ctx, acc, x[i + j], Word16(tap));
            }
            *slot = round(ctx, acc);
        }

        self.memory
            .copy_from_slice(&x[signal.len()..signal.len() + L_FIR - 1]);
    }
}

/// The noise-shaping synthesis filter's carried state.
#[derive(Debug, Clone)]
pub struct NoiseShaper {
    memory: [Word16; M16K],
}

impl Default for NoiseShaper {
    fn default() -> Self {
        Self::new()
    }
}

impl NoiseShaper {
    /// A filter with no history.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            memory: [Word16(0); M16K],
        }
    }

    /// Shape the noise with a bandwidth-expanded copy of `a`.
    ///
    /// Expansion widens the formants, so the noise follows the spectral shape
    /// without inheriting the sharp resonances — sharp peaks in a noise band
    /// sound like tones.
    pub fn shape(&mut self, ctx: &mut DspContext, a: &[Word16; M + 1], hf: &mut [Word16]) {
        let ap = weight(ctx, a, GAMMA_HF);

        // History in front, so the recursion reads its own past naturally.
        let mut y = [Word16(0); L_SUBFR16K + M];
        y[..M].copy_from_slice(&self.memory[M16K - M..]);

        let s = norm_s(ap[0]) - 2;
        let a0 = shr(ctx, ap[0], 1);

        for i in 0..hf.len() {
            let mut acc = l_mult(ctx, hf[i], a0);
            for j in 1..=M {
                acc = crate::fixed_point::arith32::l_msu(ctx, acc, ap[j], y[M + i - j]);
            }
            let acc = l_shl(ctx, acc, 3 + s);
            let sample = round(ctx, acc);
            y[M + i] = sample;
            hf[i] = sample;
        }

        self.memory[M16K - M..].copy_from_slice(&y[hf.len()..hf.len() + M]);
    }
}

/// Bandwidth-expand a predictor: `ap[i] = a[i]·γ^i`.
#[must_use]
pub fn weight(ctx: &mut DspContext, a: &[Word16; M + 1], gamma: Word16) -> [Word16; M + 1] {
    let mut ap = [Word16(0); M + 1];
    ap[0] = a[0];
    let mut fac = gamma;
    for i in 1..M {
        let scaled = l_mult(ctx, a[i], fac);
        ap[i] = round(ctx, scaled);
        let next = l_mult(ctx, fac, gamma);
        fac = round(ctx, next);
    }
    let last = l_mult(ctx, a[M], fac);
    ap[M] = round(ctx, last);
    ap
}

/// Scale a noise vector to match the excitation's energy.
///
/// Returns the noise scaled so its energy equals the excitation's, which is the
/// starting point before the tilt-derived attenuation is applied.
pub fn match_energy(ctx: &mut DspContext, excitation: &[Word16], hf: &mut [Word16], q_new: i16) {
    let (energy, exp_ener) = dot_product12(ctx, excitation, excitation);
    let ener = extract_h(energy);
    let exp_ener = exp_ener - 2 * q_new;

    let (noise_energy, mut exp) = dot_product12(ctx, hf, hf);
    let mut tmp = extract_h(noise_energy);
    // div_s needs a proper fraction, so make sure the numerator is smaller.
    if tmp.0 > ener.0 {
        tmp = shr(ctx, tmp, 1);
        exp += 1;
    }

    let ratio = l_deposit_h(div_s(tmp, ener));
    let (frac, exp) = isqrt_n(ctx, (ratio, exp - exp_ener));
    let scale = extract_h(l_shl(ctx, frac, exp + 1));

    for sample in hf.iter_mut() {
        *sample = mult(ctx, *sample, scale);
    }
}

/// The low band's spectral tilt, as a normalised first-order correlation.
///
/// 1 is strongly voiced (falling spectrum), 0 unvoiced. `synth` is consumed:
/// the 400 Hz high-pass is applied in place.
#[must_use]
pub fn spectral_tilt(ctx: &mut DspContext, filter: &mut TiltFilter, synth: &mut [Word16]) -> Word16 {
    filter.filter(ctx, synth);

    let mut acc = Word32(1);
    for &s in synth.iter() {
        acc = l_mac(ctx, acc, s, s);
    }
    let exp = norm_l(acc);
    let energy = extract_h(l_shl(ctx, acc, exp));

    let mut acc = Word32(1);
    for i in 1..synth.len() {
        acc = l_mac(ctx, acc, synth[i], synth[i - 1]);
    }
    let correlation = extract_h(l_shl(ctx, acc, exp));

    if correlation.0 > 0 {
        div_s(correlation, energy)
    } else {
        Word16(0)
    }
}

/// Derive the high-band gain from the tilt.
///
/// `vad_history` non-zero means recent speech, which selects a lower gain — the
/// high band is attenuated harder during speech than during background noise,
/// where the noise floor should sound continuous.
#[must_use]
pub fn gain_from_tilt(ctx: &mut DspContext, tilt: Word16, vad_history: i16) -> Word16 {
    let gain1 = sub(ctx, Word16(32767), tilt);
    let complement = sub(ctx, Word16(32767), tilt);
    let scaled = mult(ctx, complement, Word16(20480));
    let gain2 = shl(ctx, scaled, 1);

    let (weight1, weight2) = if vad_history > 0 {
        (Word16(0), Word16(32767))
    } else {
        (Word16(32767), Word16(0))
    };

    let a = mult(ctx, weight1, gain1);
    let b = mult(ctx, weight2, gain2);
    let mut tmp = add(ctx, a, b);
    if tmp.0 != 0 {
        tmp = add(ctx, tmp, Word16(1));
    }
    // Never fully silent: 0.1 keeps a little band energy so the switch between
    // voiced and unvoiced is not an audible gate.
    if tmp.0 < 3277 {
        tmp = Word16(3277);
    }
    tmp
}

/// The transmitted high-band gain for 23.85 kbit/s, Q14.
///
/// # Panics
///
/// If `index` is outside the 4-bit range the bitstream carries.
#[must_use]
pub const fn transmitted_gain(index: u16) -> Word16 {
    Word16(HP_GAIN[index as usize])
}

/// Order of the extrapolated high-band filter.
pub const M16K_ORDER: usize = M16K;

/// 1/12 in Q15, for the mean of the difference vector.
const INV_LENGTH: Word16 = Word16(2731);

/// Extrapolate an order-20 high-band filter from the low band's ISFs.
///
/// **Only the 6.60 kbit/s mode uses this.** Every other mode shapes the high
/// band with a bandwidth-expanded copy of the low band's own filter; at 6.60
/// there is not enough spectral detail to borrow, so a filter is invented.
///
/// The invention is not arbitrary. The *spacing* between consecutive ISFs
/// carries the formant structure, and that spacing tends to be quasi-periodic.
/// Three lag hypotheses (2, 3 and 4) are correlated against the observed
/// spacing and the best-fitting one is continued outward — so the synthesised
/// high band inherits the talker's formant rhythm rather than a flat guess.
///
/// `isf` holds the 16 low-band ISFs on entry and receives 20 ISPs on exit.
///
/// # Panics
///
/// If `isf` is shorter than [`M16K_ORDER`].
pub fn extrapolate_isf(ctx: &mut DspContext, isf: &mut [Word16]) {
    use crate::fixed_point::arith32::{l_add, l_sub};
    use crate::fixed_point::oper32::{l_extract, mpy_32};

    isf[M16K - 1] = isf[M - 1];

    // Spacing between consecutive line frequencies.
    let mut diff = [Word16(0); M - 2];
    for i in 1..M - 1 {
        diff[i - 1] = sub(ctx, isf[i], isf[i - 1]);
    }

    // Mean spacing, over the upper part only: the lowest few lines sit where
    // the spectrum is dominated by the first formant and are not typical.
    let mut acc = Word32(0);
    for i in 3..M - 1 {
        acc = l_mac(ctx, acc, diff[i - 1], INV_LENGTH);
    }
    let mut mean = round(ctx, acc);

    // Normalise on the widest spacing, so the correlations below use the full
    // dynamic range whatever the talker's absolute formant spread.
    let mut peak = Word16(0);
    for d in &diff {
        if d.0 > peak.0 {
            peak = *d;
        }
    }
    let exp = norm_s(peak);
    for d in &mut diff {
        *d = shl(ctx, *d, exp);
    }
    mean = shl(ctx, mean, exp);

    // Correlate the spacing against itself at lags 2, 3 and 4.
    let mut corr = [Word32(0); 3];
    for (slot, lag) in corr.iter_mut().zip([2usize, 3, 4]) {
        for i in 7..M - 2 {
            let a = sub(ctx, diff[i], mean);
            let b = sub(ctx, diff[i - lag], mean);
            let product = l_mult(ctx, a, b);
            let (hi, lo) = l_extract(product);
            *slot = l_add(ctx, *slot, mpy_32(hi, lo, hi, lo));
        }
    }

    let mut best = usize::from(l_sub(ctx, corr[0], corr[1]).0 <= 0);
    if l_sub(ctx, corr[2], corr[best]).0 > 0 {
        best = 2;
    }
    let lag = best + 1;

    // Continue the pattern outward at the winning lag.
    for i in M - 1..M16K - 1 {
        let step = sub(ctx, isf[i - 1 - lag], isf[i - 2 - lag]);
        isf[i] = add(ctx, isf[i - 1], step);
    }

    // Where the extrapolation should end: 7965 + (isf2 - isf3 - isf4)/6,
    // capped so the top line never exceeds 7600 Hz.
    let sum = add(ctx, isf[4], isf[3]);
    let mut target = sub(ctx, isf[2], sum);
    target = mult(ctx, target, Word16(5461));
    target = add(ctx, target, Word16(20390));
    if target.0 > 19456 {
        target = Word16(19456);
    }

    // Stretch the extrapolated part to land exactly on that target.
    let wanted = sub(ctx, target, isf[M - 2]);
    let actual = sub(ctx, isf[M16K - 2], isf[M - 2]);
    let exp2 = norm_s(actual);
    let exp = norm_s(wanted) - 1;
    let coeff = div_s(shl(ctx, wanted, exp), shl(ctx, actual, exp2));
    let exp = exp2 - exp;

    let mut stretched = [Word16(0); M16K - M];
    for i in M - 1..M16K - 1 {
        let step = sub(ctx, isf[i], isf[i - 1]);
        let scaled = mult(ctx, step, coeff);
        stretched[i - (M - 1)] = shl(ctx, scaled, exp);
    }

    // Two adjacent lines closer than 500 Hz would make the filter ring, so
    // widen whichever gap is larger and keep the pair's total.
    for i in M..M16K - 1 {
        let (a, b) = (stretched[i - (M - 1)], stretched[i - M]);
        if add(ctx, a, b).0 - 1280 < 0 {
            if a.0 > b.0 {
                stretched[i - M] = sub(ctx, Word16(1280), a);
            } else {
                stretched[i - (M - 1)] = sub(ctx, Word16(1280), b);
            }
        }
    }

    for i in M - 1..M16K - 1 {
        isf[i] = add(ctx, isf[i - 1], stretched[i - (M - 1)]);
    }

    // The low band's ISFs are normalised to 12.8 kHz; 0.8 rescales them to the
    // 16 kHz grid the high-band filter lives on.
    for slot in isf.iter_mut().take(M16K - 1) {
        *slot = mult(ctx, *slot, Word16(26214));
    }

    super::lp::isf::isf_to_isp_in_place(ctx, isf);
}


#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_row, has_block};
    use super::*;

    const A_REAL: [i16; M + 1] = [
        4096, -3559, 1097, -175, -313, 292, -73, -119, 158, -83, -20, 72, -60, 12, 25, -31, 12,
    ];

    fn expect(label: &str, got: &[Word16], blk: usize) {
        let want = block_row("highband", &format!("{label}{blk}"));
        assert_eq!(want.len(), got.len(), "{label}{blk}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w,
                "{label}{blk}: sample {i} = {} but the reference gives {w}",
                g.0
            );
        }
    }

    fn tone(block: usize, amplitude: f64, period: f64, len: usize) -> Vec<Word16> {
        (0..len)
            .map(|n| {
                #[allow(clippy::cast_precision_loss)]
                let t = (block * len + n) as f64;
                #[allow(clippy::cast_possible_truncation)]
                Word16((amplitude * (2.0 * std::f64::consts::PI * t / period).sin()) as i16)
            })
            .collect()
    }

    #[test]
    fn the_high_band_chain_is_bit_exact_against_ts26173() {
        assert!(has_block("highband"), "fixture block highband missing");
        let a: [Word16; M + 1] = A_REAL.map(Word16);

        let mut noise = NoiseGenerator::new();
        let mut tilt_filter = TiltFilter::new();
        let mut shaper = NoiseShaper::new();
        let mut band = BandFilter::band_pass();
        let mut low7k = BandFilter::low_pass_7k();
        let mut ctx = DspContext::default();

        for blk in 0..3 {
            let vad_history = i16::from(blk == 2);

            let mut exc: Vec<Word16> = tone(blk, 3000.0, 29.0, 64);
            let mut synth: Vec<Word16> = tone(blk, 6000.0, 61.0, 64);
            expect("hexc", &exc, blk);
            expect("hsyn", &synth, blk);

            let mut hf = noise.fill(&mut ctx);
            expect("noise", &hf, blk);

            // The reference scales the excitation down by 3 bits before the
            // energy sum, so q_new goes with it.
            for s in &mut exc {
                *s = shr(&mut ctx, *s, 3);
            }
            match_energy(&mut ctx, &exc, &mut hf, -3);
            expect("matched", &hf, blk);

            let tilt = spectral_tilt(&mut ctx, &mut tilt_filter, &mut synth);
            expect("hp400_", &synth, blk);

            let meta = block_row("highband", &format!("hmeta{blk}"));
            assert_eq!(i32::from(tilt.0), i32::from(meta[0]), "block {blk}: tilt");

            let gain = gain_from_tilt(&mut ctx, tilt, vad_history);
            assert_eq!(i32::from(gain.0), i32::from(meta[1]), "block {blk}: gain");

            for s in &mut hf {
                *s = mult(&mut ctx, *s, gain);
            }
            expect("tilted", &hf, blk);

            let ap = weight(&mut ctx, &a, GAMMA_HF);
            expect("weighted", &ap, blk);

            shaper.shape(&mut ctx, &a, &mut hf);
            expect("shaped", &hf, blk);

            band.filter(&mut ctx, &mut hf);
            expect("band", &hf, blk);

            low7k.filter(&mut ctx, &mut hf);
            expect("lp7k", &hf, blk);
        }
    }

    #[test]
    fn isf_extrapolation_is_bit_exact_against_ts26173() {
        assert!(has_block("isfextrp"), "fixture block isfextrp missing");
        let mut ctx = DspContext::default();

        for c in 0..4 {
            let want_in = block_row("isfextrp", &format!("xin{c}"));
            let mut isf = vec![Word16(0); M16K];
            for (i, &v) in want_in.iter().enumerate() {
                isf[i] = Word16(v);
            }

            extrapolate_isf(&mut ctx, &mut isf);
            expect_named("isfextrp", "xout", &isf, c);
        }
    }

    fn expect_named(block: &str, label: &str, got: &[Word16], blk: usize) {
        let want = block_row(block, &format!("{label}{blk}"));
        assert_eq!(want.len(), got.len(), "{label}{blk}: length");
        for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
            assert_eq!(
                g.0, w,
                "{label}{blk}: value {i} = {} but the reference gives {w}",
                g.0
            );
        }
    }

    #[test]
    fn the_noise_sequence_is_reproducible_not_random() {
        // Encoder and decoder must generate identical noise, so two generators
        // from the same seed must agree forever.
        let mut ctx = DspContext::default();
        let mut a = NoiseGenerator::new();
        let mut b = NoiseGenerator::new();
        for i in 0..500 {
            assert_eq!(a.next(&mut ctx), b.next(&mut ctx), "diverged at sample {i}");
        }
    }

    #[test]
    fn the_noise_sequence_does_not_repeat_quickly() {
        // A short cycle would be audible as a tone in the high band.
        let mut ctx = DspContext::default();
        let mut gen = NoiseGenerator::new();
        let first = gen.next(&mut ctx);
        for i in 1..2000 {
            if gen.next(&mut ctx) == first {
                assert!(i > 500, "the noise sequence repeated after {i} samples");
            }
        }
    }

    #[test]
    fn a_voiced_spectrum_gets_less_high_band_than_an_unvoiced_one() {
        // The whole point of the tilt measurement: injecting a flat noise level
        // would make vowels hiss.
        let mut ctx = DspContext::default();
        // High tilt means strongly voiced.
        let voiced = gain_from_tilt(&mut ctx, Word16(30000), 0);
        let unvoiced = gain_from_tilt(&mut ctx, Word16(0), 0);
        assert!(
            voiced.0 < unvoiced.0,
            "voiced got gain {} against unvoiced {}",
            voiced.0,
            unvoiced.0
        );
    }

    #[test]
    fn the_high_band_gain_never_reaches_silence() {
        // A hard gate between voiced and unvoiced would be audible as the band
        // switching on and off, so there is a floor.
        let mut ctx = DspContext::default();
        for tilt in [0i16, 8000, 16384, 30000, 32767] {
            for vad in [0i16, 1] {
                let gain = gain_from_tilt(&mut ctx, Word16(tilt), vad);
                assert!(
                    gain.0 >= 3277,
                    "tilt {tilt}, vad {vad} gave gain {}",
                    gain.0
                );
            }
        }
    }

    #[test]
    fn bandwidth_expansion_shrinks_the_higher_coefficients_most() {
        // ap[i] = a[i]·γ^i, so the tail is damped hardest -- that is what
        // widens the formants rather than just scaling the filter.
        let mut ctx = DspContext::default();
        let a: [Word16; M + 1] = A_REAL.map(Word16);
        let ap = weight(&mut ctx, &a, GAMMA_HF);

        assert_eq!(ap[0], a[0], "the leading coefficient must be untouched");
        let ratio = |i: usize| f64::from(ap[i].0) / f64::from(a[i].0);
        assert!(
            ratio(1).abs() > ratio(8).abs(),
            "coefficient 8 was not damped harder than coefficient 1"
        );
    }

    #[test]
    fn the_transmitted_gains_are_monotonic() {
        // The 23.85 table is a quantiser, so its entries must ascend or the
        // encoder's nearest-neighbour search would pick wrongly.
        for i in 1..16u16 {
            assert!(
                transmitted_gain(i).0 > transmitted_gain(i - 1).0,
                "gain table is not ascending at index {i}"
            );
        }
    }
}
