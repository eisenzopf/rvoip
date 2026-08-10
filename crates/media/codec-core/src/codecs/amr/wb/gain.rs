//! Gain dequantisation, 3GPP TS 26.190 §6.3, in fixed point.
//!
//! Turns a subframe's gain index into the pitch gain and the code gain — the
//! two scalars that weigh the adaptive and algebraic contributions to the
//! excitation.
//!
//! # Jointly quantised, and predicted
//!
//! The two gains are coded together because they are strongly correlated: a
//! voiced subframe wants high pitch gain and low code gain, and coding them
//! independently would spend bits on combinations that never occur.
//!
//! The code gain is quantised *relative to a prediction* from the last four
//! subframes' energies, so this module carries state. The codebook holds a
//! correction, not a gain — which is why a decoder with a wrong state update
//! drifts in loudness rather than failing outright.
//!
//! # Normalising by the innovation's own energy
//!
//! The decoded code gain is scaled by `1/sqrt(energy of the pulse vector)`.
//! Without that, a subframe whose algebraic codebook happened to place its
//! pulses constructively would come out louder than one that did not, for the
//! same transmitted index. Normalising means the index describes the intended
//! loudness rather than an artefact of pulse placement.
//!
//! # Erasures
//!
//! A lost subframe reuses a **median** of the last five gains, not a mean: one
//! bad frame in the history must not drag the estimate, and an erasure is
//! exactly what puts an outlier there. The result is then attenuated by a
//! factor that steepens with consecutive losses, so sustained loss fades to
//! silence instead of buzzing.

use super::codebook::L_SUBFR;
use super::gain_tables::{QUA_GAIN_6B, QUA_GAIN_7B};
use super::math::{dot_product12, isqrt_n, log2, median5, pow2};
use crate::fixed_point::arith::{extract_h, extract_l, mult, round, sub};
use crate::fixed_point::arith32::{l_deposit_h, l_mac, l_mult};
use crate::fixed_point::oper32::{l_extract, mpy_32_16};
use crate::fixed_point::shift::{l_shl, l_shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Order of the energy predictor.
const PRED_ORDER: usize = 4;

/// Mean frame energy in dB, the prediction's anchor.
const MEAN_ENER: i16 = 30;

/// How many past gains the concealment median spans.
const L_LTPHIST: usize = 5;

/// MA prediction coefficients `{0.5, 0.4, 0.3, 0.2}` in Q13.
const PRED: [Word16; PRED_ORDER] = [
    Word16(4096),
    Word16(3277),
    Word16(2458),
    Word16(1638),
];

/// Pitch-gain attenuation by consecutive-erasure state, for unusable frames.
const PDOWN_UNUSABLE: [Word16; 7] = [
    Word16(32767),
    Word16(31130),
    Word16(29491),
    Word16(24576),
    Word16(7537),
    Word16(1638),
    Word16(328),
];

/// Code-gain attenuation by erasure state, for unusable frames.
const CDOWN_UNUSABLE: [Word16; 7] = [
    Word16(32767),
    Word16(16384),
    Word16(8192),
    Word16(8192),
    Word16(8192),
    Word16(4915),
    Word16(3277),
];

/// Pitch-gain attenuation by erasure state, for usable frames.
const PDOWN_USABLE: [Word16; 7] = [
    Word16(32767),
    Word16(32113),
    Word16(31457),
    Word16(24576),
    Word16(7537),
    Word16(1638),
    Word16(328),
];

/// Code-gain attenuation by erasure state, for usable frames.
const CDOWN_USABLE: [Word16; 7] = [
    Word16(32767),
    Word16(32113),
    Word16(32113),
    Word16(32113),
    Word16(32113),
    Word16(32113),
    Word16(22938),
];

/// The decoded gains for one subframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gains {
    /// Weight on the adaptive (pitch) contribution, Q14.
    pub pitch: Word16,
    /// Weight on the algebraic (innovation) contribution, Q16.
    pub code: Word32,
}

