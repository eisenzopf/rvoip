//! The assembled AMR-WB decoder, 3GPP TS 26.190 §6.
//!
//! Wires the verified stages into a decoder: payload in, 16 kHz PCM out.
//!
//! # Two excitations, and why
//!
//! The single most error-prone thing in this file is that there are **two**
//! excitation signals per subframe and they must not be conflated:
//!
//! - `exc` — the plain sum of adaptive and algebraic contributions. This is
//!   written back to the adaptive-codebook history, because it is what the
//!   *encoder* believes it produced. Anything else and the two pitch
//!   predictors drift apart over seconds.
//! - `exc2` — the same sum built from the *enhanced* innovation and gain
//!   (phase dispersion, noise enhancer, pitch enhancer). This is what the
//!   synthesis filter runs on, and it never re-enters the history.
//!
//! Using one buffer for both produces audio that is recognisably speech and
//! steadily wrong — the failure mode no per-stage test can catch.

use super::codebook::{self, L_SUBFR};
use super::enhance::{
    agc2, enhance_strength, pitch_enhance_with, stability_factor, voice_factor,
    DispersionLevel, NoiseEnhancer, PhaseDispersion,
};
use super::excitation::{Excitation, Upsampler, L_SUBFR16K};
use super::gain::{FrameQuality, GainDecoder};
use super::highband::{self, BandFilter, NoiseGenerator, NoiseShaper, TiltFilter};
use super::lp::autocorr::LP_ORDER;
use super::lp::isf::{interpolate_isp, isf_to_isp, NB_SUBFR};
use super::lp::isf_dequant::{IsfDecoder, IsfQuantizer};
use super::ltp::{self, PIT_SHARP};
use super::params::FrameParams;
use super::synthesis::{deemphasis, HighPass50, SynthesisFilter, PREEMPH_FAC};
use crate::codecs::amr::mode::AmrMode;
use crate::fixed_point::arith::{add, mult, round};
use crate::fixed_point::oper32::l_extract;
use crate::fixed_point::shift::{l_shl, shl, shr};
use crate::fixed_point::types::{DspContext, Word16};

/// Samples one frame produces, at 16 kHz.
pub const FRAME_SIZE_16K: usize = 320;

/// Frame sizes in bits, by mode index.
const FRAME_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

/// A complete AMR-WB decoder.
#[derive(Debug, Clone)]
pub struct Decoder {
    isf: IsfDecoder,
    gains: GainDecoder,
    excitation: Excitation,
    dispersion: PhaseDispersion,
    noise_enhancer: NoiseEnhancer,
    synthesis: SynthesisFilter,
    deemph_memory: Word16,
    high_pass: HighPass50,
    upsampler: Upsampler,
    noise: NoiseGenerator,
    tilt_filter: TiltFilter,
    shaper: NoiseShaper,
    band_pass: BandFilter,
    low_pass_7k: BandFilter,
    /// Previous frame's ISPs, for interpolation.
    isp_old: [Word16; LP_ORDER],
    /// Previous frame's ISFs, for the stability measure.
    isf_old: [Word16; LP_ORDER],
    /// Spectral tilt of the innovation, carried to the next subframe.
    tilt_code: Word16,
    /// Whether any frame has been decoded yet.
    started: bool,
    /// Consecutive frames the encoder marked as containing no voice activity.
    ///
    /// Selects a different high-band gain curve: during background noise the
    /// band is attenuated less, so the noise floor sounds continuous rather
    /// than gated.
    vad_history: i16,
    /// Scalar trace of the last decode.
    ///
    /// Exists to be diffed against the instrumented reference decoder — see
    /// `tools/trace-amr-reference.sh`. Comparing intermediates found every bug
    /// in this file so far; reasoning from output PCM found none of them.
    #[cfg(test)]
    pub(crate) trace: Vec<(&'static str, i64)>,
    /// Vector trace of the last decode, same purpose.
    #[cfg(test)]
    pub(crate) vtrace: Vec<(&'static str, Vec<i16>)>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder in its reset state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            isf: IsfDecoder::new(),
            gains: GainDecoder::new(),
            excitation: Excitation::new(),
            dispersion: PhaseDispersion::new(),
            noise_enhancer: NoiseEnhancer::new(),
            synthesis: SynthesisFilter::new(),
            deemph_memory: Word16(0),
            high_pass: HighPass50::new(),
            upsampler: Upsampler::new(),
            noise: NoiseGenerator::new(),
            tilt_filter: TiltFilter::new(),
            shaper: NoiseShaper::new(),
            band_pass: BandFilter::band_pass(),
            low_pass_7k: BandFilter::low_pass_7k(),
            isp_old: [Word16(0); LP_ORDER],
            isf_old: [Word16(0); LP_ORDER],
            tilt_code: Word16(0),
            started: false,
            vad_history: 0,
            #[cfg(test)]
            trace: Vec::new(),
            #[cfg(test)]
            vtrace: Vec::new(),
        }
    }

