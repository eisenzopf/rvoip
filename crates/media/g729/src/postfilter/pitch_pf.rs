#![allow(clippy::manual_memcpy)]
//! Provenance: Post-filter stages adapted from ITU G.729 Annex A formant/pitch/AGC post-processing.
//! Q-format: Post-filter coefficients and synthesis states use Q12/Q14/Q15 fixed-point paths.

use crate::codec::state::RES2_BUF_LEN;
use crate::constants::{GAMMAP, GAMMAP_2, INV_GAMMAP, L_SUBFR, PIT_MAX};
use crate::dsp::arith::{add, mult, round, sub};
use crate::dsp::arith32::{l_mac, l_mult, l_sub};
use crate::dsp::div::div_s;
use crate::dsp::shift::{l_shl, norm_l, shr};
use crate::dsp::types::{DspContext, MIN_32, Word16, Word32};

pub(crate) fn pitch_post_filter(
    signal: &[i16; RES2_BUF_LEN],
    scal_sig: &[i16; RES2_BUF_LEN],
    t0_min: i16,
    t0_max: i16,
    signal_pst: &mut [i16; L_SUBFR],
) {
    let mut ctx = DspContext::default();
    let base = PIT_MAX as usize;
    let mut cor_max = Word32(MIN_32);
    let mut best_t0 = t0_min;

    for i in t0_min..=t0_max {
        let mut corr = Word32(0);
        for j in 0..L_SUBFR {
            corr = l_mac(
                &mut ctx,
                corr,
                Word16(scal_sig[base + j]),
                Word16(scal_sig[base + j - i as usize]),
            );
        }
        if l_sub(&mut ctx, corr, cor_max).0 > 0 {
            cor_max = corr;
            best_t0 = i;
        }
    }

    let mut ener = Word32(1);
    for i in 0..L_SUBFR {
        ener = l_mac(
            &mut ctx,
            ener,
            Word16(scal_sig[base + i - best_t0 as usize]),
            Word16(scal_sig[base + i - best_t0 as usize]),
        );
    }

    let mut ener0 = Word32(1);
    for i in 0..L_SUBFR {
        ener0 = l_mac(
            &mut ctx,
            ener0,
            Word16(scal_sig[base + i]),
            Word16(scal_sig[base + i]),
        );
    }

    if cor_max.0 < 0 {
        cor_max = Word32(0);
    }

    let mut temp = cor_max;
    if ener.0 > temp.0 {
        temp = ener;
    }
    if ener0.0 > temp.0 {
        temp = ener0;
    }

    let j = norm_l(temp);
    let cmax = {
        let t = l_shl(&mut ctx, cor_max, j);
        round(&mut ctx, t)
    };
    let en = {
        let t = l_shl(&mut ctx, ener, j);
        round(&mut ctx, t)
    };
    let en0 = {
        let t = l_shl(&mut ctx, ener0, j);
        round(&mut ctx, t)
    };

    let mut t = l_mult(&mut ctx, cmax, cmax);
    let t2 = l_mult(&mut ctx, en, en0);
    let t2 = crate::dsp::shift::l_shr(&mut ctx, t2, 1);
    t = l_sub(&mut ctx, t, t2);
    if t.0 < 0 {
        for i in 0..L_SUBFR {
            signal_pst[i] = signal[base + i];
        }
        return;
    }

    let (g0, gain) = if sub(&mut ctx, cmax, en).0 > 0 {
        (INV_GAMMAP, GAMMAP_2)
    } else {
        let m = mult(&mut ctx, cmax, Word16(GAMMAP));
        let c = shr(&mut ctx, m, 1);
        let e = shr(&mut ctx, en, 1);
        let d = add(&mut ctx, c, e);
        if d.0 > 0 {
            let g = div_s(c, d).0;
            (sub(&mut ctx, Word16(32767), Word16(g)).0, g)
        } else {
            (32767, 0)
        }
    };

    for i in 0..L_SUBFR {
        let a = mult(&mut ctx, Word16(g0), Word16(signal[base + i]));
        let b = mult(
            &mut ctx,
            Word16(gain),
            Word16(signal[base + i - best_t0 as usize]),
        );
        signal_pst[i] = add(&mut ctx, a, b).0;
    }
}