/// How a frame arrived, which selects the concealment behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameQuality {
    /// Decode from the transmitted index.
    Good,
    /// Damaged but the bits may still be usable for some parameters.
    Bad,
    /// No usable bits at all.
    Unusable,
}

/// Carried gain-prediction and concealment state, one per decoder.
#[derive(Debug, Clone)]
pub struct GainDecoder {
    /// Last four quantised energies, Q10, for the MA prediction.
    past_energy: [Word16; PRED_ORDER],
    /// Last decoded pitch gain, Q14.
    past_pitch: Word16,
    /// Last decoded code gain, Q3.
    past_code: Word16,
    /// The code gain before the most recent frame, for the post-erasure clamp.
    prev_code: Word16,
    /// Recent pitch gains, for the concealment median.
    pitch_history: [Word16; L_LTPHIST],
    /// Recent code gains, for the concealment median.
    code_history: [Word16; L_LTPHIST],
}

impl Default for GainDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// What the gain decoder needs to know about the frame this subframe is in.
///
/// Grouped rather than passed loose because every field is frame-scoped while
/// the call is per-subframe, and passing them separately invites exactly the
/// mistake this struct was created to fix: `previous_frame_bad` was once kept
/// inside the decoder and cleared after the first good *subframe*, where
/// TS 26.173 keeps it for the whole *frame*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameContext {
    /// Whether this frame is usable, damaged, or absent.
    pub quality: FrameQuality,
    /// Consecutive bad frames, saturating at 6; drives how hard concealment
    /// attenuates.
    pub erasure_state: usize,
    /// Recent non-speech frames, from the encoder's VAD flag.
    pub vad_history: i16,
    /// The reference's `prev_bfi`. Set for **all four** subframes of the first
    /// good frame after an erasure, where it caps a sudden jump in code gain —
    /// a jump that is audible as a crack on exactly the frame concealment was
    /// trying to hide.
    pub previous_frame_bad: bool,
}

impl FrameContext {
    /// The context for an ordinary frame of a clean stream.
    #[must_use]
    pub const fn good() -> Self {
        Self {
            quality: FrameQuality::Good,
            erasure_state: 0,
            vad_history: 0,
            previous_frame_bad: false,
        }
    }
}

