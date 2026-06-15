//! Annex B SID codebook search helpers.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::w;
use crate::constants::M;
use crate::dsp::arith::{extract_h, mult, sub};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, MAX_16, Word32};
use crate::tables::annexa::{lspcb1, lspcb2};
use crate::tables::sid::{MP_WEIGHT, NOISE_FG_SUM};

pub(super) fn new_ml_search_1(
    d_data: &[i16],
    j: i16,
    new_d_data: &mut [i16],
    k: i16,
    best_indx: &mut [i16],
    ptr_back: &mut [i16],
    ptr_tab: &[i16; 32],
    mq: i16,
) {
    let mut ctx = DspContext::default();
    let mut sum = [0i16; 10 * 10];
    let mut min = [MAX_16; 10];
    let mut min_indx_p = [0i16; 10];
    let mut min_indx_m = [0i16; 10];

    for p in 0..j as usize {
        for m in 0..mq as usize {
            let mut acc = Word32(0);
            for l in 0..M {
                let tmp = sub(
                    &mut ctx,
                    w(d_data[p * M + l]),
                    w(lspcb1(ptr_tab[m] as usize, l)),
                )
                .0;
                acc = l_mac(&mut ctx, acc, w(tmp), w(tmp));
            }
            let mut s = extract_h(acc).0;
            s = mult(&mut ctx, w(s), w(MP_WEIGHT[p])).0;
            sum[p * mq as usize + m] = s;
        }
    }

    for q in 0..k as usize {
        for p in 0..j as usize {
            for m in 0..mq as usize {
                if sub(&mut ctx, w(sum[p * mq as usize + m]), w(min[q])).0 < 0 {
                    min[q] = sum[p * mq as usize + m];
                    min_indx_p[q] = p as i16;
                    min_indx_m[q] = m as i16;
                }
            }
        }
        sum[min_indx_p[q] as usize * mq as usize + min_indx_m[q] as usize] = MAX_16;
    }

    for q in 0..k as usize {
        for l in 0..M {
            new_d_data[q * M + l] = sub(
                &mut ctx,
                w(d_data[min_indx_p[q] as usize * M + l]),
                w(lspcb1(ptr_tab[min_indx_m[q] as usize] as usize, l)),
            )
            .0;
        }
        ptr_back[q] = min_indx_p[q];
        best_indx[q] = min_indx_m[q];
    }
}

pub(super) fn new_ml_search_2(
    d_data: &[i16],
    weight: &[i16; M],
    j: i16,
    new_d_data: &mut [i16],
    k: i16,
    best_indx: &mut [i16],
    ptr_prd: &[i16],
    ptr_back: &mut [i16],
    ptr_tab: &[[i16; 16]; 2],
    mq: i16,
) {
    let mut ctx = DspContext::default();
    let mut sum = [0i16; 10 * 10];
    let mut min = [MAX_16; 10];
    let mut min_indx_p = [0i16; 10];
    let mut min_indx_m = [0i16; 10];

    for p in 0..j as usize {
        for m in 0..mq as usize {
            let mut acc = Word32(0);
            for l in 0..(M / 2) {
                let mut tmp1 = l_mult(
                    &mut ctx,
                    w(NOISE_FG_SUM[ptr_prd[p] as usize][l]),
                    w(NOISE_FG_SUM[ptr_prd[p] as usize][l]),
                );
                tmp1 = l_shl(&mut ctx, tmp1, 2);
                let mut t1 = extract_h(tmp1).0;
                t1 = mult(&mut ctx, w(t1), w(weight[l])).0;
                let t2 = sub(
                    &mut ctx,
                    w(d_data[p * M + l]),
                    w(lspcb2(ptr_tab[0][m] as usize, l)),
                )
                .0;
                let mut t1l = l_mult(&mut ctx, w(t1), w(t2));
                t1l = l_shl(&mut ctx, t1l, 3);
                let t1h = extract_h(t1l).0;
                acc = l_mac(&mut ctx, acc, w(t1h), w(t2));
            }
            for l in (M / 2)..M {
                let mut tmp1 = l_mult(
                    &mut ctx,
                    w(NOISE_FG_SUM[ptr_prd[p] as usize][l]),
                    w(NOISE_FG_SUM[ptr_prd[p] as usize][l]),
                );
                tmp1 = l_shl(&mut ctx, tmp1, 2);
                let mut t1 = extract_h(tmp1).0;
                t1 = mult(&mut ctx, w(t1), w(weight[l])).0;
                let t2 = sub(
                    &mut ctx,
                    w(d_data[p * M + l]),
                    w(lspcb2(ptr_tab[1][m] as usize, l)),
                )
                .0;
                let mut t1l = l_mult(&mut ctx, w(t1), w(t2));
                t1l = l_shl(&mut ctx, t1l, 3);
                let t1h = extract_h(t1l).0;
                acc = l_mac(&mut ctx, acc, w(t1h), w(t2));
            }
            sum[p * mq as usize + m] = extract_h(acc).0;
        }
    }

    for q in 0..k as usize {
        for p in 0..j as usize {
            for m in 0..mq as usize {
                if sub(&mut ctx, w(sum[p * mq as usize + m]), w(min[q])).0 < 0 {
                    min[q] = sum[p * mq as usize + m];
                    min_indx_p[q] = p as i16;
                    min_indx_m[q] = m as i16;
                }
            }
        }
        sum[min_indx_p[q] as usize * mq as usize + min_indx_m[q] as usize] = MAX_16;
    }

    for q in 0..k as usize {
        for l in 0..(M / 2) {
            new_d_data[q * M + l] = sub(
                &mut ctx,
                w(d_data[min_indx_p[q] as usize * M + l]),
                w(lspcb2(ptr_tab[0][min_indx_m[q] as usize] as usize, l)),
            )
            .0;
        }
        for l in (M / 2)..M {
            new_d_data[q * M + l] = sub(
                &mut ctx,
                w(d_data[min_indx_p[q] as usize * M + l]),
                w(lspcb2(ptr_tab[1][min_indx_m[q] as usize] as usize, l)),
            )
            .0;
        }
        ptr_back[q] = min_indx_p[q];
        best_indx[q] = min_indx_m[q];
    }
}
