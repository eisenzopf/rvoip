//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::codec::state::DecoderState;
use crate::constants::{L_SUBFR, NCODE2_B};
use crate::dsp::arith::{add, extract_h, extract_l, mult, sub};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mult};
use crate::dsp::oper32::l_comp;
use crate::dsp::shift::{l_shl, l_shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::gain::predict::predict_decoder_gain;
use crate::tables::annexa::{IMAP1, IMAP2, gbk1, gbk2};

fn gain_update_decode(past_qua_en: &mut [i16; 4], l_gbk12: Word32) {
    let mut ctx = DspContext::default();
    for i in (1..4).rev() {
        past_qua_en[i] = past_qua_en[i - 1];
    }

    let mut exp = Word16(0);
    let mut frac = Word16(0);
    crate::dsp::div::Log2(l_gbk12, &mut exp, &mut frac);
    let l_acc = l_comp(sub(&mut ctx, exp, Word16(13)), frac);
    let tmp = extract_h(l_shl(&mut ctx, l_acc, 13));
    past_qua_en[0] = mult(&mut ctx, tmp, Word16(24660)).0;
}

fn gain_update_erasure(past_qua_en: &mut [i16; 4]) {
    let mut ctx = DspContext::default();
    let mut l_tmp = Word32(0);
    for &v in past_qua_en.iter() {
        l_tmp = l_add(&mut ctx, l_tmp, l_deposit_l(Word16(v)));
    }
    let mut av_pred_en = extract_l(l_shr(&mut ctx, l_tmp, 2)).0;
    av_pred_en = sub(&mut ctx, Word16(av_pred_en), Word16(4096)).0;
    if sub(&mut ctx, Word16(av_pred_en), Word16(-14336)).0 < 0 {
        av_pred_en = -14336;
    }
    for i in (1..4).rev() {
        past_qua_en[i] = past_qua_en[i - 1];
    }
    past_qua_en[0] = av_pred_en;
}

pub(crate) fn decode_gain(
    state: &mut DecoderState,
    index: i16,
    code: &[i16; L_SUBFR],
    bfi: i16,
    gain_pit: &mut i16,
    gain_cod: &mut i16,
) {
    let mut ctx = DspContext::default();

    if bfi != 0 {
        *gain_pit = mult(&mut ctx, Word16(*gain_pit), Word16(29491)).0;
        if sub(&mut ctx, Word16(*gain_pit), Word16(29491)).0 > 0 {
            *gain_pit = 29491;
        }
        *gain_cod = mult(&mut ctx, Word16(*gain_cod), Word16(32111)).0;
        gain_update_erasure(&mut state.past_qua_en);
        return;
    }

    let index1 = IMAP1[(index >> NCODE2_B) as usize] as usize;
    let index2 = IMAP2[(index & ((1 << NCODE2_B) - 1)) as usize] as usize;
    *gain_pit = add(&mut ctx, Word16(gbk1(index1, 0)), Word16(gbk2(index2, 0))).0;

    let mut gcode0 = 0i16;
    let mut exp_gcode0 = 0i16;
    predict_decoder_gain(&state.past_qua_en, code, &mut gcode0, &mut exp_gcode0);

    let l_acc = l_deposit_l(Word16(gbk1(index1, 1)));
    let l_accb = l_deposit_l(Word16(gbk2(index2, 1)));
    let l_gbk12 = l_add(&mut ctx, l_acc, l_accb);
    let tmp = extract_l(l_shr(&mut ctx, l_gbk12, 1));
    let mut l_acc2 = l_mult(&mut ctx, tmp, Word16(gcode0));
    let shift = 4 - exp_gcode0;
    l_acc2 = l_shl(&mut ctx, l_acc2, shift);
    *gain_cod = extract_h(l_acc2).0;

    gain_update_decode(&mut state.past_qua_en, l_gbk12);
}
