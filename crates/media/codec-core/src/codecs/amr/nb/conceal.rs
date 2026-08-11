//! Concealing a narrowband frame that never arrived, TS 26.073 `b_cn_cod.c`.
//!
//! # A lost frame is not a damaged frame
//!
//! Wideband draws the distinction in its own frame type — RFC 4867 gives AMR-WB
//! `SPEECH_LOST`, and the decoder branches on it. Narrowband has no such type:
//! RFC 4867 §4.3.1 gives it FT 15, `NO_DATA`, and the reference maps that to
//! `RX_NO_DATA`. The difference from `RX_SPEECH_BAD` is one step, taken before
//! anything else:
//!
//! > `build_CN_param(&st->nodataSeed, prmno[mode], bitno[mode], parm)`
//! > — `dec_amr.c`, on `RX_NO_DATA` and `RX_ONSET` only
//!
//! The parameter vector is *manufactured* from a deterministic sequence and
//! then decoded as if it had arrived, with the bad-frame flag set. So the LSFs
//! and gains are concealed exactly as for a damaged frame — those paths ignore
//! the decoded indices — but the innovation is not: `decode_codebook` runs
//! unconditionally, and the synthesised pulses *are* the excitation. That is
//! why lost and damaged decode differently, and why collapsing them into one
//! path produces plausible audio that matches no reference.
//!
//! # Two generators, one of which is not this one
//!
//! `b_cn_cod.c` holds two unrelated pseudo-random sources, and using the wrong
//! one is an easy mistake with no visible symptom until a fixture comparison:
//!
//! - `pseudonoise()` is a 31-bit LFSR over a `Word32`, seeded `0x70816958`. It
//!   feeds `build_CN_code`, the *comfort-noise excitation*, which is DTX work
//!   and is not implemented here.
//! - `build_CN_param()` uses `nodataSeed`, a plain 16-bit LCG seeded 21845.
//!   That is this module.
//!
//! The LCG is the same recurrence the wideband noise generator already
//! implements (`wb::highband::NoiseGenerator`), from the same initial seed —
//! but it is a separate register advancing on a separate schedule, so the two
//! are deliberately not shared.

use super::bitstream::parameter_widths;
use super::decoder_tables::LP_WINDOW_200_40;
use crate::fixed_point::arith::extract_l;
use crate::fixed_point::arith32::{l_add, l_mult};
use crate::fixed_point::shift::l_shr;
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// The only value `nodataSeed` is ever initialised to — `dec_amr.c`.
///
/// Set once, at decoder reset, and thereafter advanced *only* by a lost frame.
/// A good or damaged frame leaves it untouched, so the sequence a stream sees
/// depends on how many frames it has lost and on nothing else.
pub const NODATA_SEED_INIT: Word16 = Word16(21845);

/// One step of the `nodataSeed` recurrence: `seed = seed * 31821 + 13849`.
///
/// Written through the saturating primitives rather than as `i32` arithmetic
/// because that is what the reference does. It cannot actually saturate —
/// `|2 * 32768 * 31821|` is under `2^31` — but the point of this crate is that
/// the reference's operator choice is the specification, not an implementation
/// detail to be optimised past.
pub fn advance_seed(ctx: &mut DspContext, seed: Word16) -> Word16 {
    // `l_mult` is `2 * a * b`; the `l_shr` by one undoes the doubling exactly.
    let product = l_mult(ctx, seed, Word16(31821));
    let halved = l_shr(ctx, product, 1);
    extract_l(l_add(ctx, halved, Word32(13849)))
}