impl GainDecoder {
    /// A decoder in its reset state.
    ///
    /// The energy predictor starts at -14 dB rather than zero: a zero would
    /// claim the last four subframes were at the mean energy, so the first
    /// frame after a reset would decode far too loud.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            past_energy: [Word16(-14336); PRED_ORDER],
            past_pitch: Word16(0),
            past_code: Word16(0),
            prev_code: Word16(0),
            pitch_history: [Word16(0); L_LTPHIST],
            code_history: [Word16(0); L_LTPHIST],
        }
    }

    /// Decode one subframe's gains.
    ///
    /// `index` is the transmitted gain index and `bits` its width, 6 or 7.
    /// `code` is the innovation vector from [`super::codebook`], needed for the
    /// energy normalisation. `erasure_state` counts consecutive bad frames,
    /// saturating at 6, and drives how hard concealment attenuates.
    /// `vad_history` is the count of recent non-speech frames.
    ///
    /// `previous_frame_bad` is the reference's `prev_bfi`, and it is a
    /// **frame** flag rather than a subframe one: TS 26.173 assigns it once at
    /// the end of `decoder()`, so all four subframes of the first good frame
    /// after an erasure see it set. Tracking it inside this struct and clearing
    /// it after the first good subframe would let subframes 1 to 3 of a
    /// recovery frame skip the cap on a sudden jump in code gain — which is
    /// audible as a crack on exactly the frame concealment was trying to hide.
    #[must_use]
    pub fn decode(
        &mut self,
        index: u16,
        bits: usize,
        code: &[Word16; L_SUBFR],
        frame: FrameContext,
    ) -> Gains {
        let mut ctx = DspContext::default();

        // 1/sqrt(energy per sample) of the innovation, Q12.
        let (energy, exp) = dot_product12(&mut ctx, code, code);
        // -18 because code is Q9, -6 because of the division by L_SUBFR.
        let (inv_norm, exp) = isqrt_n(&mut ctx, (energy, exp - 24));
        let gcode_inov = extract_h(l_shl(&mut ctx, inv_norm, exp - 3));

        if frame.quality == FrameQuality::Good {
            self.decode_good(&mut ctx, index, bits, gcode_inov, frame.previous_frame_bad)
        } else {
            self.conceal(
                &mut ctx,
                frame.quality,
                frame.erasure_state,
                frame.vad_history,
                gcode_inov,
            )
        }
    }

    /// Decode from the transmitted index.
    fn decode_good(
        &mut self,
        ctx: &mut DspContext,
        index: u16,
        bits: usize,
        gcode_inov: Word16,
        previous_frame_bad: bool,
    ) -> Gains {
        // Predict this subframe's energy from the last four, in dB.
        let mut acc = l_shl(ctx, l_deposit_h(Word16(MEAN_ENER)), 8);
        for (coeff, past) in PRED.iter().zip(self.past_energy.iter()) {
            acc = l_mac(ctx, acc, *coeff, *past);
        }
        let gcode0 = extract_h(acc);

        // 10^(gcode0/20) = 2^(0.166096 · gcode0).
        let scaled = l_mult(ctx, gcode0, Word16(5443));
        let acc = l_shr(ctx, scaled, 8);
        let (mut exp_gcode0, frac) = l_extract(acc);
        // Fixing the exponent at 14 keeps the mantissa in 16384..=32767.
        let gcode0 = extract_l(pow2(ctx, 14, frac));
        exp_gcode0 = Word16(exp_gcode0.0 - 14);

        let table: &[i16] = if bits == 6 { &QUA_GAIN_6B } else { &QUA_GAIN_7B };
        let entry = index as usize * 2;
        let pitch = Word16(table[entry]);
        let g_code = Word16(table[entry + 1]);

        // Q11 · Q0 -> Q12, then up to Q16.
        let acc = l_mult(ctx, g_code, gcode0);
        let mut code_gain = l_shl(ctx, acc, exp_gcode0.0 + 4);

        // After an erasure the predictor is unreliable, so cap any sudden jump
        // in loudness. Without this a recovered frame can crack audibly.
        if previous_frame_bad {
            let ceiling = l_mult(ctx, self.prev_code, Word16(5120));
            if code_gain.0 > ceiling.0 && code_gain.0 > 6_553_600 {
                code_gain = ceiling;
            }
        }

        let raised = l_shl(ctx, code_gain, 3);
        self.past_code = round(ctx, raised);
        self.past_pitch = pitch;
        self.prev_code = self.past_code;
        self.push_history(self.past_pitch, self.past_code);

        // Scale by the innovation's own energy, so the index describes the
        // intended loudness rather than the pulse placement.
        let (hi, lo) = l_extract(code_gain);
        let code = l_shl(ctx, mpy_32_16(hi, lo, gcode_inov), 3);

        // Record this subframe's energy in dB for the next prediction:
        // 20·log10(g) = 6.0206·(log2(g in Q11) - 11).
        let (exp, frac) = log2(ctx, Word32(i32::from(g_code.0)));
        let scaled = mpy_32_16(Word16(exp - 11), frac, Word16(24660));
        let energy = extract_l(l_shr(ctx, scaled, 3));
        self.push_energy(energy);

        Gains { pitch, code }
    }

    /// Reconstruct gains for an erased subframe from history.
    fn conceal(
        &mut self,
        ctx: &mut DspContext,
        quality: FrameQuality,
        erasure_state: usize,
        vad_history: i16,
        gcode_inov: Word16,
    ) -> Gains {
        let state = erasure_state.min(6);
        let unusable = quality == FrameQuality::Unusable;

        // A median, not a mean: an erasure is exactly what puts an outlier in
        // the history, and one must not drag the estimate.
        let mut pitch = median5(&self.pitch_history);
        // Cap at 0.95 — a concealed pitch gain at or above 1.0 makes the
        // long-term predictor self-oscillate and ring.
        if pitch.0 > 15565 {
            pitch = Word16(15565);
        }
        self.past_pitch = pitch;

        let attenuation = if unusable {
            PDOWN_UNUSABLE[state]
        } else {
            PDOWN_USABLE[state]
        };
        let pitch_out = mult(ctx, attenuation, self.past_pitch);

        let median_code = median5(&self.code_history);
        // During a long silence the gain is already low, so attenuating again
        // would fade to nothing; hold it instead.
        self.past_code = if vad_history > 2 {
            median_code
        } else if unusable {
            mult(ctx, CDOWN_UNUSABLE[state], median_code)
        } else {
            mult(ctx, CDOWN_USABLE[state], median_code)
        };

        // Decay the energy prediction: average the last four and drop 3 dB,
        // floored at -14, so sustained loss fades rather than buzzing.
        let mut acc = Word32(0);
        for past in &self.past_energy {
            acc = l_mac(ctx, acc, *past, Word16(8192));
        }
        let mut energy = extract_h(acc);
        energy = sub(ctx, energy, Word16(3072));
        if energy.0 < -14336 {
            energy = Word16(-14336);
        }
        self.push_energy(energy);
        self.push_history(self.past_pitch, self.past_code);

        // past_code is Q3, gcode_inov Q12, so the product is Q16.
        let code = l_mult(ctx, self.past_code, gcode_inov);

        Gains {
            pitch: pitch_out,
            code,
        }
    }

    /// Shift a new energy into the predictor's window.
    fn push_energy(&mut self, energy: Word16) {
        self.past_energy.rotate_right(1);
        self.past_energy[0] = energy;
    }

    /// Shift new gains into the concealment history.
    fn push_history(&mut self, pitch: Word16, code: Word16) {
        self.pitch_history.rotate_left(1);
        self.pitch_history[L_LTPHIST - 1] = pitch;
        self.code_history.rotate_left(1);
        self.code_history[L_LTPHIST - 1] = code;
    }
}

