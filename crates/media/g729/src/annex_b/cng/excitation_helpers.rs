//! Annex B comfort-noise excitation helper primitives.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::w;
use crate::constants::{L_INTER10, L_SUBFR, UP_SAMP};
use crate::dsp::arith::{add, extract_l, negate, round, sub};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mac, l_mult, l_sub};
use crate::dsp::oper32::{l_extract, mpy_32_16};
use crate::dsp::shift::{l_shl, l_shr, shr};
use crate::dsp::types::{DspContext, Word32};
use crate::tables::annexa::{INTER_3L, TAB_ZONE};

pub(super) fn random_itu(seed: &mut i16) -> i16 {
    let mut ctx = DspContext::default();
    let l = l_mult(&mut ctx, w(*seed), w(31821));
    let l = l_shr(&mut ctx, l, 1);
    let l = l_add(&mut ctx, l, Word32(13849));
    *seed = extract_l(l).0;
    *seed
}
pub(super) fn pred_lt_3(old_exc: &mut [i16], exc_index: usize, t0: i16, frac_in: i16) {
    let mut ctx = DspContext::default();
    let mut x0 = exc_index.saturating_sub(t0 as usize);
    let mut frac = negate(&mut ctx, w(frac_in)).0;
    if frac < 0 {
        frac = add(&mut ctx, w(frac), w(UP_SAMP as i16)).0;
        x0 = x0.saturating_sub(1);
    }

    for j in 0..L_SUBFR {
        let x1 = x0;
        x0 += 1;
        let x2 = x0;
        let c1 = frac as usize;
        let c2 = sub(&mut ctx, w(UP_SAMP as i16), w(frac)).0 as usize;

        let mut s = Word32(0);
        let mut k = 0usize;
        for i in 0..L_INTER10 {
            s = l_mac(&mut ctx, s, w(old_exc[x1 - i]), w(INTER_3L[c1 + k]));
            s = l_mac(&mut ctx, s, w(old_exc[x2 + i]), w(INTER_3L[c2 + k]));
            k += UP_SAMP;
        }
        old_exc[exc_index + j] = round(&mut ctx, s).0;
    }
}

pub(super) fn gauss(seed: &mut i16) -> i16 {
    let mut ctx = DspContext::default();
    let mut l_acc = Word32(0);
    for _ in 0..12 {
        l_acc = l_add(&mut ctx, l_acc, l_deposit_l(w(random_itu(seed))));
    }
    l_acc = l_shr(&mut ctx, l_acc, 7);
    extract_l(l_acc).0
}

pub(super) fn sqrt_half(num: Word32) -> i16 {
    let mut ctx = DspContext::default();
    let mut rez = 0i16;
    let mut exp = 0x4000i16;

    for _ in 0..14 {
        let s = add(&mut ctx, w(rez), w(exp));
        let acc = l_mult(&mut ctx, s, s);
        let l_temp = l_sub(&mut ctx, num, acc);
        if l_temp.0 >= 0 {
            rez = add(&mut ctx, w(rez), w(exp)).0;
        }
        exp = shr(&mut ctx, w(exp), 1).0;
    }
    rez
}

pub(super) fn update_exc_err_l(l_exc_err: &mut [i32; 4], gain_pit: i16, t0: i16) {
    let mut ctx = DspContext::default();
    let mut l_worst = Word32(-1);
    let n = sub(&mut ctx, w(t0), w(L_SUBFR as i16)).0;

    if n < 0 {
        let mut l_temp = Word32(l_exc_err[0]);
        for _ in 0..2 {
            let (hi, lo) = l_extract(l_temp);
            l_temp = mpy_32_16(hi, lo, w(gain_pit));
            l_temp = l_shl(&mut ctx, l_temp, 1);
            l_temp = l_add(&mut ctx, Word32(0x0000_4000), l_temp);
            if l_sub(&mut ctx, l_temp, l_worst).0 > 0 {
                l_worst = l_temp;
            }
        }
    } else {
        let zone1 = TAB_ZONE[n as usize] as usize;
        let i = sub(&mut ctx, w(t0), w(1)).0;
        let zone2 = TAB_ZONE[i as usize] as usize;
        for &v in l_exc_err.iter().take(zone2 + 1).skip(zone1) {
            let (hi, lo) = l_extract(Word32(v));
            let mut l_temp = mpy_32_16(hi, lo, w(gain_pit));
            l_temp = l_shl(&mut ctx, l_temp, 1);
            l_temp = l_add(&mut ctx, Word32(0x0000_4000), l_temp);
            if l_sub(&mut ctx, l_temp, l_worst).0 > 0 {
                l_worst = l_temp;
            }
        }
    }

    for i in (1..4).rev() {
        l_exc_err[i] = l_exc_err[i - 1];
    }
    l_exc_err[0] = l_worst.0;
}
