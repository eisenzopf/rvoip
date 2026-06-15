//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::constants::{INV_COEF, NCAN1, NCAN2};
use crate::dsp::arith::{extract_h, mult};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mult, l_sub};
use crate::dsp::shift::{l_shl, l_shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::{THR1, THR2, coef, l_coef};

pub(crate) fn gbk_presel(best_gain: &[i16; 2], cand1: &mut usize, cand2: &mut usize, gcode0: i16) {
    let mut ctx = DspContext::default();

    let l_cfbg = l_mult(&mut ctx, Word16(coef(0, 0)), Word16(best_gain[0]));
    let mut l_acc = l_shr(&mut ctx, Word32(l_coef(1, 1)), 15);
    l_acc = l_add(&mut ctx, l_cfbg, l_acc);
    let mut acc_h = extract_h(l_acc).0;
    let l_preg = l_mult(&mut ctx, Word16(acc_h), Word16(gcode0));
    l_acc = l_shl(&mut ctx, l_deposit_l(Word16(best_gain[1])), 7);
    l_acc = l_sub(&mut ctx, l_acc, l_preg);
    acc_h = extract_h(l_shl(&mut ctx, l_acc, 2)).0;
    let l_tmp_x = l_mult(&mut ctx, Word16(acc_h), Word16(INV_COEF));

    l_acc = l_shr(&mut ctx, Word32(l_coef(0, 1)), 10);
    l_acc = l_sub(&mut ctx, l_cfbg, l_acc);
    acc_h = extract_h(l_acc).0;
    acc_h = mult(&mut ctx, Word16(acc_h), Word16(gcode0)).0;
    let l_tmp = l_mult(&mut ctx, Word16(acc_h), Word16(coef(1, 0)));
    let l_preg2 = l_mult(&mut ctx, Word16(coef(0, 0)), Word16(best_gain[1]));
    let preg2s = l_shr(&mut ctx, l_preg2, 3);
    l_acc = l_sub(&mut ctx, l_tmp, preg2s);
    acc_h = extract_h(l_shl(&mut ctx, l_acc, 2)).0;
    let l_tmp_y = l_mult(&mut ctx, Word16(acc_h), Word16(INV_COEF));

    let sft_y = (14 + 4 + 1) - 16;
    let sft_x = (15 + 4 + 1) - 15;

    if gcode0 > 0 {
        *cand1 = 0;
        while *cand1 < (8 - NCAN1) {
            let thr = l_mult(&mut ctx, Word16(THR1[*cand1]), Word16(gcode0));
            let rhs = l_shr(&mut ctx, thr, sft_y);
            if l_sub(&mut ctx, l_tmp_y, rhs).0 > 0 {
                *cand1 += 1;
            } else {
                break;
            }
        }

        *cand2 = 0;
        while *cand2 < (16 - NCAN2) {
            let thr = l_mult(&mut ctx, Word16(THR2[*cand2]), Word16(gcode0));
            let rhs = l_shr(&mut ctx, thr, sft_x);
            if l_sub(&mut ctx, l_tmp_x, rhs).0 > 0 {
                *cand2 += 1;
            } else {
                break;
            }
        }
    } else {
        *cand1 = 0;
        while *cand1 < (8 - NCAN1) {
            let thr = l_mult(&mut ctx, Word16(THR1[*cand1]), Word16(gcode0));
            let rhs = l_shr(&mut ctx, thr, sft_y);
            if l_sub(&mut ctx, l_tmp_y, rhs).0 < 0 {
                *cand1 += 1;
            } else {
                break;
            }
        }

        *cand2 = 0;
        while *cand2 < (16 - NCAN2) {
            let thr = l_mult(&mut ctx, Word16(THR2[*cand2]), Word16(gcode0));
            let rhs = l_shr(&mut ctx, thr, sft_x);
            if l_sub(&mut ctx, l_tmp_x, rhs).0 < 0 {
                *cand2 += 1;
            } else {
                break;
            }
        }
    }
}
