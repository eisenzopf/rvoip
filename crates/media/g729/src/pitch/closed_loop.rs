//! Closed-loop fractional pitch search.
//! Provenance: Pitch search and lag coding adapted from ITU G.729 open/closed-loop pitch routines.
//! Q-format: Lag correlations and adaptive codebook gains use Q0/Q15 accumulator scaling.

use crate::codec::state::ENC_OLD_EXC_LEN;
use crate::constants::L_SUBFR;
use crate::dsp::arith::{round, sub};
use crate::dsp::arith32::{l_mac, l_sub};
use crate::dsp::div::div_s;
use crate::dsp::shift::{l_shl, norm_l, shr};
use crate::dsp::types::{DspContext, MIN_32, Word16, Word32};
use crate::fixed_cb::correlation::correlate_target_with_impulse;
use crate::pitch::pred_lt3::interpolate_excitation_encode;

fn dot_product_at(x: &[i16], x_off: usize, y: &[i16], y_off: usize, lg: usize) -> Word32 {
    let mut ctx = DspContext::default();
    let mut s = Word32(0);
    for i in 0..lg {
        s = l_mac(&mut ctx, s, Word16(x[x_off + i]), Word16(y[y_off + i]));
    }
    s
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn search_closed_loop(
    old_exc: &mut [i16; ENC_OLD_EXC_LEN],
    exc_index: usize,
    xn: &[i16; L_SUBFR],
    h: &[i16; L_SUBFR],
    t0_min: i16,
    t0_max: i16,
    i_subfr: usize,
    pit_frac: &mut i16,
) -> i16 {
    let mut ctx = DspContext::default();
    let mut dn = [0i16; L_SUBFR];
    let mut exc_tmp = [0i16; L_SUBFR];
    correlate_target_with_impulse(h, xn, &mut dn);

    let mut max = Word32(MIN_32);
    let mut t0 = t0_min;
    for t in t0_min..=t0_max {
        let corr = dot_product_at(&dn, 0, old_exc, exc_index - t as usize, L_SUBFR);
        if l_sub(&mut ctx, corr, max).0 > 0 {
            max = corr;
            t0 = t;
        }
    }

    interpolate_excitation_encode(old_exc, exc_index, t0, 0);
    max = dot_product_at(&dn, 0, old_exc, exc_index, L_SUBFR);
    *pit_frac = 0;

    if i_subfr == 0 && t0 > 84 {
        return t0;
    }

    exc_tmp.copy_from_slice(&old_exc[exc_index..exc_index + L_SUBFR]);

    interpolate_excitation_encode(old_exc, exc_index, t0, -1);
    let corr_m1 = dot_product_at(&dn, 0, old_exc, exc_index, L_SUBFR);
    if l_sub(&mut ctx, corr_m1, max).0 > 0 {
        max = corr_m1;
        *pit_frac = -1;
        exc_tmp.copy_from_slice(&old_exc[exc_index..exc_index + L_SUBFR]);
    }

    interpolate_excitation_encode(old_exc, exc_index, t0, 1);
    let corr_p1 = dot_product_at(&dn, 0, old_exc, exc_index, L_SUBFR);
    if l_sub(&mut ctx, corr_p1, max).0 > 0 {
        *pit_frac = 1;
    } else {
        old_exc[exc_index..exc_index + L_SUBFR].copy_from_slice(&exc_tmp);
    }

    t0
}

pub(crate) fn g_pitch(xn: &[i16; L_SUBFR], y1: &[i16; L_SUBFR], g_coeff: &mut [i16; 4]) -> i16 {
    let mut ctx = DspContext::default();
    let mut scaled_y1 = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        scaled_y1[i] = shr(&mut ctx, Word16(y1[i]), 2).0;
    }

    ctx.overflow = false;
    let mut s = Word32(1);
    for &v in y1 {
        s = l_mac(&mut ctx, s, Word16(v), Word16(v));
    }
    let (exp_yy, yy) = if !ctx.overflow {
        let e = norm_l(s);
        let sy = l_shl(&mut ctx, s, e);
        let y = round(&mut ctx, sy).0;
        (e, y)
    } else {
        let mut s2 = Word32(1);
        for &v in &scaled_y1 {
            s2 = l_mac(&mut ctx, s2, Word16(v), Word16(v));
        }
        let e = sub(&mut ctx, Word16(norm_l(s2)), Word16(4)).0;
        let sy = l_shl(&mut ctx, s2, e + 4);
        let y = round(&mut ctx, sy).0;
        (e, y)
    };

    ctx.overflow = false;
    s = Word32(0);
    for i in 0..L_SUBFR {
        s = l_mac(&mut ctx, s, Word16(xn[i]), Word16(y1[i]));
    }
    let (exp_xy, xy) = if !ctx.overflow {
        let e = norm_l(s);
        let sx = l_shl(&mut ctx, s, e);
        let x = round(&mut ctx, sx).0;
        (e, x)
    } else {
        let mut s2 = Word32(0);
        for i in 0..L_SUBFR {
            s2 = l_mac(&mut ctx, s2, Word16(xn[i]), Word16(scaled_y1[i]));
        }
        let e = sub(&mut ctx, Word16(norm_l(s2)), Word16(2)).0;
        let sx = l_shl(&mut ctx, s2, e + 2);
        let x = round(&mut ctx, sx).0;
        (e, x)
    };

    g_coeff[0] = yy;
    g_coeff[1] = sub(&mut ctx, Word16(15), Word16(exp_yy)).0;
    g_coeff[2] = xy;
    g_coeff[3] = sub(&mut ctx, Word16(15), Word16(exp_xy)).0;

    if xy <= 0 {
        g_coeff[3] = -15;
        return 0;
    }

    let xyh = shr(&mut ctx, Word16(xy), 1);
    let mut gain = div_s(xyh, Word16(yy)).0;
    let sh = sub(&mut ctx, Word16(exp_xy), Word16(exp_yy)).0;
    gain = shr(&mut ctx, Word16(gain), sh).0;
    if sub(&mut ctx, Word16(gain), Word16(19661)).0 > 0 {
        gain = 19661;
    }
    gain
}
