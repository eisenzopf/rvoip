//! Pitch lag encoding.
//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use crate::dsp::arith::{add, sub};
use crate::dsp::types::{DspContext, Word16};

#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_lag3(
    t0: i16,
    t0_frac: i16,
    t0_min: &mut i16,
    t0_max: &mut i16,
    pit_min: i16,
    pit_max: i16,
    pit_flag: i16,
) -> i16 {
    let mut ctx = DspContext::default();
    if pit_flag == 0 {
        let index = if sub(&mut ctx, Word16(t0), Word16(85)).0 <= 0 {
            let tt = add(&mut ctx, Word16(t0), Word16(t0)).0;
            let i = add(&mut ctx, Word16(tt), Word16(t0)).0;
            let im58 = sub(&mut ctx, Word16(i), Word16(58));
            add(&mut ctx, im58, Word16(t0_frac)).0
        } else {
            add(&mut ctx, Word16(t0), Word16(112)).0
        };

        *t0_min = sub(&mut ctx, Word16(t0), Word16(5)).0;
        if sub(&mut ctx, Word16(*t0_min), Word16(pit_min)).0 < 0 {
            *t0_min = pit_min;
        }
        *t0_max = add(&mut ctx, Word16(*t0_min), Word16(9)).0;
        if sub(&mut ctx, Word16(*t0_max), Word16(pit_max)).0 > 0 {
            *t0_max = pit_max;
            *t0_min = sub(&mut ctx, Word16(*t0_max), Word16(9)).0;
        }
        index
    } else {
        let i = sub(&mut ctx, Word16(t0), Word16(*t0_min)).0;
        let ii2 = add(&mut ctx, Word16(i), Word16(i)).0;
        let i3 = add(&mut ctx, Word16(ii2), Word16(i)).0;
        let i32 = add(&mut ctx, Word16(i3), Word16(2));
        add(&mut ctx, i32, Word16(t0_frac)).0
    }
}
