//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::codec::state::EncoderState;
use crate::constants::{GPCLIP2, L_SUBFR};
use crate::dsp::arith::{add, extract_h, negate, sub};
use crate::dsp::arith32::{l_deposit_h, l_deposit_l, l_mult, l_sub};
use crate::dsp::div::div_s;
use crate::dsp::oper32::l_extract;
use crate::dsp::shift::{l_shl, l_shr, norm_l, shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::gain::predict::predict_encoder_gain;
use crate::gain::presel::gbk_presel;

pub(crate) struct QuantizePrep {
    pub gcode0: i16,
    pub exp_gcode0: i16,
    pub cand1: usize,
    pub cand2: usize,
    pub coeff: [i16; 5],
    pub coeff_lsf: [i16; 5],
}

pub(crate) fn prepare_quantization(
    state: &EncoderState,
    code: &[i16; L_SUBFR],
    g_coeff: &[i16; 5],
    exp_coeff: &[i16; 5],
    tameflag: i16,
) -> QuantizePrep {
    let mut ctx = DspContext::default();
    let mut gcode0 = 0i16;
    let mut exp_gcode0 = 0i16;
    predict_encoder_gain(&state.past_qua_en, code, &mut gcode0, &mut exp_gcode0);

    let l_tmp1 = l_mult(&mut ctx, Word16(g_coeff[0]), Word16(g_coeff[2]));
    let e1_t = add(&mut ctx, Word16(exp_coeff[0]), Word16(exp_coeff[2])).0;
    let exp1 = add(&mut ctx, Word16(e1_t), Word16(-1)).0;
    let l_tmp2 = l_mult(&mut ctx, Word16(g_coeff[4]), Word16(g_coeff[4]));
    let e2_t = add(&mut ctx, Word16(exp_coeff[4]), Word16(exp_coeff[4])).0;
    let exp2 = add(&mut ctx, Word16(e2_t), Word16(1)).0;
    let (l_tmp, exp) = if sub(&mut ctx, Word16(exp1), Word16(exp2)).0 > 0 {
        let shift = sub(&mut ctx, Word16(exp1), Word16(exp2)).0;
        let l_tmp1s = l_shr(&mut ctx, l_tmp1, shift);
        (l_sub(&mut ctx, l_tmp1s, l_tmp2), exp2)
    } else {
        let shift = sub(&mut ctx, Word16(exp2), Word16(exp1)).0;
        let l_tmp2s = l_shr(&mut ctx, l_tmp2, shift);
        (l_sub(&mut ctx, l_tmp1, l_tmp2s), exp1)
    };
    let sft = norm_l(l_tmp);
    let l_tmpn = l_shl(&mut ctx, l_tmp, sft);
    let denom = extract_h(l_tmpn).0;
    let expt = add(&mut ctx, Word16(exp), Word16(sft)).0;
    let exp_denom = sub(&mut ctx, Word16(expt), Word16(16)).0;
    let mut inv_denom = div_s(Word16(16384), Word16(denom)).0;
    inv_denom = negate(&mut ctx, Word16(inv_denom)).0;
    let exp_inv_denom = sub(&mut ctx, Word16(14 + 15), Word16(exp_denom)).0;

    let l_tmp1 = l_mult(&mut ctx, Word16(g_coeff[2]), Word16(g_coeff[1]));
    let exp1 = add(&mut ctx, Word16(exp_coeff[2]), Word16(exp_coeff[1])).0;
    let l_tmp2 = l_mult(&mut ctx, Word16(g_coeff[3]), Word16(g_coeff[4]));
    let e2_t = add(&mut ctx, Word16(exp_coeff[3]), Word16(exp_coeff[4])).0;
    let exp2 = add(&mut ctx, Word16(e2_t), Word16(1)).0;
    let (l_tmp, exp) = if sub(&mut ctx, Word16(exp1), Word16(exp2)).0 > 0 {
        let d = sub(&mut ctx, Word16(exp1), Word16(exp2)).0;
        let d1 = add(&mut ctx, Word16(d), Word16(1)).0;
        let l_tmp1s = l_shr(&mut ctx, l_tmp1, d1);
        let l_tmp2s = l_shr(&mut ctx, l_tmp2, 1);
        let e = sub(&mut ctx, Word16(exp2), Word16(1)).0;
        (l_sub(&mut ctx, l_tmp1s, l_tmp2s), e)
    } else {
        let d = sub(&mut ctx, Word16(exp2), Word16(exp1)).0;
        let d1 = add(&mut ctx, Word16(d), Word16(1)).0;
        let l_tmp1s = l_shr(&mut ctx, l_tmp1, 1);
        let l_tmp2s = l_shr(&mut ctx, l_tmp2, d1);
        let e = sub(&mut ctx, Word16(exp1), Word16(1)).0;
        (l_sub(&mut ctx, l_tmp1s, l_tmp2s), e)
    };
    let sft = norm_l(l_tmp);
    let l_tmpn = l_shl(&mut ctx, l_tmp, sft);
    let nume = extract_h(l_tmpn).0;
    let expt = add(&mut ctx, Word16(exp), Word16(sft)).0;
    let exp_nume = sub(&mut ctx, Word16(expt), Word16(16)).0;
    let expt2 = add(&mut ctx, Word16(exp_nume), Word16(exp_inv_denom)).0;
    let sft2 = sub(&mut ctx, Word16(expt2), Word16(9 + 16 - 1)).0;
    let l_nm = l_mult(&mut ctx, Word16(nume), Word16(inv_denom));
    let l_acc = l_shr(&mut ctx, l_nm, sft2);
    let mut best_gain0 = extract_h(l_acc).0;
    if tameflag == 1 && sub(&mut ctx, Word16(best_gain0), Word16(GPCLIP2)).0 > 0 {
        best_gain0 = GPCLIP2;
    }

    let l_tmp1 = l_mult(&mut ctx, Word16(g_coeff[0]), Word16(g_coeff[3]));
    let exp1 = add(&mut ctx, Word16(exp_coeff[0]), Word16(exp_coeff[3])).0;
    let l_tmp2 = l_mult(&mut ctx, Word16(g_coeff[1]), Word16(g_coeff[4]));
    let e2_t = add(&mut ctx, Word16(exp_coeff[1]), Word16(exp_coeff[4])).0;
    let exp2 = add(&mut ctx, Word16(e2_t), Word16(1)).0;
    let (l_tmp, exp) = if sub(&mut ctx, Word16(exp1), Word16(exp2)).0 > 0 {
        let d = sub(&mut ctx, Word16(exp1), Word16(exp2)).0;
        let d1 = add(&mut ctx, Word16(d), Word16(1)).0;
        let l_tmp1s = l_shr(&mut ctx, l_tmp1, d1);
        let l_tmp2s = l_shr(&mut ctx, l_tmp2, 1);
        let e = sub(&mut ctx, Word16(exp2), Word16(1)).0;
        (l_sub(&mut ctx, l_tmp1s, l_tmp2s), e)
    } else {
        let d = sub(&mut ctx, Word16(exp2), Word16(exp1)).0;
        let d1 = add(&mut ctx, Word16(d), Word16(1)).0;
        let l_tmp1s = l_shr(&mut ctx, l_tmp1, 1);
        let l_tmp2s = l_shr(&mut ctx, l_tmp2, d1);
        let e = sub(&mut ctx, Word16(exp1), Word16(1)).0;
        (l_sub(&mut ctx, l_tmp1s, l_tmp2s), e)
    };
    let sft = norm_l(l_tmp);
    let l_tmpn = l_shl(&mut ctx, l_tmp, sft);
    let nume = extract_h(l_tmpn).0;
    let expt = add(&mut ctx, Word16(exp), Word16(sft)).0;
    let exp_nume = sub(&mut ctx, Word16(expt), Word16(16)).0;
    let expt2 = add(&mut ctx, Word16(exp_nume), Word16(exp_inv_denom)).0;
    let sft2 = sub(&mut ctx, Word16(expt2), Word16(2 + 16 - 1)).0;
    let l_nm = l_mult(&mut ctx, Word16(nume), Word16(inv_denom));
    let l_acc = l_shr(&mut ctx, l_nm, sft2);
    let best_gain1 = extract_h(l_acc).0;
    let best_gain = [best_gain0, best_gain1];

    let gcode0_org = if sub(&mut ctx, Word16(exp_gcode0), Word16(4)).0 >= 0 {
        let sh = sub(&mut ctx, Word16(exp_gcode0), Word16(4)).0;
        shr(&mut ctx, Word16(gcode0), sh).0
    } else {
        let mut l_acc = l_deposit_l(Word16(gcode0));
        let sh = sub(&mut ctx, Word16(4 + 16), Word16(exp_gcode0)).0;
        l_acc = l_shl(&mut ctx, l_acc, sh);
        extract_h(l_acc).0
    };

    let mut cand1 = 0usize;
    let mut cand2 = 0usize;
    gbk_presel(&best_gain, &mut cand1, &mut cand2, gcode0_org);

    let mut exp_min = [0i16; 5];
    exp_min[0] = add(&mut ctx, Word16(exp_coeff[0]), Word16(13)).0;
    exp_min[1] = add(&mut ctx, Word16(exp_coeff[1]), Word16(14)).0;
    let eg2 = shl(&mut ctx, Word16(exp_gcode0), 1);
    let eg2m21 = sub(&mut ctx, eg2, Word16(21));
    exp_min[2] = add(&mut ctx, Word16(exp_coeff[2]), eg2m21).0;
    let egm3 = sub(&mut ctx, Word16(exp_gcode0), Word16(3));
    exp_min[3] = add(&mut ctx, Word16(exp_coeff[3]), egm3).0;
    let egm4 = sub(&mut ctx, Word16(exp_gcode0), Word16(4));
    exp_min[4] = add(&mut ctx, Word16(exp_coeff[4]), egm4).0;
    let mut e_min = exp_min[0];
    for &e in &exp_min[1..] {
        if sub(&mut ctx, Word16(e), Word16(e_min)).0 < 0 {
            e_min = e;
        }
    }

    let mut coeff = [0i16; 5];
    let mut coeff_lsf = [0i16; 5];
    for i in 0..5 {
        let j = sub(&mut ctx, Word16(exp_min[i]), Word16(e_min)).0;
        let mut l_tmp = l_deposit_h(Word16(g_coeff[i]));
        l_tmp = l_shr(&mut ctx, l_tmp, j);
        let (ch, cl) = l_extract(l_tmp);
        coeff[i] = ch.0;
        coeff_lsf[i] = cl.0;
    }

    QuantizePrep {
        gcode0,
        exp_gcode0,
        cand1,
        cand2,
        coeff,
        coeff_lsf,
    }
}
