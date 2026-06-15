#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::manual_clamp)]
#![allow(clippy::needless_range_loop)]
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

//! Decoder frame helpers extracted from the decode pipeline.

use crate::codec::decode_sub_helpers::{
    random_itu, save_sid_energy_if_good_frame, slide_excitation_history,
};
use crate::codec::state::{DecoderState, EXC_OFFSET};
use crate::constants::{L_FRAME, L_SUBFR, M, MP1, PRM_SIZE, SHARPMAX, SHARPMIN};
use crate::dsp::arith::{add, mult, round};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::shift::{l_shl, shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::filter::syn_filt_with_ctx;
use crate::fixed_cb::decod_acelp;

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn decod_ld8a(
    state: &mut DecoderState,
    parm: &[i16; PRM_SIZE + 1],
    synth: &mut [i16; L_FRAME],
    a_t: &mut [i16; MP1 * 2],
    t2: &mut [i16; 2],
    sid_energy_out: Option<(&mut i16, &mut i16)>,
) {
    let bfi = parm[0];
    let mut prm_ptr = 1usize;
    let erase = if bfi != 0 || state.bad_lsf != 0 { 1 } else { 0 };

    let mut lsp_new = [0i16; M];
    crate::lsp_quant::decode::d_lsp(state, &parm[prm_ptr..], &mut lsp_new, erase);
    prm_ptr += 2;

    crate::lp::interp::int_qlpc(&state.lsp_old, &lsp_new, a_t);
    state.lsp_old = lsp_new;

    let mut az_idx = 0usize;
    let mut t0_var = state.old_t0;
    for sf in 0..2 {
        let i_subfr = sf * L_SUBFR;
        let index = parm[prm_ptr];
        prm_ptr += 1;

        let parity_err = if sf == 0 {
            let p = parm[prm_ptr];
            prm_ptr += 1;
            Some(p)
        } else {
            None
        };
        let (t0, t0_frac) = crate::codec::erasure::decode_lag_or_erasure(
            state,
            bfi,
            sf,
            index,
            parity_err,
            &mut t0_var,
        );
        t2[sf] = t0;

        let exc_idx = EXC_OFFSET + i_subfr;
        crate::pitch::pred_lt3::interpolate_excitation_decode(
            &mut state.old_exc,
            exc_idx,
            t0,
            t0_frac,
        );

        let (cb_index, cb_sign) = if bfi != 0 {
            (
                random_itu(&mut state.rand_seed) & 0x1fff,
                random_itu(&mut state.rand_seed) & 0x000f,
            )
        } else {
            (parm[prm_ptr], parm[prm_ptr + 1])
        };
        let code_w = decod_acelp(w(cb_sign), w(cb_index));
        prm_ptr += 2;

        let mut code = [0i16; L_SUBFR];
        for i in 0..L_SUBFR {
            code[i] = code_w[i].0;
        }

        let mut ctx = DspContext::default();
        let j = shl(&mut ctx, w(state.sharp), 1);
        if t0 < L_SUBFR as i16 {
            for i in t0 as usize..L_SUBFR {
                let m = mult(&mut ctx, w(code[i - t0 as usize]), j);
                code[i] = add(&mut ctx, w(code[i]), m).0;
            }
        }

        let gain_index = parm[prm_ptr];
        prm_ptr += 1;
        let mut gain_pit = state.gain_pitch;
        let mut gain_cod = state.gain_code;
        crate::gain::decode::decode_gain(
            state,
            gain_index,
            &code,
            bfi,
            &mut gain_pit,
            &mut gain_cod,
        );
        state.gain_pitch = gain_pit;
        state.gain_code = gain_cod;

        state.sharp = gain_pit;
        if state.sharp > SHARPMAX {
            state.sharp = SHARPMAX;
        }
        if state.sharp < SHARPMIN {
            state.sharp = SHARPMIN;
        }

        for i in 0..L_SUBFR {
            let idx = exc_idx + i;
            let mut l_temp = l_mult(&mut ctx, w(state.old_exc[idx]), w(gain_pit));
            l_temp = l_mac(&mut ctx, l_temp, w(code[i]), w(gain_cod));
            l_temp = l_shl(&mut ctx, l_temp, 1);
            state.old_exc[idx] = round(&mut ctx, l_temp).0;
        }

        let mut a = [Word16(0); MP1];
        for i in 0..MP1 {
            a[i] = w(a_t[az_idx + i]);
        }
        let mut x = [Word16(0); L_SUBFR];
        for i in 0..L_SUBFR {
            x[i] = w(state.old_exc[exc_idx + i]);
        }
        let mut y = [Word16(0); L_SUBFR];
        let mut mem = [Word16(0); M];
        for i in 0..M {
            mem[i] = w(state.mem_syn[i]);
        }

        let mut syn_ctx = DspContext::default();
        syn_ctx.overflow = false;
        syn_filt_with_ctx(&mut syn_ctx, &a, &x, &mut y, L_SUBFR, &mut mem, false);
        if syn_ctx.overflow {
            for v in &mut state.old_exc {
                *v = shr(&mut ctx, w(*v), 2).0;
            }
            for i in 0..L_SUBFR {
                x[i] = w(state.old_exc[exc_idx + i]);
            }
            syn_filt_with_ctx(&mut syn_ctx, &a, &x, &mut y, L_SUBFR, &mut mem, true);
            for i in 0..M {
                state.mem_syn[i] = mem[i].0;
            }
        } else {
            for i in 0..M {
                state.mem_syn[i] = y[L_SUBFR - M + i].0;
            }
        }
        for i in 0..L_SUBFR {
            synth[i_subfr + i] = y[i].0;
        }

        az_idx += MP1;
    }

    save_sid_energy_if_good_frame(state, bfi, sid_energy_out);
    slide_excitation_history(state);
}
