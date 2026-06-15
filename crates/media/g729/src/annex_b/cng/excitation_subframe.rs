//! Annex B comfort-noise subframe synthesis.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::excitation_helpers::{gauss, pred_lt_3, sqrt_half, update_exc_err_l};
use super::excitation_params::sample_excitation_params;
use super::w;
use crate::constants::{FRAC1, G_MAX, K0, L_SUBFR};
use crate::dsp::arith::{abs_s, add, extract_h, extract_l, mult_r, negate, sub};
use crate::dsp::arith32::{l_mac, l_mult, l_sub};
use crate::dsp::oper32::{l_extract, mpy_32_16};
use crate::dsp::shift::{l_shl, l_shr, norm_l, norm_s, shl, shr, shr_r};
use crate::dsp::types::{DspContext, Word32};

#[allow(clippy::too_many_arguments)]
pub(super) fn synth_subframe(
    cur_gain: i16,
    old_exc: &mut [i16],
    exc_index: usize,
    cur_exc: usize,
    seed: &mut i16,
    flag_cod: bool,
    enc_taming: &mut Option<&mut [i32; 4]>,
) {
    let mut ctx = DspContext::default();
    let params = sample_excitation_params(seed);
    let pos = params.pos;
    let sign = params.sign;
    let t0 = params.t0;
    let frac = params.frac;
    let gp = params.gp;
    let gp2 = params.gp2;

    let mut excg = [0i16; L_SUBFR];
    let mut excs = [0i16; L_SUBFR];
    let mut l_acc = Word32(0);
    for (i, eg) in excg.iter_mut().enumerate().take(L_SUBFR) {
        let g = gauss(seed);
        l_acc = l_mac(&mut ctx, l_acc, w(g), w(g));
        *eg = g;
        excs[i] = 0;
    }

    l_acc = crate::dsp::div::Inv_sqrt(l_shr(&mut ctx, l_acc, 1));
    let (hi, lo) = l_extract(l_acc);
    let mut temp = mult_r(&mut ctx, w(cur_gain), w(FRAC1)).0;
    temp = add(&mut ctx, w(cur_gain), w(temp)).0;

    l_acc = mpy_32_16(hi, lo, w(temp));
    let mut sh = norm_l(l_acc);
    temp = extract_h(l_shl(&mut ctx, l_acc, sh)).0;

    sh = sub(&mut ctx, w(sh), w(14)).0;
    for eg in excg.iter_mut().take(L_SUBFR) {
        let t = mult_r(&mut ctx, w(*eg), w(temp)).0;
        *eg = shr_r(&mut ctx, w(t), sh).0;
    }

    pred_lt_3(old_exc, exc_index + cur_exc, t0, frac);

    let mut max = 0i16;
    for i in 0..L_SUBFR {
        let idx = exc_index + cur_exc + i;
        let t = mult_r(&mut ctx, w(old_exc[idx]), w(gp2)).0;
        let t = add(&mut ctx, w(t), w(excg[i])).0;
        old_exc[idx] = t;
        let a = abs_s(&mut ctx, w(t)).0;
        if sub(&mut ctx, w(a), w(max)).0 > 0 {
            max = a;
        }
    }

    if max == 0 {
        sh = 0;
    } else {
        sh = sub(&mut ctx, w(3), w(norm_s(w(max)))).0;
        if sh <= 0 {
            sh = 0;
        }
    }
    for i in 0..L_SUBFR {
        let idx = exc_index + cur_exc + i;
        excs[i] = shr(&mut ctx, w(old_exc[idx]), sh).0;
    }

    let mut l_ener = Word32(0);
    for &v in excs.iter().take(L_SUBFR) {
        l_ener = l_mac(&mut ctx, l_ener, w(v), w(v));
    }

    let mut inter_exc = 0i16;
    for i in 0..4 {
        let j = pos[i] as usize;
        if sign[i] == 0 {
            inter_exc = sub(&mut ctx, w(inter_exc), w(excs[j])).0;
        } else {
            inter_exc = add(&mut ctx, w(inter_exc), w(excs[j])).0;
        }
    }

    l_acc = l_mult(&mut ctx, w(cur_gain), w(L_SUBFR as i16));
    l_acc = l_shr(&mut ctx, l_acc, 6);
    temp = extract_l(l_acc).0;
    let l_k = l_mult(&mut ctx, w(cur_gain), w(temp));
    let sh2 = shl(&mut ctx, w(sh), 1);
    let t = add(&mut ctx, w(1), sh2).0;
    l_acc = l_shr(&mut ctx, l_k, t);

    l_acc = l_sub(&mut ctx, l_acc, l_ener);
    inter_exc = shr(&mut ctx, w(inter_exc), 1).0;
    l_acc = l_mac(&mut ctx, l_acc, w(inter_exc), w(inter_exc));
    sh = add(&mut ctx, w(sh), w(1)).0;

    let mut gp_loc = gp;
    if l_acc.0 < 0 {
        old_exc[exc_index + cur_exc..exc_index + cur_exc + L_SUBFR]
            .copy_from_slice(&excg[..L_SUBFR]);
        let t1 = abs_s(&mut ctx, w(excg[pos[0] as usize])).0
            | abs_s(&mut ctx, w(excg[pos[1] as usize])).0;
        let t2 = abs_s(&mut ctx, w(excg[pos[2] as usize])).0
            | abs_s(&mut ctx, w(excg[pos[3] as usize])).0;
        temp = t1 | t2;
        sh = if (temp & 0x4000) == 0 { 1 } else { 2 };

        inter_exc = 0;
        for i in 0..4 {
            let t = shr(&mut ctx, w(excg[pos[i] as usize]), sh).0;
            if sign[i] == 0 {
                inter_exc = sub(&mut ctx, w(inter_exc), w(t)).0;
            } else {
                inter_exc = add(&mut ctx, w(inter_exc), w(t)).0;
            }
        }

        let (hk, lk) = l_extract(l_k);
        l_acc = mpy_32_16(hk, lk, w(K0));
        let sh2 = shl(&mut ctx, w(sh), 1);
        temp = sub(&mut ctx, sh2, w(1)).0;
        l_acc = l_shr(&mut ctx, l_acc, temp);
        l_acc = l_mac(&mut ctx, l_acc, w(inter_exc), w(inter_exc));
        gp_loc = 0;
    }

    let temp2 = sqrt_half(l_acc);
    let mut x1 = sub(&mut ctx, w(temp2), w(inter_exc)).0;
    let xsum = add(&mut ctx, w(inter_exc), w(temp2)).0;
    let x2 = negate(&mut ctx, w(xsum)).0;
    let ax2 = abs_s(&mut ctx, w(x2)).0;
    let ax1 = abs_s(&mut ctx, w(x1)).0;
    if sub(&mut ctx, w(ax2), w(ax1)).0 < 0 {
        x1 = x2;
    }

    temp = sub(&mut ctx, w(2), w(sh)).0;
    let mut g = shr_r(&mut ctx, w(x1), temp).0;
    if g >= 0 {
        if sub(&mut ctx, w(g), w(G_MAX)).0 > 0 {
            g = G_MAX;
        }
    } else if add(&mut ctx, w(g), w(G_MAX)).0 < 0 {
        g = negate(&mut ctx, w(G_MAX)).0;
    }

    for i in 0..4 {
        let j = pos[i] as usize;
        let idx = exc_index + cur_exc + j;
        if sign[i] != 0 {
            old_exc[idx] = add(&mut ctx, w(old_exc[idx]), w(g)).0;
        } else {
            old_exc[idx] = sub(&mut ctx, w(old_exc[idx]), w(g)).0;
        }
    }

    if flag_cod {
        if let Some(l_exc_err) = enc_taming.as_deref_mut() {
            update_exc_err_l(l_exc_err, gp_loc, t0);
        }
    }
}
