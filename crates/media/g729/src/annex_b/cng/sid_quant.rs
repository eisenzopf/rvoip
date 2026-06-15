//! Annex B SID quantization helpers.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::{
    sid_search::{new_ml_search_1, new_ml_search_2},
    w,
};
use crate::constants::M;
use crate::dsp::arith::{add, mult, mult_r, sub};
use crate::dsp::arith32::{l_add, l_deposit_l};
use crate::dsp::div::Log2;
use crate::dsp::oper32::{l_extract, mpy_32_16};
use crate::dsp::shift::{l_shl, shl, shr};
use crate::dsp::types::{DspContext, Word32};
use crate::tables::annexa::{lspcb1, lspcb2};
use crate::tables::sid::{FACT, MARG, PTR_TAB_1, PTR_TAB_2};

pub(super) fn qnt_e(
    errlsf: &[i16],
    weight: &[i16; M],
    din: i16,
    qlsf: &mut [i16; M],
    pptr: &mut i16,
    dout: i16,
    cluster: &mut [i16; 2],
    ms: &[i16; 2],
) {
    let mut d_data = [[0i16; 10 * M]; 2];
    let mut best_indx = [[0i16; 10]; 2];
    let mut ptr_back = [[0i16; 10]; 2];

    new_ml_search_1(
        errlsf,
        din,
        &mut d_data[0],
        4,
        &mut best_indx[0],
        &mut ptr_back[0],
        &PTR_TAB_1,
        ms[0],
    );

    let (d0, d1) = d_data.split_at_mut(1);
    let d0_ref: &[i16] = &d0[0];
    let d1_mut: &mut [i16] = &mut d1[0];
    let (pb0, pb1) = ptr_back.split_at_mut(1);
    let pb0_ref: &[i16] = &pb0[0];
    let pb1_mut: &mut [i16] = &mut pb1[0];

    new_ml_search_2(
        d0_ref,
        weight,
        4,
        d1_mut,
        dout,
        &mut best_indx[1],
        pb0_ref,
        pb1_mut,
        &PTR_TAB_2,
        ms[1],
    );

    cluster[1] = best_indx[1][0];
    let ptr = ptr_back[1][0] as usize;
    cluster[0] = best_indx[0][ptr];
    *pptr = ptr_back[0][ptr];

    let mut ctx = DspContext::default();
    for (l, q) in qlsf.iter_mut().enumerate().take(M) {
        *q = lspcb1(PTR_TAB_1[cluster[0] as usize] as usize, l);
    }
    for i in 0..(M / 2) {
        qlsf[i] = add(
            &mut ctx,
            w(qlsf[i]),
            w(lspcb2(PTR_TAB_2[0][cluster[1] as usize] as usize, i)),
        )
        .0;
    }
    for i in (M / 2)..M {
        qlsf[i] = add(
            &mut ctx,
            w(qlsf[i]),
            w(lspcb2(PTR_TAB_2[1][cluster[1] as usize] as usize, i)),
        )
        .0;
    }
}
/// Public function `qua_sidgain`.
pub fn qua_sidgain(
    ener: &[i16; 2],
    sh_ener: &[i16; 2],
    nb_ener: i16,
    enerq: &mut i16,
    idx: &mut i16,
) {
    let mut ctx = DspContext::default();
    let (l_x, sh1) = if nb_ener == 0 {
        let mut l_acc = l_deposit_l(w(ener[0]));
        l_acc = l_shl(&mut ctx, l_acc, sh_ener[0]);
        let (hi, lo) = l_extract(l_acc);
        (mpy_32_16(hi, lo, w(FACT[0])), 0)
    } else {
        let n = nb_ener as usize;
        let mut sh = sh_ener[0];
        for &v in sh_ener.iter().take(n).skip(1) {
            if v < sh {
                sh = v;
            }
        }
        sh = add(&mut ctx, w(sh), w(16 - MARG[n])).0;

        let mut lx = Word32(0);
        for i in 0..n {
            let temp = sub(&mut ctx, w(sh), w(sh_ener[i])).0;
            let mut l_acc = l_deposit_l(w(ener[i]));
            l_acc = l_shl(&mut ctx, l_acc, temp);
            lx = l_add(&mut ctx, lx, l_acc);
        }
        let (hi, lo) = l_extract(lx);
        (mpy_32_16(hi, lo, w(FACT[n])), sh)
    };

    *idx = quant_energy(l_x, sh1, enerq);
}

fn quant_energy(l_x: Word32, sh: i16, enerq: &mut i16) -> i16 {
    let mut ctx = DspContext::default();
    let mut exp = w(0);
    let mut frac = w(0);
    Log2(l_x, &mut exp, &mut frac);
    let temp = sub(&mut ctx, exp, w(sh));
    let mut e_tmp = shl(&mut ctx, temp, 10).0;
    let frac_mul = mult_r(&mut ctx, frac, w(1024));
    e_tmp = add(&mut ctx, w(e_tmp), frac_mul).0;

    let mut tmp = sub(&mut ctx, w(e_tmp), w(-2721)).0;
    if tmp <= 0 {
        *enerq = -12;
        return 0;
    }

    tmp = sub(&mut ctx, w(e_tmp), w(22111)).0;
    if tmp > 0 {
        *enerq = 66;
        return 31;
    }

    tmp = sub(&mut ctx, w(e_tmp), w(4762)).0;
    if tmp <= 0 {
        e_tmp = add(&mut ctx, w(e_tmp), w(3401)).0;
        let mut index = mult(&mut ctx, w(e_tmp), w(24)).0;
        if index < 1 {
            index = 1;
        }
        let idx4 = shl(&mut ctx, w(index), 2);
        *enerq = sub(&mut ctx, idx4, w(8)).0;
        return index;
    }

    e_tmp = sub(&mut ctx, w(e_tmp), w(340)).0;
    let tmpm = mult(&mut ctx, w(e_tmp), w(193));
    let tmps = shr(&mut ctx, tmpm, 2);
    let mut index = sub(&mut ctx, tmps, w(1)).0;
    if index < 6 {
        index = 6;
    }
    let idx2 = shl(&mut ctx, w(index), 1);
    *enerq = add(&mut ctx, idx2, w(4)).0;
    index
}