    /// Decode one frame's payload into 320 samples at 16 kHz.
    ///
    /// Returns `None` if the payload does not parse for this mode.
    ///
    /// Deliberately one function: the subframe loop is a single sequence whose
    /// order matters at every step, and splitting it would hide the two
    /// excitations' divergence behind a call boundary.
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn decode(&mut self, mode: AmrMode, payload: &[u8]) -> Option<[i16; FRAME_SIZE_16K]> {
        let frame_bits = *FRAME_BITS.get(mode.index() as usize)?;
        let params = FrameParams::parse(mode, payload)?;
        let mut ctx = DspContext::default();
        #[cfg(test)]
        {
            self.trace.clear();
            self.vtrace.clear();
        }
        macro_rules! vtrace {
            ($n:expr, $v:expr) => {
                #[cfg(test)]
                self.vtrace
                    .push(($n, $v.iter().map(|w: &Word16| w.0).collect()));
            };
        }
        macro_rules! trace {
            ($n:expr, $v:expr) => {
                #[cfg(test)]
                self.trace.push(($n, i64::from($v)));
            };
        }

        // --- spectrum ---
        let quantizer = if frame_bits <= 132 {
            IsfQuantizer::Bits36
        } else {
            IsfQuantizer::Bits46
        };
        // The VAD flag counts up while the encoder sees no speech and resets
        // the moment it does.
        if params.vad_flag {
            self.vad_history = 0;
        } else {
            self.vad_history = self.vad_history.saturating_add(1);
        }

        let isf = self.isf.decode(quantizer, &params.isf_indices, false);
        let stab_fac = stability_factor(&mut ctx, &isf, &self.isf_old);
        self.isf_old = isf;

        trace!("stab_fac", stab_fac.0);
        let isp_new = isf_to_isp(&isf);
        if !self.started {
            // Nothing to interpolate from on the first frame.
            self.isp_old = isp_new;
            self.started = true;
        }
        let coefficients = interpolate_isp(&self.isp_old, &isp_new);
        self.isp_old = isp_new;

        // --- excitation and synthesis, subframe by subframe ---
        let mut out = [0i16; FRAME_SIZE_16K];
        let gain_bits = if frame_bits <= 177 { 6 } else { 7 };
        let dispersion = DispersionLevel::for_frame_bits(frame_bits);

        for (sf, sub) in params.subframes.iter().enumerate().take(NB_SUBFR) {
            // Adaptive codebook. One extra sample: the low-pass reads ahead.
            let (buffer, offset) = self.excitation.buffer_mut();
            ltp::predict(
                buffer,
                offset,
                sub.pitch_lag as usize,
                sub.pitch_frac,
                L_SUBFR + 1,
            );
            #[cfg(test)]
            let pred_snapshot: Vec<i16> =
                buffer[offset..offset + L_SUBFR].iter().map(|w| w.0).collect();
            #[cfg(test)]
            self.vtrace.push(("pred", pred_snapshot));
            // Below 12.65 the filter is always on; above it the encoder picks.
            if !sub.ltp_filter {
                let smoothed = ltp::low_pass(&buffer[offset - 1..]);
                buffer[offset..offset + L_SUBFR].copy_from_slice(&smoothed);
            }

            trace!("T0", sub.pitch_lag);
            trace!("T0_frac", sub.pitch_frac);
            trace!("tilt_code_used", self.tilt_code.0);
            // Algebraic codebook, shaped by the previous subframe's tilt and
            // sharpened at the pitch period.
            let mut code: [Word16; L_SUBFR] = codebook::decode(&sub.pulses, frame_bits)?.map(Word16);
            let mut discard = Word16(0);
            ltp::preemphasis(&mut code, self.tilt_code, &mut discard);
            ltp::sharpen(
                &mut code,
                ltp::sharpening_lag(sub.pitch_lag as usize, sub.pitch_frac),
                PIT_SHARP,
            );

            vtrace!("code", code);
            let gains = self.gains.decode(
                sub.gain_index,
                gain_bits,
                &code,
                FrameQuality::Good,
                0,
                self.vad_history,
            );

            trace!("gain_pit", gains.pitch.0);
            trace!("L_gain_code", gains.code.0);
            // Scale the buffer first: the adaptive-only copy below must be at
            // the new scale, which is what the reference captures.
            let (q_new, gain_code_word) = self.excitation.rescale_to(gains.code);

            trace!("Q_new", q_new);
            trace!("gain_code_scaled", gain_code_word.0);
            // The adaptive-only excitation, before the total is built over it.
            // This is what voice_factor and the enhanced path start from.
            let (buffer, offset) = self.excitation.buffer_mut();
            let mut exc2 = [Word16(0); L_SUBFR];
            exc2.copy_from_slice(&buffer[offset..offset + L_SUBFR]);

            // The plain total, written back to the history.
            self.excitation.build(&code, gains.pitch, gain_code_word, q_new);

            #[cfg(test)]
            {
                let (b, o) = self.excitation.buffer_mut();
                let t: Vec<i16> = b[o..o + L_SUBFR].iter().map(|w| w.0).collect();
                self.vtrace.push(("exc_total", t));
            }
            // voice_factor works on the adaptive part scaled down by 3 bits.
            let mut scaled = exc2;
            for s in &mut scaled {
                *s = shr(&mut ctx, *s, 3);
            }
            let voice_fac = voice_factor(
                &mut ctx,
                &scaled,
                -3,
                gains.pitch,
                &code,
                gain_code_word,
            );
            // Tilt for the next subframe: 0.5 voiced, 0 unvoiced.
            let quartered = shr(&mut ctx, voice_fac, 2);
            self.tilt_code = add(&mut ctx, quartered, Word16(8192));

            // At the low rates a strongly voiced subframe gets an extra
            // sharpened copy blended in later. It is built from the
            // adaptive-only excitation *before* the total is assembled over
            // it, so it has to be computed here.
            let pit_sharp = shl(&mut ctx, gains.pitch, 1);
            let sharpened = if frame_bits <= 177 && pit_sharp.0 > 16384 {
                let mut excp = [Word16(0); L_SUBFR];
                for (i, slot) in excp.iter_mut().enumerate() {
                    let t = mult(&mut ctx, scaled[i], pit_sharp);
                    let acc = crate::fixed_point::arith32::l_mult(&mut ctx, t, gains.pitch);
                    let acc = crate::fixed_point::shift::l_shr(&mut ctx, acc, 1);
                    *slot = round(&mut ctx, acc);
                }
                Some(excp)
            } else {
                None
            };

            trace!("voice_fac", voice_fac.0);
            // The enhanced path, for the listener only.
            // Phase dispersion takes the HIGH HALF of the Q16 code gain, not
            // a rounded or rescaled version of it -- the reference splits
            // L_gain_code with L_Extract and passes the high word.
            let (gain_code_hi, _) = l_extract(gains.code);
            self.dispersion
                .apply(&mut ctx, &mut code, gain_code_hi, gains.pitch, dispersion);
            let enhanced_gain =
                self.noise_enhancer
                    .apply(&mut ctx, gains.code, voice_fac, stab_fac);
            let strength = enhance_strength(&mut ctx, voice_fac);
            let code2 = pitch_enhance_with(&mut ctx, &code, strength);

            let lifted = l_shl(&mut ctx, enhanced_gain, q_new);
            let enhanced_gain_word = round(&mut ctx, lifted);
            for i in 0..L_SUBFR {
                let mut acc =
                    crate::fixed_point::arith32::l_mult(&mut ctx, code2[i], enhanced_gain_word);
                acc = l_shl(&mut ctx, acc, 5);
                acc = crate::fixed_point::arith32::l_mac(&mut ctx, acc, exc2[i], gains.pitch);
                let acc = l_shl(&mut ctx, acc, 1);
                exc2[i] = round(&mut ctx, acc);
            }

            // Blend the sharpened copy in, matching its loudness to the
            // excitation it replaces so the sharpening is heard as periodicity
            // rather than as a level jump.
            if let Some(mut excp) = sharpened {
                for (i, slot) in excp.iter_mut().enumerate() {
                    *slot = add(&mut ctx, *slot, exc2[i]);
                }
                agc2(&mut ctx, &exc2, &mut excp);
                exc2 = excp;
            }

            vtrace!("exc2_final", exc2);
            // --- synthesis ---
            let a = &coefficients[sf];
            let (high, low) = self.synthesis.filter(a, &exc2, q_new);
            let mut speech = deemphasis(&high, &low, PREEMPH_FAC, &mut self.deemph_memory);
            self.high_pass.filter(&mut speech);
            let upsampled = self.upsampler.process(&speech);

            // --- high band ---
            let mut hf = self.noise.fill(&mut ctx);
            let mut energy_source = exc2;
            for s in &mut energy_source {
                *s = shr(&mut ctx, *s, 3);
            }
            highband::match_energy(&mut ctx, &energy_source, &mut hf, q_new - 3);

            let tilt = highband::spectral_tilt(&mut ctx, &mut self.tilt_filter, &mut speech);
            if let Some(index) = sub.hf_gain {
                // 23.85 transmits the gain, and applies it with an extra
                // doubling the estimated path does not have: the reference is
                // shl(mult(HF, gain), 1), not mult alone.
                let gain = highband::transmitted_gain(index);
                for s in &mut hf {
                    let scaled = mult(&mut ctx, *s, gain);
                    *s = shl(&mut ctx, scaled, 1);
                }
            } else {
                let gain = highband::gain_from_tilt(&mut ctx, tilt, self.vad_history);
                for s in &mut hf {
                    *s = mult(&mut ctx, *s, gain);
                }
            }
            vtrace!("hfnoise_scaled", hf);
            self.shaper.shape(&mut ctx, a, &mut hf);
            self.band_pass.filter(&mut ctx, &mut hf);
            if frame_bits >= 477 {
                self.low_pass_7k.filter(&mut ctx, &mut hf);
            }

            vtrace!("hfband", hf);
            let base = sf * L_SUBFR16K;
            for i in 0..L_SUBFR16K {
                out[base + i] = add(&mut ctx, upsampled[i], hf[i]).0;
            }

            self.excitation.advance();
        }

        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::amr::mode::AmrVariant;
    use crate::codecs::amr::storage;

    fn fixture(mode_index: usize) -> (&'static [u8], &'static [u8]) {
        const BITS: [&[u8]; 9] = [
            include_bytes!("../testdata/amrwb_mode0.amr"),
            include_bytes!("../testdata/amrwb_mode1.amr"),
            include_bytes!("../testdata/amrwb_mode2.amr"),
            include_bytes!("../testdata/amrwb_mode3.amr"),
            include_bytes!("../testdata/amrwb_mode4.amr"),
            include_bytes!("../testdata/amrwb_mode5.amr"),
            include_bytes!("../testdata/amrwb_mode6.amr"),
            include_bytes!("../testdata/amrwb_mode7.amr"),
            include_bytes!("../testdata/amrwb_mode8.amr"),
        ];
        const PCM: [&[u8]; 9] = [
            include_bytes!("../testdata/amrwb_mode0.pcm"),
            include_bytes!("../testdata/amrwb_mode1.pcm"),
            include_bytes!("../testdata/amrwb_mode2.pcm"),
            include_bytes!("../testdata/amrwb_mode3.pcm"),
            include_bytes!("../testdata/amrwb_mode4.pcm"),
            include_bytes!("../testdata/amrwb_mode5.pcm"),
            include_bytes!("../testdata/amrwb_mode6.pcm"),
            include_bytes!("../testdata/amrwb_mode7.pcm"),
            include_bytes!("../testdata/amrwb_mode8.pcm"),
        ];
        (BITS[mode_index], PCM[mode_index])
    }

    /// Reference PCM as samples.
    fn reference(pcm: &[u8]) -> Vec<i16> {
        pcm.chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    /// Decode a mode and report where it first departs from the reference.
    fn compare(mode_index: usize) -> (usize, usize, i64) {
        let (bits, pcm) = fixture(mode_index);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode =
            AmrMode::new(AmrVariant::WideBand, u8::try_from(mode_index).expect("index"))
                .expect("mode");

        let mut dec = Decoder::new();
        let mut matched = 0usize;
        let mut total = 0usize;
        let mut worst = 0i64;

        for (f, frame) in frames.iter().enumerate() {
            let Some(got) = dec.decode(mode, &frame.data) else {
                break;
            };
            for (i, &g) in got.iter().enumerate() {
                let index = f * FRAME_SIZE_16K + i;
                if index >= want.len() {
                    break;
                }
                total += 1;
                // AMR-WB's output is defined as 14-bit linear: the reference
                // harness masks the low two bits before writing (decoder.c
                // line 211, `synth[i] & 0xfffC`). Comparing unmasked 16-bit
                // output against it is comparing different things.
                let g = g & !3i16;
                let delta = i64::from(g) - i64::from(want[index]);
                worst = worst.max(delta.abs());
                if delta == 0 {
                    matched += 1;
                }
            }
        }
        (matched, total, worst)
    }

    /// Diagnostic for the assembly's remaining error, kept because it is how
    /// the next step starts rather than because it asserts anything.
    ///
    /// Run with `--ignored --nocapture`, and compare against the instrumented
    /// reference produced by `tools/trace-amr-reference.sh`. The two emit the
    /// same intermediates under the same names.
    ///
    /// Verified matching for mode 12.65, frame 0, subframes 0 and 1: every
    /// scalar (`T0`, gains, `Q_new`, `voice_fac`) and every vector (`pred`,
    /// `code`, `exc_total`, `exc2_final`, `hfband`) is identical to the
    /// reference. The remaining error is therefore downstream of the
    /// excitation.
    #[test]
    #[ignore = "diagnostic, not an assertion"]
    fn where_does_it_diverge() {
        let mode_index = 2;
        let (bits, pcm) = fixture(mode_index);
        let want = reference(pcm);
        let (_, frames) = storage::read(bits).expect("fixture parses");
        let mode = AmrMode::new(AmrVariant::WideBand, 2).expect("mode");
        let mut dec = Decoder::new();
        let got = dec.decode(mode, &frames[0].data).expect("decodes");

        for (k, v) in &dec.trace {
            println!("S {k} = {v}");
        }
        for (k, v) in &dec.vtrace {
            println!("V {k}: {:?}", &v[..6.min(v.len())]);
        }
        let first_bad = (0..FRAME_SIZE_16K).find(|&i| got[i] != want[i]);
        let first_nz = (0..FRAME_SIZE_16K).find(|&i| want[i] != 0);
        println!("first non-zero {first_nz:?}, first mismatch {first_bad:?}");
        println!("got  {:?}", &got[20..30]);
        println!("want {:?}", &want[20..30]);

        for sf in 0..4 {
            let n = (sf * 80..(sf + 1) * 80).filter(|&i| got[i] == want[i]).count();
            println!("  subframe {sf}: {n}/80 exact");
        }
    }

    #[test]
    fn the_decoder_runs_without_panicking_on_every_mode() {
        // A first, weak claim: the wiring holds together on real payloads.
        // Sample accuracy is asserted separately.
        for mode_index in 0..9 {
            let (_, total, _) = compare(mode_index);
            assert!(total > 0, "mode {mode_index}: decoded nothing");
        }
    }

    /// Where the assembly currently stands against the reference, per mode.
    ///
    /// This is a **ratchet, not a target**: it fails if accuracy regresses, and
    /// the numbers are expected to rise toward 100% as the wiring is corrected.
    /// It exists because "runs without panicking" is far too weak a claim and
    /// the exact test below cannot pass yet.
    #[test]
    fn the_assembly_does_not_regress_against_the_reference() {
        // Floors, well under the measured values, so ordinary noise does not
        // fail the build but a real regression does.
        const FLOOR_PERMILLE: [u64; 9] = [530, 590, 790, 790, 750, 720, 770, 780, 700];

        for (mode_index, &floor) in FLOOR_PERMILLE.iter().enumerate() {
            let (matched, total, worst) = compare(mode_index);
            assert!(total > 0, "mode {mode_index}: decoded nothing");
            let permille = matched as u64 * 1000 / total as u64;
            println!("mode {mode_index}: {matched}/{total} = {permille} per mille, worst {worst}");
            assert!(
                permille >= floor,
                "mode {mode_index}: {matched}/{total} exact ({permille} per mille, \
                 floor {floor}), worst |delta| = {worst}"
            );
        }
    }

    #[test]
    #[ignore = "the assembly is ~1-3% sample-exact; see AMR_IMPLEMENTATION_STATUS.md"]
    fn the_decoder_matches_the_reference_sample_for_sample() {
        for mode_index in 0..9 {
            let (matched, total, worst) = compare(mode_index);
            assert_eq!(
                matched, total,
                "mode {mode_index}: {matched} of {total} samples match, worst error {worst}"
            );
        }
    }
}
