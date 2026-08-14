//! The narrowband comfort-noise excitation, TS 26.073 `b_cn_cod.c`.
//!
//! The other half of that file — `build_CN_param`, which manufactures a
//! parameter vector for a frame that never arrived — lives in
//! [`super::conceal`]. The two are kept apart because they are driven by
//! *different generators on different schedules*, and the file header there
//! explains why confusing them is an easy mistake with a late symptom.
//!
//! This module is the DTX one. [`pseudonoise`] is a 31-bit linear-feedback
//! shift register held in a `Word32`, seeded [`PN_INITIAL_SEED`]; it is drawn
//! by two callers in the decode path, the LSF variability index in `dtx_dec`
//! and the pulse positions and signs here, and they share one register. So the
//! sequence any one of them sees depends on how many times the *other* has been
//! drawn — which is why both draws have to happen in the reference's order,
//! and why neither can be given a private generator "for clarity".

use crate::fixed_point::arith::{add, extract_l};
use crate::fixed_point::arith32::l_mult;
use crate::fixed_point::shift::{shl, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// The subframe length the codebook vector is built over.
const L_SUBFR: usize = 40;

/// How many pulses `build_CN_code` places. `NB_PULSE` in the reference.
const NB_PULSE: usize = 10;

/// `dtx_dec.c`'s `PN_INITIAL_SEED`, the register's value after a reset.
pub const PN_INITIAL_SEED: Word32 = Word32(0x7081_6958);

/// The pulse amplitude, Q12: `4096` is 1.0.
const PULSE: Word16 = Word16(4096);

/// `pseudonoise`: draw `no_bits` bits from the shift register.
///
/// The register is a 31-bit LFSR tapping states 31 and 3. Two details are worth
/// being explicit about, because both are easy to "correct" into something that
/// still looks like a noise source:
///
/// The bit *returned* is the register's low bit **before** the shift, while the
/// bit *fed back* is the exclusive-or of the low bit and bit 28. They are not
/// the same bit, and taking the feedback bit as the output changes every
/// sequence this generates.
///
/// And the feedback is inserted at bit 30 (`0x40000000`), not at bit 31. That
/// is what makes the register 31 bits wide rather than 32.
///
/// # Panics
/// Never: the masked value is one bit. The conversion is written fallibly
/// because the register is a `Word32` and the accumulator a `Word16`.
pub fn pseudonoise(reg: &mut Word32, no_bits: usize) -> Word16 {
    let mut noise_bits = 0i16;
    for _ in 0..no_bits {
        // State n == 31, then state n == 3, exclusive-ored together.
        let sn = ((reg.0 & 0x0000_0001) != 0) ^ ((reg.0 & 0x1000_0000) != 0);

        noise_bits = (noise_bits << 1) | i16::try_from(reg.0 & 1).expect("one bit");

        // `L_shr` on a non-negative register is a plain logical shift; the
        // register never goes negative because the feedback sets bit 30, not
        // bit 31.
        reg.0 >>= 1;
        if sn {
            reg.0 |= 0x4000_0000;
        }
    }
    Word16(noise_bits)
}

/// `build_CN_code`: ten signed unit pulses at pseudo-random positions.
///
/// Each pulse takes a two-bit position draw and a one-bit sign draw, in that
/// order. The position is `(2 * draw * 10) / 2 + k` — track `k` of a ten-track
/// interleave, so pulse `k` can only land on positions congruent to `k` mod 10
/// and the ten pulses can never collide.
///
/// The reference computes that as `shr(extract_l(L_mult(i, 10)), 1)`, and it is
/// transcribed rather than simplified to `i * 10`: `L_mult` doubles, and the
/// `shr` undoes the doubling, so the two agree — but only because the draw is
/// two bits wide. This module keeps the reference's shape so that stays true by
/// construction rather than by an argument no one rechecks.
///
/// # Panics
/// Never: the position is bounded by the two-bit draw and the track index.
pub fn build_cn_code(ctx: &mut DspContext, reg: &mut Word32) -> [Word16; L_SUBFR] {
    let mut cod = [Word16(0); L_SUBFR];

    for k in 0..NB_PULSE {
        let draw = pseudonoise(reg, 2);
        let tenfold = extract_l(l_mult(ctx, draw, Word16(10)));
        let scaled = shr(ctx, tenfold, 1);
        let i = add(ctx, scaled, Word16(i16::try_from(k).expect("k < 10")));

        let sign = pseudonoise(reg, 1);

        let position = usize::try_from(i.0).expect("a two-bit draw times ten, plus k, is 0..=39");
        cod[position] = if sign.0 > 0 { PULSE } else { Word16(-PULSE.0) };
    }

    cod
}

/// `A_Refl`: direct-form LP coefficients to reflection coefficients, Q15.
///
/// A backward Levinson recursion. `a` is `acoeff[1..=M]` — the reference passes
/// `&acoeff[1]`, so the leading 1.0 is *not* included, and passing the whole
/// vector shifts every coefficient by one and yields a plausible-looking but
/// wrong prediction gain.
///
/// Two guards abort the whole recursion and return all zeros: a coefficient at
/// or above 4096 (1.0 in Q12, so an unstable filter), and an intermediate that
/// will not fit a `Word16`. The reference implements both with a `goto` out of
/// the nested loop, and the zeros it leaves are meaningful — they say "no
/// prediction gain", which the caller then reads as a gain of exactly 1.
///
/// # Panics
/// If `a` is not exactly `M` coefficients.
#[must_use]
pub fn a_refl(ctx: &mut DspContext, a: &[Word16]) -> [Word16; 10] {
    use crate::fixed_point::arith::{abs_s, round, sub};
    use crate::fixed_point::arith32::{l_deposit_h, l_msu, l_sub};
    use crate::fixed_point::div::div_s;
    use crate::fixed_point::shift::{l_shl, l_shr_r, norm_l};

    const M: usize = 10;
    assert_eq!(
        a.len(),
        M,
        "A_Refl takes the ten coefficients after the leading 1.0"
    );

    let mut refl = [Word16(0); M];
    let mut state = [Word16(0); M];
    state.copy_from_slice(a);
    let mut next = [Word16(0); M];

    for i in (0..M).rev() {
        let magnitude = abs_s(ctx, state[i]);
        if sub(ctx, magnitude, Word16(4096)).0 >= 0 {
            return [Word16(0); M];
        }

        refl[i] = shl(ctx, state[i], 3);

        let l_temp = l_mult(ctx, refl[i], refl[i]);
        let mut l_acc = l_sub(ctx, Word32(i32::MAX), l_temp);

        let norm_shift = norm_l(l_acc);
        let scale = sub(ctx, Word16(15), Word16(norm_shift));

        l_acc = l_shl(ctx, l_acc, norm_shift);
        let norm_prod = round(ctx, l_acc);

        let mult_factor = div_s(Word16(16384), norm_prod);

        for j in 0..i {
            let mut acc = l_deposit_h(state[j]);
            acc = l_msu(ctx, acc, refl[i], state[i - j - 1]);

            let temp = round(ctx, acc);
            let mut l_temp = l_mult(ctx, mult_factor, temp);
            l_temp = l_shr_r(ctx, l_temp, scale.0);

            if l_temp.0.unsigned_abs() > 32767 {
                return [Word16(0); M];
            }

            next[j] = extract_l(l_temp);
        }

        state[..i].copy_from_slice(&next[..i]);
    }

    refl
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generator's period and balance, and that it is not a constant.
    ///
    /// A 31-bit maximal LFSR visits every non-zero state once, so a million
    /// single-bit draws must be very close to balanced and must never repeat
    /// the full register inside that span. A generator that has fallen into a
    /// short cycle — the usual symptom of feeding back the wrong bit or into
    /// the wrong position — fails both halves.
    #[test]
    fn the_shift_register_is_balanced_and_does_not_cycle_short() {
        let mut reg = PN_INITIAL_SEED;
        let mut ones = 0u32;
        let mut seen_initial_again = false;
        for _ in 0..1_000_000 {
            ones += u32::from(pseudonoise(&mut reg, 1).0 != 0);
            if reg == PN_INITIAL_SEED {
                seen_initial_again = true;
            }
        }
        assert!(
            !seen_initial_again,
            "the register returned to its seed inside a million draws"
        );
        assert!(
            (490_000..=510_000).contains(&ones),
            "{ones} ones in a million draws is not a balanced sequence"
        );
    }

    /// Multi-bit draws are the same sequence read in groups.
    ///
    /// `pseudonoise(reg, 3)` must equal three consecutive single-bit draws
    /// packed most-significant first. If it did not, the LSF variability index
    /// and the pulse positions would disagree about how far the register has
    /// advanced.
    #[test]
    fn a_wide_draw_is_narrow_draws_packed_msb_first() {
        let mut wide = PN_INITIAL_SEED;
        let mut narrow = PN_INITIAL_SEED;
        for _ in 0..64 {
            let w = pseudonoise(&mut wide, 3).0;
            let a = pseudonoise(&mut narrow, 1).0;
            let b = pseudonoise(&mut narrow, 1).0;
            let c = pseudonoise(&mut narrow, 1).0;
            assert_eq!(w, (a << 2) | (b << 1) | c);
        }
        assert_eq!(wide, narrow);
    }

    /// Ten pulses, one per track, never colliding, and both signs occurring.
    #[test]
    fn the_codebook_vector_holds_ten_pulses_on_ten_tracks() {
        let mut ctx = DspContext::default();
        let mut reg = PN_INITIAL_SEED;
        let (mut positives, mut negatives) = (0usize, 0usize);

        for _ in 0..200 {
            let cod = build_cn_code(&mut ctx, &mut reg);
            let placed: Vec<usize> = (0..L_SUBFR).filter(|&i| cod[i].0 != 0).collect();
            assert_eq!(placed.len(), NB_PULSE, "pulses collided");
            // One pulse per track: the ten residues mod ten are all distinct,
            // which is what makes a collision impossible rather than merely
            // unobserved. Sorted position order is *not* track order -- pulse
            // 3 at position 3 precedes pulse 1 at position 11 -- so this is a
            // statement about the set, not about the sequence.
            let mut tracks: Vec<usize> = placed.iter().map(|&p| p % NB_PULSE).collect();
            tracks.sort_unstable();
            assert_eq!(
                tracks,
                (0..NB_PULSE).collect::<Vec<_>>(),
                "two pulses shared a track"
            );
            positives += placed.iter().filter(|&&p| cod[p].0 > 0).count();
            negatives += placed.iter().filter(|&&p| cod[p].0 < 0).count();
        }

        assert_eq!(positives + negatives, 2000);
        assert!(
            positives > 800 && negatives > 800,
            "signs are not balanced: {positives}/{negatives}"
        );
    }

    /// Reflection coefficients of a known filter, checked against the
    /// definition rather than against this implementation.
    ///
    /// For a first-order predictor `1 - a·z⁻¹` the only reflection coefficient
    /// is `a` itself, so a filter whose coefficients are zero except the first
    /// has a known answer. Q12 in, Q15 out — hence the factor of eight.
    #[test]
    fn a_first_order_filters_reflection_coefficient_is_its_own_coefficient() {
        let mut ctx = DspContext::default();
        let mut a = [Word16(0); 10];
        a[0] = Word16(1000); // 0.244 in Q12
        let refl = a_refl(&mut ctx, &a);
        assert_eq!(refl[0], Word16(8000), "0.244 in Q12 is 8000 in Q15");
        assert!(refl[1..].iter().all(|c| c.0 == 0));
    }

    /// An unstable filter aborts to all zeros rather than returning garbage.
    #[test]
    fn an_out_of_range_coefficient_aborts_the_recursion() {
        let mut ctx = DspContext::default();
        let mut a = [Word16(0); 10];
        a[9] = Word16(4096); // exactly 1.0 in Q12 -- the guard is `>=`.
        assert_eq!(a_refl(&mut ctx, &a), [Word16(0); 10]);

        a[9] = Word16(4095);
        assert_ne!(a_refl(&mut ctx, &a), [Word16(0); 10], "4095 must not abort");
    }
}
