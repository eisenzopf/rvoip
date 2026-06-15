#![allow(clippy::needless_range_loop)]
//! Provenance: Post-filter stages adapted from ITU G.729 Annex A formant/pitch/AGC post-processing.
//! Q-format: Post-filter coefficients and synthesis states use Q12/Q14/Q15 fixed-point paths.

use crate::codec::state::DecoderState;
use crate::constants::{AGC_FAC, AGC_FAC1, L_SUBFR};
use crate::dsp::arith::{add, extract_h, mult, round, sub};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::div::{Inv_sqrt, div_s};
use crate::dsp::shift::{l_shl, norm_l, shr};
use crate::dsp::types::{DspContext, Word16, Word32};

pub(crate) fn agc(state: &mut DecoderState, sig_in: &[i16; L_SUBFR], sig_out: &mut [i16; L_SUBFR]) {
    let mut ctx = DspContext::default();
    let mut signal = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        signal[i] = shr(&mut ctx, Word16(sig_out[i]), 2).0;
    }
    let mut s = Word32(0);
    for &v in &signal {
        s = l_mac(&mut ctx, s, Word16(v), Word16(v));
    }
    if s.0 == 0 {
        state.past_gain = 0;
        return;
    }
    let mut exp = sub(&mut ctx, Word16(norm_l(s)), Word16(1)).0;
    let gain_out = {
        let t = l_shl(&mut ctx, s, exp);
        round(&mut ctx, t)
    };

    for i in 0..L_SUBFR {
        signal[i] = shr(&mut ctx, Word16(sig_in[i]), 2).0;
    }
    s = Word32(0);
    for &v in &signal {
        s = l_mac(&mut ctx, s, Word16(v), Word16(v));
    }

    let g0 = if s.0 == 0 {
        0
    } else {
        let i = norm_l(s);
        let gain_in = {
            let t = l_shl(&mut ctx, s, i);
            round(&mut ctx, t)
        };
        exp = sub(&mut ctx, Word16(exp), Word16(i)).0;

        let mut ratio = crate::dsp::arith32::l_deposit_l(div_s(gain_out, gain_in));
        ratio = l_shl(&mut ctx, ratio, 7);
        ratio = crate::dsp::shift::l_shr(&mut ctx, ratio, exp);

        let inv = Inv_sqrt(ratio);
        let i_q12 = {
            let t = l_shl(&mut ctx, inv, 9);
            round(&mut ctx, t)
        };
        mult(&mut ctx, i_q12, Word16(AGC_FAC1)).0
    };

    let mut gain = state.past_gain;
    for i in 0..L_SUBFR {
        gain = mult(&mut ctx, Word16(gain), Word16(AGC_FAC)).0;
        gain = add(&mut ctx, Word16(gain), Word16(g0)).0;
        let t = l_mult(&mut ctx, Word16(sig_out[i]), Word16(gain));
        let t = l_shl(&mut ctx, t, 3);
        sig_out[i] = extract_h(t).0;
    }
    state.past_gain = gain;
}
