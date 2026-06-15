//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::constants::L_SUBFR;
use crate::dsp::arith::{extract_h, mult, sub};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::div::{Log2, Pow2};
use crate::dsp::oper32::{l_comp, l_extract, mpy_32_16};
use crate::dsp::shift::{l_shl, l_shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::PRED;

pub(crate) fn predict_encoder_gain(
    past_qua_en: &[i16; 4],
    code: &[i16; L_SUBFR],
    gcode0: &mut i16,
    exp_gcode0: &mut i16,
) {
    let mut ctx = DspContext::default();
    let mut l_tmp = Word32(0);
    for &c in code {
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(c), Word16(c));
    }

    let mut exp = Word16(0);
    let mut frac = Word16(0);
    Log2(l_tmp, &mut exp, &mut frac);
    l_tmp = mpy_32_16(exp, frac, Word16(-24660));
    l_tmp = l_mac(&mut ctx, l_tmp, Word16(32588), Word16(32));
    l_tmp = l_shl(&mut ctx, l_tmp, 10);
    for i in 0..4 {
        l_tmp = l_mac(&mut ctx, l_tmp, Word16(PRED[i]), Word16(past_qua_en[i]));
    }
    *gcode0 = extract_h(l_tmp).0;

    l_tmp = l_mult(&mut ctx, Word16(*gcode0), Word16(5439));
    l_tmp = l_shr(&mut ctx, l_tmp, 8);
    let (exp2, frac2) = l_extract(l_tmp);
    *gcode0 = crate::dsp::arith::extract_l(Pow2(Word16(14), frac2)).0;
    *exp_gcode0 = sub(&mut ctx, Word16(14), exp2).0;
}

pub(crate) fn update_encoder_gain_history(past_qua_en: &mut [i16; 4], l_gbk12: Word32) {
    let mut ctx = DspContext::default();
    for i in (1..4).rev() {
        past_qua_en[i] = past_qua_en[i - 1];
    }
    let mut exp = Word16(0);
    let mut frac = Word16(0);
    Log2(l_gbk12, &mut exp, &mut frac);
    let l_acc = l_comp(sub(&mut ctx, exp, Word16(13)), frac);
    let tmp = extract_h(l_shl(&mut ctx, l_acc, 13));
    past_qua_en[0] = mult(&mut ctx, tmp, Word16(24660)).0;
}

pub(crate) fn predict_decoder_gain(
    past_qua_en: &[i16; 4],
    code: &[i16; L_SUBFR],
    gcode0: &mut i16,
    exp_gcode0: &mut i16,
) {
    predict_encoder_gain(past_qua_en, code, gcode0, exp_gcode0);
}