/// Manufacture the parameter vector for a lost frame — `build_CN_param`.
///
/// The seed advances **once per frame**, not once per parameter: its low seven
/// bits pick a base offset into the LP analysis window, and the parameters are
/// then consecutive window entries, each masked to its own bit width. Stepping
/// the generator per parameter instead would produce a vector that is equally
/// noise-like and never matches the reference.
///
/// `mode_index` is the mode the stream was last using. A lost frame carries no
/// mode of its own, so the reference substitutes `prev_mode`; it selects both
/// how many parameters are produced and how wide each one is.
///
/// # Panics
/// If `mode_index` is not a speech mode, 0..=7. SID and reserved types have no
/// parameter layout to synthesise.
#[must_use]
pub fn build_cn_param(ctx: &mut DspContext, seed: &mut Word16, mode_index: u8) -> Vec<u16> {
    assert!(
        mode_index < 8,
        "AMR-NB concealment needs a speech mode, got {mode_index}"
    );
    *seed = advance_seed(ctx, *seed);

    // The low seven bits, so 0..=127 even when the seed is negative. The
    // reference writes `*seed & 0x7F` on a signed `Word16` and relies on the
    // mask, not on the sign.
    #[allow(clippy::cast_sign_loss)]
    let base = usize::from(seed.0 as u16 & 0x7F);
    let widths = parameter_widths(mode_index);
    // 127 + 57 - 1 = 183 < 240: the widest mode starting from the highest base
    // still ends inside the window, so the walk can never leave it.
    debug_assert!(
        base + widths.len() <= LP_WINDOW_200_40.len(),
        "window walk left the table: base {base} + {} parameters",
        widths.len()
    );

    widths
        .iter()
        .enumerate()
        .map(|(index, &width)| {
            // `*p++ & ~(0xFFFF << w)` in the reference. Every window entry is
            // positive, so this is a plain low-`w`-bit mask with no sign
            // extension to reason about.
            #[allow(clippy::cast_sign_loss)]
            let entry = LP_WINDOW_200_40[base + index] as u16;
            entry & ((1u16 << width) - 1)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_advances_once_per_frame_not_once_per_parameter() {
        // If the generator were stepped per parameter the second and third
        // values would be unrelated to the first; they are consecutive window
        // entries instead. This is the distinction the module header calls out,
        // asserted rather than only described.
        let mut ctx = DspContext::default();
        let mut seed = NODATA_SEED_INIT;
        let params = build_cn_param(&mut ctx, &mut seed, 4);

        #[allow(clippy::cast_sign_loss)]
        let base = usize::from(seed.0 as u16 & 0x7F);
        let widths = parameter_widths(4);
        assert_eq!(params.len(), widths.len());
        for (index, (&value, &width)) in params.iter().zip(widths).enumerate() {
            #[allow(clippy::cast_sign_loss)]
            let expected = (LP_WINDOW_200_40[base + index] as u16) & ((1u16 << width) - 1);
            assert_eq!(value, expected, "parameter {index}");
        }
    }

    #[test]
    fn every_parameter_fits_the_width_it_was_masked_to() {
        // A value wider than its field would be silently truncated by the
        // decoder's own table lookups, or index past a codebook.
        let mut ctx = DspContext::default();
        let mut compared = 0;
        for mode_index in 0..8u8 {
            let mut seed = NODATA_SEED_INIT;
            for _ in 0..8 {
                let params = build_cn_param(&mut ctx, &mut seed, mode_index);
                for (&value, &width) in params.iter().zip(parameter_widths(mode_index)) {
                    assert!(
                        u32::from(value) < (1u32 << width),
                        "mode {mode_index}: {value} does not fit {width} bits"
                    );
                    compared += 1;
                }
            }
        }
        // 8 modes x 8 draws x (17+19+19+19+19+23+39+57)/8 parameters.
        assert_eq!(compared, 8 * (17 + 19 + 19 + 19 + 19 + 23 + 39 + 57));
    }

    #[test]
    fn successive_frames_draw_different_vectors() {
        // The seed carries across frames. If it were re-seeded per loss, every
        // lost frame would synthesise the same excitation — audible as a tone
        // rather than as noise, and wrong against the reference from the second
        // loss onward.
        let mut ctx = DspContext::default();
        let mut seed = NODATA_SEED_INIT;
        let first = build_cn_param(&mut ctx, &mut seed, 4);
        let second = build_cn_param(&mut ctx, &mut seed, 4);
        assert_ne!(first, second);

        let mut fresh = NODATA_SEED_INIT;
        let restarted = build_cn_param(&mut ctx, &mut fresh, 4);
        assert_eq!(first, restarted, "the sequence must be deterministic");
    }

    #[test]
    #[should_panic(expected = "needs a speech mode")]
    fn sid_has_no_parameter_layout_to_synthesise() {
        let mut ctx = DspContext::default();
        let mut seed = NODATA_SEED_INIT;
        let _ = build_cn_param(&mut ctx, &mut seed, 8);
    }
}
