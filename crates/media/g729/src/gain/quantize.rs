//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::codec::state::EncoderState;
use crate::constants::{GP0999, L_SUBFR, NCAN1, NCAN2, NCODE2};
use crate::dsp::arith::{add, extract_h, extract_l, mult, negate};
use crate::dsp::arith32::{l_add, l_deposit_l, l_mult, l_sub};
use crate::dsp::shift::{l_shl, l_shr};
use crate::dsp::types::{DspContext, MAX_32, Word16, Word32};
use crate::gain::predict::update_encoder_gain_history;
use crate::gain::quantize_prepare::prepare_quantization;
use crate::tables::annexa::{MAP1, MAP2, gbk1, gbk2};

#[allow(clippy::too_many_arguments)]
pub(crate) fn quantize_gain(
    state: &mut EncoderState,
    code: &[i16; L_SUBFR],
    g_coeff: &[i16; 5],
    exp_coeff: &[i16; 5],
    gain_pit: &mut i16,
    gain_cod: &mut i16,
    tameflag: i16,
) -> i16 {
    let mut ctx = DspContext::default();
    let prep = prepare_quantization(state, code, g_coeff, exp_coeff, tameflag);

    let mut l_dist_min = Word32(MAX_32);
    let mut index1 = prep.cand1;
    let mut index2 = prep.cand2;

    for i in 0..NCAN1 {
        for j in 0..NCAN2 {
            let g_pitch = add(
                &mut ctx,
                Word16(gbk1(prep.cand1 + i, 0)),
                Word16(gbk2(prep.cand2 + j, 0)),
            )
            .0;
            if tameflag == 1 && g_pitch >= GP0999 {
                continue;
            }
            let l_acc = l_deposit_l(Word16(gbk1(prep.cand1 + i, 1)));
            let l_accb = l_deposit_l(Word16(gbk2(prep.cand2 + j, 1)));
            let l_tmp = l_add(&mut ctx, l_acc, l_accb);
            let tmp = extract_l(l_shr(&mut ctx, l_tmp, 1)).0;

            let g_code = mult(&mut ctx, Word16(prep.gcode0), Word16(tmp)).0;
            let g2_pitch = mult(&mut ctx, Word16(g_pitch), Word16(g_pitch)).0;
            let g2_code = mult(&mut ctx, Word16(g_code), Word16(g_code)).0;
            let g_pit_cod = mult(&mut ctx, Word16(g_code), Word16(g_pitch)).0;

            let mut l_tmp = crate::dsp::oper32::mpy_32_16(
                Word16(prep.coeff[0]),
                Word16(prep.coeff_lsf[0]),
                Word16(g2_pitch),
            );
            l_tmp = l_add(
                &mut ctx,
                l_tmp,
                crate::dsp::oper32::mpy_32_16(
                    Word16(prep.coeff[1]),
                    Word16(prep.coeff_lsf[1]),
                    Word16(g_pitch),
                ),
            );
            l_tmp = l_add(
                &mut ctx,
                l_tmp,
                crate::dsp::oper32::mpy_32_16(
                    Word16(prep.coeff[2]),
                    Word16(prep.coeff_lsf[2]),
                    Word16(g2_code),
                ),
            );
            l_tmp = l_add(
                &mut ctx,
                l_tmp,
                crate::dsp::oper32::mpy_32_16(
                    Word16(prep.coeff[3]),
                    Word16(prep.coeff_lsf[3]),
                    Word16(g_code),
                ),
            );
            l_tmp = l_add(
                &mut ctx,
                l_tmp,
                crate::dsp::oper32::mpy_32_16(
                    Word16(prep.coeff[4]),
                    Word16(prep.coeff_lsf[4]),
                    Word16(g_pit_cod),
                ),
            );
            if l_sub(&mut ctx, l_tmp, l_dist_min).0 < 0 {
                l_dist_min = l_tmp;
                index1 = prep.cand1 + i;
                index2 = prep.cand2 + j;
            }
        }
    }

    *gain_pit = add(&mut ctx, Word16(gbk1(index1, 0)), Word16(gbk2(index2, 0))).0;
    let l_acc = l_deposit_l(Word16(gbk1(index1, 1)));
    let l_accb = l_deposit_l(Word16(gbk2(index2, 1)));
    let l_gbk12 = l_add(&mut ctx, l_acc, l_accb);
    let tmp = extract_l(l_shr(&mut ctx, l_gbk12, 1)).0;
    let mut l_acc2 = l_mult(&mut ctx, Word16(tmp), Word16(prep.gcode0));
    let neg_exp = negate(&mut ctx, Word16(prep.exp_gcode0));
    let sh = add(&mut ctx, neg_exp, Word16(-12 - 1 + 1 + 16)).0;
    l_acc2 = l_shl(&mut ctx, l_acc2, sh);
    *gain_cod = extract_h(l_acc2).0;
    update_encoder_gain_history(&mut state.past_qua_en, l_gbk12);

    add(
        &mut ctx,
        Word16(MAP1[index1] * NCODE2 as i16),
        Word16(MAP2[index2]),
    )
    .0
}