#[cfg(test)]
mod tests {
    use super::super::lp::isp_to_lp::tests_support::{block_has, block_row_i32, has_block};
    use super::super::params::FrameParams;
    use super::super::codebook;
    use super::*;
    use crate::codecs::amr::mode::{AmrMode, AmrVariant};
    use crate::codecs::amr::storage;

    fn mode_for(index: usize) -> AmrMode {
        AmrMode::new(AmrVariant::WideBand, u8::try_from(index).expect("index")).expect("mode")
    }

    fn fixture(mode_index: usize) -> &'static [u8] {
        const FILES: [&[u8]; 9] = [
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
        FILES[mode_index]
    }

    const FRAME_BITS: [usize; 9] = [132, 177, 253, 285, 317, 365, 397, 461, 477];

    #[test]
    fn gains_are_bit_exact_against_ts26173() {
        let mut checked = 0;

        for (mode_index, &frame_bits) in FRAME_BITS.iter().enumerate() {
            let block = format!("bitstream{mode_index}");
            assert!(has_block(&block), "fixture block {block} missing");
            let (_, frames) = storage::read(fixture(mode_index)).expect("fixture parses");
            let mode = mode_for(mode_index);

            // The predictor runs across subframes and frames, so replay from
            // reset rather than decoding any subframe in isolation.
            let mut dec = GainDecoder::new();
            let gain_bits = if frame_bits <= 177 { 6 } else { 7 };

            for f in 0.. {
                if !block_has(&block, &format!("meta{f}")) {
                    break;
                }
                let frame = frames.get(f).expect("frame");
                let params = FrameParams::parse(mode, &frame.data).expect("parses");

                for (sf, sub) in params.subframes.iter().enumerate() {
                    let code = codebook::decode(&sub.pulses, frame_bits).expect("code");
                    let code: [Word16; L_SUBFR] = code.map(Word16);
                    let got = dec.decode(sub.gain_index, gain_bits, &code, FrameContext::good());

                    let want = block_row_i32(&block, &format!("gain{f}_{sf}"));
                    assert_eq!(
                        i32::from(got.pitch.0),
                        want[0],
                        "{block} frame {f} subframe {sf}: pitch gain"
                    );
                    assert_eq!(
                        got.code.0, want[1],
                        "{block} frame {f} subframe {sf}: code gain"
                    );
                    checked += 1;
                }
            }
        }

        assert!(checked >= 72, "only {checked} subframes checked");
    }

