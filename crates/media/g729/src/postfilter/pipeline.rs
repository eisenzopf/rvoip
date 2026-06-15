#![allow(clippy::needless_range_loop)]
//! Provenance: Post-filter stages adapted from ITU G.729 Annex A formant/pitch/AGC post-processing.
//! Q-format: Post-filter coefficients and synthesis states use Q12/Q14/Q15 fixed-point paths.

use crate::codec::state::DecoderState;
use crate::constants::{GAMMA1_PST, GAMMA2_PST, L_FRAME, L_H, L_SUBFR, M, MP1, MU, PIT_MAX};
use crate::dsp::arith::{extract_h, mult};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::shift::shr;
use crate::dsp::types::{DspContext, Word16};
use crate::filter::resid::residu_with_ctx;
use crate::filter::syn::syn_filt_with_ctx;
use crate::postfilter::agc::agc;
use crate::postfilter::formant::formant_weight;
use crate::postfilter::pitch_pf::pitch_post_filter;

pub(crate) fn post_filter(
    state: &mut DecoderState,
    az_4: &[i16; MP1 * 2],
    t: &[i16; 2],
    synth: &mut [i16; L_FRAME],
    vad: i16,
) {
    let mut ap3 = [0i16; MP1];
    let mut ap4 = [0i16; MP1];
    let mut res2_pst = [0i16; L_SUBFR];
    let mut syn_pst = [0i16; L_FRAME];
    let mut h = [0i16; L_H];
    let mut ctx = DspContext::default();

    for sf in 0..2 {
        let i_subfr = sf * L_SUBFR;
        let mut t0_min = t[sf] - 3;
        let mut t0_max = t0_min + 6;
        if t0_max > PIT_MAX {
            t0_max = PIT_MAX;
            t0_min = t0_max - 6;
        }

        let mut az = [0i16; MP1];
        az.copy_from_slice(&az_4[sf * MP1..sf * MP1 + MP1]);
        formant_weight(&az, GAMMA2_PST, &mut ap3);
        formant_weight(&az, GAMMA1_PST, &mut ap4);

        let mut ap3w = [Word16(0); MP1];
        for i in 0..MP1 {
            ap3w[i] = Word16(ap3[i]);
        }
        let mut xhist = [Word16(0); M + L_SUBFR];
        for i in 0..M {
            xhist[i] = Word16(state.synth_buf[i_subfr + i]);
        }
        for i in 0..L_SUBFR {
            xhist[M + i] = Word16(synth[i_subfr + i]);
        }
        let mut y = [Word16(0); L_SUBFR];
        residu_with_ctx(&mut ctx, &ap3w, &xhist, &mut y, L_SUBFR);
        for i in 0..L_SUBFR {
            state.res2_buf[PIT_MAX as usize + i] = y[i].0;
            state.scal_res2_buf[PIT_MAX as usize + i] = shr(&mut ctx, y[i], 2).0;
        }

        if vad == 1 {
            pitch_post_filter(
                &state.res2_buf,
                &state.scal_res2_buf,
                t0_min,
                t0_max,
                &mut res2_pst,
            );
        } else {
            res2_pst.copy_from_slice(&state.res2_buf[PIT_MAX as usize..PIT_MAX as usize + L_SUBFR]);
        }

        h[..MP1].copy_from_slice(&ap3);
        for v in h.iter_mut().skip(M + 1) {
            *v = 0;
        }
        let mut ap4w = [Word16(0); MP1];
        for i in 0..MP1 {
            ap4w[i] = Word16(ap4[i]);
        }
        let mut hw = [Word16(0); L_H];
        for i in 0..L_H {
            hw[i] = Word16(h[i]);
        }
        let mut mem = [Word16(0); M];
        for i in 0..M {
            mem[i] = hw[M + 1 + i];
        }
        let hw_in = hw;
        syn_filt_with_ctx(&mut ctx, &ap4w, &hw_in, &mut hw, L_H, &mut mem, false);
        for i in 0..L_H {
            h[i] = hw[i].0;
        }

        let mut l_tmp = l_mult(&mut ctx, Word16(h[0]), Word16(h[0]));
        for i in 1..L_H {
            l_tmp = l_mac(&mut ctx, l_tmp, Word16(h[i]), Word16(h[i]));
        }
        let temp1 = extract_h(l_tmp).0;

        l_tmp = l_mult(&mut ctx, Word16(h[0]), Word16(h[1]));
        for i in 1..(L_H - 1) {
            l_tmp = l_mac(&mut ctx, l_tmp, Word16(h[i]), Word16(h[i + 1]));
        }
        let mut temp2 = extract_h(l_tmp).0;
        if temp2 <= 0 {
            temp2 = 0;
        } else {
            temp2 = mult(&mut ctx, Word16(temp2), Word16(MU)).0;
            temp2 = crate::dsp::div::div_s(Word16(temp2), Word16(temp1)).0;
        }

        crate::filter::preemph::preemphasis_with_mem(&mut res2_pst, temp2, &mut state.mem_pre);

        let mut xw = [Word16(0); L_SUBFR];
        for i in 0..L_SUBFR {
            xw[i] = Word16(res2_pst[i]);
        }
        let mut yw = [Word16(0); L_SUBFR];
        let mut mem_syn = [Word16(0); M];
        for i in 0..M {
            mem_syn[i] = Word16(state.mem_syn_pst[i]);
        }
        syn_filt_with_ctx(&mut ctx, &ap4w, &xw, &mut yw, L_SUBFR, &mut mem_syn, true);
        for i in 0..M {
            state.mem_syn_pst[i] = mem_syn[i].0;
        }
        let mut sf_out = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            sf_out[i] = yw[i].0;
        }

        let mut sf_in = [0i16; L_SUBFR];
        sf_in.copy_from_slice(&synth[i_subfr..i_subfr + L_SUBFR]);
        agc(state, &sf_in, &mut sf_out);
        syn_pst[i_subfr..i_subfr + L_SUBFR].copy_from_slice(&sf_out);

        for i in 0..PIT_MAX as usize {
            state.res2_buf[i] = state.res2_buf[i + L_SUBFR];
            state.scal_res2_buf[i] = state.scal_res2_buf[i + L_SUBFR];
        }
    }

    state.synth_buf[..M].copy_from_slice(&synth[L_FRAME - M..L_FRAME]);
    synth.copy_from_slice(&syn_pst);
}
