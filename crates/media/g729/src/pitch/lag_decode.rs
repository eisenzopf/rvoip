//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use crate::dsp::arith::{add, mult, sub};
use crate::dsp::types::{DspContext, Word16};

/// Public function `dec_lag3`.
pub fn dec_lag3(
    index: Word16,
    pit_min: Word16,
    pit_max: Word16,
    i_subfr: Word16,
    t0_prev: Word16,
) -> (Word16, Word16, Word16) {
    let mut ctx = DspContext::default();

    if i_subfr.0 == 0 {
        if sub(&mut ctx, index, Word16(197)).0 < 0 {
            let idx_plus2 = add(&mut ctx, index, Word16(2));
            let mul = mult(&mut ctx, idx_plus2, Word16(10923));
            let t0 = add(&mut ctx, mul, Word16(19));

            let two_t0 = add(&mut ctx, t0, t0);
            let i = add(&mut ctx, two_t0, t0);
            let diff = sub(&mut ctx, index, i);
            let t0_frac = add(&mut ctx, diff, Word16(58));
            (t0, t0_frac, t0)
        } else {
            let t0 = sub(&mut ctx, index, Word16(112));
            (t0, Word16(0), t0)
        }
    } else {
        let mut t0_min = sub(&mut ctx, t0_prev, Word16(5));
        if sub(&mut ctx, t0_min, pit_min).0 < 0 {
            t0_min = pit_min;
        }

        let mut t0_max = add(&mut ctx, t0_min, Word16(9));
        if sub(&mut ctx, t0_max, pit_max).0 > 0 {
            t0_max = pit_max;
            t0_min = sub(&mut ctx, t0_max, Word16(9));
        }

        let idx_plus2 = add(&mut ctx, index, Word16(2));
        let mul = mult(&mut ctx, idx_plus2, Word16(10923));
        let i = sub(&mut ctx, mul, Word16(1));
        let t0 = add(&mut ctx, i, t0_min);

        let two_i = add(&mut ctx, i, i);
        let i3 = add(&mut ctx, two_i, i);
        let index_minus2 = sub(&mut ctx, index, Word16(2));
        let t0_frac = sub(&mut ctx, index_minus2, i3);
        (t0, t0_frac, t0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dec_lag3_first_subframe_basic() {
        let (t0, frac, old_t0) =
            dec_lag3(Word16(60), Word16(20), Word16(143), Word16(0), Word16(60));
        assert!(t0.0 >= 20 && t0.0 <= 143);
        assert!(frac.0 >= -1 && frac.0 <= 58);
        assert_eq!(old_t0, t0);
    }

    #[test]
    fn dec_lag3_second_subframe_basic() {
        let (t0, frac, old_t0) =
            dec_lag3(Word16(12), Word16(20), Word16(143), Word16(40), Word16(80));
        assert!(t0.0 >= 20 && t0.0 <= 143);
        assert!(frac.0 >= -2 && frac.0 <= 2);
        assert_eq!(old_t0, t0);
    }
}
