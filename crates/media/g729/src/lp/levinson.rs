#![allow(clippy::manual_memcpy)]
//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::codec::state::EncoderState;
use crate::constants::{M, MP1};
use crate::dsp::arith::{abs_s, add, round, sub};
use crate::dsp::arith32::{l_abs, l_add, l_negate, l_sub};
use crate::dsp::oper32::{div_32, l_comp, l_extract, mpy_32};
use crate::dsp::shift::{l_shl, l_shr, norm_l, shr};
use crate::dsp::types::{DspContext, Word16, Word32};

pub(crate) fn levinson_10(
    state: &mut EncoderState,
    r_h: &[i16; M + 1],
    r_l: &[i16; M + 1],
    a: &mut [i16; MP1],
    rc: &mut [i16; M],
    err_out: Option<&mut i16>,
) {
    let mut ctx = DspContext::default();

    let mut ah = [0i16; MP1];
    let mut al = [0i16; MP1];
    let mut anh = [0i16; MP1];
    let mut anl = [0i16; MP1];

    let t1 = l_comp(Word16(r_h[1]), Word16(r_l[1]));
    let t2_abs = l_abs(&mut ctx, t1);
    let mut t0 = div_32(t2_abs, Word16(r_h[0]), Word16(r_l[0]));
    if t1.0 > 0 {
        t0 = l_negate(&mut ctx, t0);
    }

    let (mut kh, mut kl) = l_extract(t0);
    rc[0] = kh.0;

    t0 = l_shr(&mut ctx, t0, 4);
    let (a1h, a1l) = l_extract(t0);
    ah[1] = a1h.0;
    al[1] = a1l.0;

    t0 = mpy_32(kh, kl, kh, kl);
    t0 = l_abs(&mut ctx, t0);
    t0 = l_sub(&mut ctx, Word32(0x7fff_ffff), t0);
    let (mut hi, mut lo) = l_extract(t0);
    t0 = mpy_32(Word16(r_h[0]), Word16(r_l[0]), hi, lo);

    let mut alp_exp = norm_l(t0);
    t0 = l_shl(&mut ctx, t0, alp_exp);
    let (mut alp_h, mut alp_l) = l_extract(t0);

    for i in 2..=M {
        t0 = Word32(0);
        for j in 1..i {
            let t = mpy_32(
                Word16(r_h[j]),
                Word16(r_l[j]),
                Word16(ah[i - j]),
                Word16(al[i - j]),
            );
            t0 = l_add(&mut ctx, t0, t);
        }

        t0 = l_shl(&mut ctx, t0, 4);
        let ri = l_comp(Word16(r_h[i]), Word16(r_l[i]));
        t0 = l_add(&mut ctx, t0, ri);

        let t1_abs = l_abs(&mut ctx, t0);
        let mut t2 = div_32(t1_abs, alp_h, alp_l);
        if t0.0 > 0 {
            t2 = l_negate(&mut ctx, t2);
        }
        t2 = l_shl(&mut ctx, t2, alp_exp);
        let (kh2, kl2) = l_extract(t2);
        kh = kh2;
        kl = kl2;
        rc[i - 1] = kh.0;

        let abs_kh = abs_s(&mut ctx, kh);
        if sub(&mut ctx, abs_kh, Word16(32750)).0 > 0 {
            a.copy_from_slice(&state.old_a);
            rc[0] = state.old_rc[0];
            rc[1] = state.old_rc[1];
            return;
        }

        for j in 1..i {
            let t = mpy_32(kh, kl, Word16(ah[i - j]), Word16(al[i - j]));
            let t = l_add(&mut ctx, t, l_comp(Word16(ah[j]), Word16(al[j])));
            let (h, l) = l_extract(t);
            anh[j] = h.0;
            anl[j] = l.0;
        }

        let tk = l_shr(&mut ctx, t2, 4);
        let (h, l) = l_extract(tk);
        anh[i] = h.0;
        anl[i] = l.0;

        t0 = mpy_32(kh, kl, kh, kl);
        t0 = l_abs(&mut ctx, t0);
        t0 = l_sub(&mut ctx, Word32(0x7fff_ffff), t0);
        let (hi2, lo2) = l_extract(t0);
        hi = hi2;
        lo = lo2;
        t0 = mpy_32(alp_h, alp_l, hi, lo);

        let nrm = norm_l(t0);
        t0 = l_shl(&mut ctx, t0, nrm);
        let (ahv, alv) = l_extract(t0);
        alp_h = ahv;
        alp_l = alv;
        alp_exp = add(&mut ctx, Word16(alp_exp), Word16(nrm)).0;

        for j in 1..=i {
            ah[j] = anh[j];
            al[j] = anl[j];
        }
    }

    if let Some(err) = err_out {
        *err = shr(&mut ctx, Word16(alp_h.0), alp_exp).0;
    }

    a[0] = 4096;
    state.old_a[0] = 4096;
    for i in 1..=M {
        let t = l_comp(Word16(ah[i]), Word16(al[i]));
        let t = l_shl(&mut ctx, t, 1);
        let ai = round(&mut ctx, t).0;
        a[i] = ai;
        state.old_a[i] = ai;
    }
    state.old_rc[0] = rc[0];
    state.old_rc[1] = rc[1];
}
