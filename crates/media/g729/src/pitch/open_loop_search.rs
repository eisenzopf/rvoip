//! Provenance: open-loop lag search helpers extracted from ITU `PITCH_A.C`.
//! Q-format: correlations in Q31-like accumulator, normalized scores in Q15-ish domain.

use crate::codec::state::OLD_WSP_LEN;
use crate::constants::{L_FRAME, PIT_MAX};
use crate::dsp::arith::extract_l;
use crate::dsp::arith32::{l_mac, l_sub};
use crate::dsp::div::Inv_sqrt;
use crate::dsp::oper32::{l_extract, mpy_32};
use crate::dsp::shift::{shl, shr};
use crate::dsp::types::{DspContext, MIN_32, Word16, Word32};

pub(super) fn scale_open_loop_signal(
    ctx: &mut DspContext,
    old_wsp: &[i16; OLD_WSP_LEN],
    old_wsp_offset: usize,
    pit_max: i16,
    l_frame: usize,
    scaled_signal: &mut [i16; L_FRAME + PIT_MAX as usize],
    scal_base: usize,
) {
    ctx.overflow = false;
    let mut sum = Word32(0);
    let mut i = -pit_max;
    while i < l_frame as i16 {
        let si = old_wsp[(old_wsp_offset as isize + i as isize) as usize];
        sum = l_mac(ctx, sum, Word16(si), Word16(si));
        i += 2;
    }

    if ctx.overflow {
        for i in -pit_max..l_frame as i16 {
            let si = old_wsp[(old_wsp_offset as isize + i as isize) as usize];
            scaled_signal[(scal_base as isize + i as isize) as usize] = shr(ctx, Word16(si), 3).0;
        }
        return;
    }

    let l_temp = l_sub(ctx, sum, Word32(1_048_576));
    if l_temp.0 < 0 {
        for i in -pit_max..l_frame as i16 {
            let si = old_wsp[(old_wsp_offset as isize + i as isize) as usize];
            scaled_signal[(scal_base as isize + i as isize) as usize] = shl(ctx, Word16(si), 3).0;
        }
    } else {
        for i in -pit_max..l_frame as i16 {
            let si = old_wsp[(old_wsp_offset as isize + i as isize) as usize];
            scaled_signal[(scal_base as isize + i as isize) as usize] = si;
        }
    }
}

pub(super) fn correlation_for_lag(
    ctx: &mut DspContext,
    scaled_signal: &[i16; L_FRAME + PIT_MAX as usize],
    scal_base: usize,
    l_frame: usize,
    lag: i16,
) -> Word32 {
    let mut corr = Word32(0);
    let mut j = 0usize;
    while j < l_frame {
        let p = scaled_signal[scal_base + j];
        let p1 = scaled_signal[(scal_base as isize + j as isize - lag as isize) as usize];
        corr = l_mac(ctx, corr, Word16(p), Word16(p1));
        j += 2;
    }
    corr
}

pub(super) fn best_lag_in_range(
    ctx: &mut DspContext,
    scaled_signal: &[i16; L_FRAME + PIT_MAX as usize],
    scal_base: usize,
    l_frame: usize,
    start: i16,
    end_exclusive: i16,
    step: i16,
) -> (Word32, i16) {
    let mut max = Word32(MIN_32);
    let mut best = start;
    let mut lag = start;
    while lag < end_exclusive {
        let corr = correlation_for_lag(ctx, scaled_signal, scal_base, l_frame, lag);
        if l_sub(ctx, corr, max).0 > 0 {
            max = corr;
            best = lag;
        }
        lag += step;
    }
    (max, best)
}

pub(super) fn normalized_correlation_score(
    ctx: &mut DspContext,
    scaled_signal: &[i16; L_FRAME + PIT_MAX as usize],
    scal_base: usize,
    l_frame: usize,
    max: Word32,
    lag: i16,
) -> i16 {
    let mut sum = Word32(1);
    let mut j = 0usize;
    while j < l_frame {
        let p = scaled_signal[(scal_base as isize + j as isize - lag as isize) as usize];
        sum = l_mac(ctx, sum, Word16(p), Word16(p));
        j += 2;
    }

    let inv = Inv_sqrt(sum);
    let (max_h, max_l) = l_extract(max);
    let (ener_h, ener_l) = l_extract(inv);
    extract_l(mpy_32(max_h, max_l, ener_h, ener_l)).0
}
