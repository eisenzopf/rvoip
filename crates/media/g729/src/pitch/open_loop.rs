//! Open-loop pitch search.
//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use super::open_loop_search::{
    best_lag_in_range, correlation_for_lag, normalized_correlation_score, scale_open_loop_signal,
};
use crate::codec::state::OLD_WSP_LEN;
use crate::constants::{L_FRAME, PIT_MAX};
use crate::dsp::arith::{abs_s, add, mult, sub};
use crate::dsp::arith32::l_sub;
use crate::dsp::shift::{shl, shr};
use crate::dsp::types::{DspContext, Word16};

pub(crate) fn search_open_loop(
    old_wsp: &[i16; OLD_WSP_LEN],
    old_wsp_offset: usize,
    pit_max: i16,
    l_frame: usize,
) -> i16 {
    let mut ctx = DspContext::default();
    let mut scaled_signal = [0i16; L_FRAME + PIT_MAX as usize];
    let scal_base = pit_max as usize;
    scale_open_loop_signal(
        &mut ctx,
        old_wsp,
        old_wsp_offset,
        pit_max,
        l_frame,
        &mut scaled_signal,
        scal_base,
    );

    let (max, mut t1) = best_lag_in_range(&mut ctx, &scaled_signal, scal_base, l_frame, 20, 40, 1);
    let mut max1 =
        normalized_correlation_score(&mut ctx, &scaled_signal, scal_base, l_frame, max, t1);

    let (max_2, t2) = best_lag_in_range(&mut ctx, &scaled_signal, scal_base, l_frame, 40, 80, 1);
    let mut max2 =
        normalized_correlation_score(&mut ctx, &scaled_signal, scal_base, l_frame, max_2, t2);

    let (mut max_3, mut t3) =
        best_lag_in_range(&mut ctx, &scaled_signal, scal_base, l_frame, 80, 143, 2);
    let i3 = t3;
    let corr_p1 = correlation_for_lag(&mut ctx, &scaled_signal, scal_base, l_frame, i3 + 1);
    if l_sub(&mut ctx, corr_p1, max_3).0 > 0 {
        max_3 = corr_p1;
        t3 = i3 + 1;
    }
    let corr_m1 = correlation_for_lag(&mut ctx, &scaled_signal, scal_base, l_frame, i3 - 1);
    if l_sub(&mut ctx, corr_m1, max_3).0 > 0 {
        max_3 = corr_m1;
        t3 = i3 - 1;
    }
    let max3 =
        normalized_correlation_score(&mut ctx, &scaled_signal, scal_base, l_frame, max_3, t3);

    let t2x2 = shl(&mut ctx, Word16(t2), 1);
    let mut ii = sub(&mut ctx, t2x2, Word16(t3)).0;
    let abs_ii = abs_s(&mut ctx, Word16(ii));
    let mut jj = sub(&mut ctx, abs_ii, Word16(5)).0;
    if jj < 0 {
        let addv = shr(&mut ctx, Word16(max3), 2);
        max2 = add(&mut ctx, Word16(max2), addv).0;
    }
    ii = add(&mut ctx, Word16(ii), Word16(t2)).0;
    let abs_ii = abs_s(&mut ctx, Word16(ii));
    jj = sub(&mut ctx, abs_ii, Word16(7)).0;
    if jj < 0 {
        let addv = shr(&mut ctx, Word16(max3), 2);
        max2 = add(&mut ctx, Word16(max2), addv).0;
    }
    let t1x2 = shl(&mut ctx, Word16(t1), 1);
    ii = sub(&mut ctx, t1x2, Word16(t2)).0;
    let abs_ii = abs_s(&mut ctx, Word16(ii));
    jj = sub(&mut ctx, abs_ii, Word16(5)).0;
    if jj < 0 {
        let addv = mult(&mut ctx, Word16(max2), Word16(6554));
        max1 = add(&mut ctx, Word16(max1), addv).0;
    }
    ii = add(&mut ctx, Word16(ii), Word16(t1)).0;
    let abs_ii = abs_s(&mut ctx, Word16(ii));
    jj = sub(&mut ctx, abs_ii, Word16(7)).0;
    if jj < 0 {
        let addv = mult(&mut ctx, Word16(max2), Word16(6554));
        max1 = add(&mut ctx, Word16(max1), addv).0;
    }

    if sub(&mut ctx, Word16(max1), Word16(max2)).0 < 0 {
        max1 = max2;
        t1 = t2;
    }
    if sub(&mut ctx, Word16(max1), Word16(max3)).0 < 0 {
        t1 = t3;
    }

    t1
}