    #[test]
    fn a_reset_decoder_does_not_start_loud() {
        // The predictor initialises to -14 dB, not 0. A zero would claim the
        // last four subframes sat at the mean energy and the first frame would
        // decode far too loud.
        let quiet = GainDecoder::new();
        assert!(
            quiet.past_energy.iter().all(|e| e.0 == -14336),
            "predictor did not start at -14 dB"
        );
    }

    #[test]
    fn sustained_erasure_fades_toward_silence() {
        // The attenuation tables steepen with consecutive losses, so a long
        // gap must decay rather than hold a buzz.
        let code = [Word16(512); L_SUBFR];
        let mut dec = GainDecoder::new();
        for _ in 0..8 {
            let _ = dec.decode(20, 7, &code, FrameContext::good());
        }

        let mut previous = i64::MAX;
        for state in 1..7 {
            let g = dec.decode(0, 7, &code, FrameContext { quality: FrameQuality::Unusable, erasure_state: state, ..FrameContext::good() });
            let level = i64::from(g.pitch.0);
            assert!(
                level <= previous,
                "erasure state {state} gave pitch gain {level}, above the previous {previous}"
            );
            previous = level;
        }
        assert!(previous < 1000, "sustained erasure held at {previous}");
    }

    #[test]
    fn a_concealed_pitch_gain_cannot_self_oscillate() {
        // A pitch gain at or above 1.0 (16384 in Q14) makes the long-term
        // predictor ring. Concealment caps at 0.95 for exactly that reason.
        let code = [Word16(512); L_SUBFR];
        let mut dec = GainDecoder::new();
        // Drive the history high, then erase.
        for _ in 0..8 {
            let _ = dec.decode(63, 7, &code, FrameContext::good());
        }
        for state in 0..7 {
            let g = dec.decode(0, 7, &code, FrameContext { quality: FrameQuality::Unusable, erasure_state: state, ..FrameContext::good() });
            assert!(
                g.pitch.0 < 16384,
                "erasure state {state}: pitch gain {} would self-oscillate",
                g.pitch.0
            );
        }
    }

    #[test]
    fn the_energy_normalisation_offsets_pulse_placement() {
        // Two innovation vectors with the same index but different energies
        // should not produce the same code gain -- that is the whole point of
        // dividing by the vector's own norm.
        let sparse = {
            let mut c = [Word16(0); L_SUBFR];
            c[0] = Word16(512);
            c
        };
        let dense = [Word16(512); L_SUBFR];

        let mut a = GainDecoder::new();
        let mut b = GainDecoder::new();
        let g_sparse = a.decode(30, 7, &sparse, FrameContext::good());
        let g_dense = b.decode(30, 7, &dense, FrameContext::good());

        assert!(
            g_sparse.code.0 > g_dense.code.0,
            "the sparser vector should get the larger gain: {} against {}",
            g_sparse.code.0,
            g_dense.code.0
        );
        // Both come from the same index, so the pitch gain is untouched.
        assert_eq!(g_sparse.pitch, g_dense.pitch);
    }
}
