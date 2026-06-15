//! Provenance: Gain prediction/quantization adapted from ITU G.729 gain predictor and quantizer flow.
//! Q-format: Pitch/code gains and predictors use Q14/Q15 arithmetic with 32-bit intermediates.

use crate::codec::state::EncoderState;
use crate::constants::{L_INTER10, L_SUBFR, L_THRESH_ERR};
use crate::dsp::arith::{add, sub};
use crate::dsp::arith32::{l_add, l_sub};
use crate::dsp::oper32::{l_extract, mpy_32_16};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::TAB_ZONE;

pub(crate) fn test_excitation_error(state: &EncoderState, t0: i16, t0_frac: i16) -> i16 {
    let mut ctx = DspContext::default();
    let t1 = if t0_frac > 0 {
        add(&mut ctx, Word16(t0), Word16(1)).0
    } else {
        t0
    };

    let mut i = sub(&mut ctx, Word16(t1), Word16((L_SUBFR + L_INTER10) as i16)).0;
    if i < 0 {
        i = 0;
    }
    let zone1 = TAB_ZONE[i as usize] as usize;
    i = add(&mut ctx, Word16(t1), Word16((L_INTER10 - 2) as i16)).0;
    let zone2 = TAB_ZONE[i as usize] as usize;

    let mut l_maxloc = Word32(-1);
    for zi in (zone1..=zone2).rev() {
        let l_acc = l_sub(&mut ctx, Word32(state.l_exc_err[zi]), l_maxloc);
        if l_acc.0 > 0 {
            l_maxloc = Word32(state.l_exc_err[zi]);
        }
    }
    if l_sub(&mut ctx, l_maxloc, Word32(L_THRESH_ERR)).0 > 0 {
        1
    } else {
        0
    }
}

pub(crate) fn update_excitation_error(state: &mut EncoderState, gain_pit: i16, t0: i16) {
    let mut ctx = DspContext::default();
    let mut l_worst = Word32(-1);
    let n = sub(&mut ctx, Word16(t0), Word16(L_SUBFR as i16)).0;

    if n < 0 {
        let mut l_temp = Word32(state.l_exc_err[0]);
        for _ in 0..2 {
            let (hi, lo) = l_extract(l_temp);
            l_temp = mpy_32_16(hi, lo, Word16(gain_pit));
            l_temp = l_shl(&mut ctx, l_temp, 1);
            l_temp = l_add(&mut ctx, Word32(0x0000_4000), l_temp);
            if l_sub(&mut ctx, l_temp, l_worst).0 > 0 {
                l_worst = l_temp;
            }
        }
    } else {
        let zone1 = TAB_ZONE[n as usize] as usize;
        let i = sub(&mut ctx, Word16(t0), Word16(1)).0;
        let zone2 = TAB_ZONE[i as usize] as usize;
        for zi in zone1..=zone2 {
            let (hi, lo) = l_extract(Word32(state.l_exc_err[zi]));
            let mut l_temp = mpy_32_16(hi, lo, Word16(gain_pit));
            l_temp = l_shl(&mut ctx, l_temp, 1);
            l_temp = l_add(&mut ctx, Word32(0x0000_4000), l_temp);
            if l_sub(&mut ctx, l_temp, l_worst).0 > 0 {
                l_worst = l_temp;
            }
        }
    }

    for i in (1..4).rev() {
        state.l_exc_err[i] = state.l_exc_err[i - 1];
    }
    state.l_exc_err[0] = l_worst.0;
}
