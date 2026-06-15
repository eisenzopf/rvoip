#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::needless_range_loop)]
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::annex_b::cng::CngState;
use crate::api::FrameType;
use crate::codec::decode_annexb_bits::{bits2prm_ld8k, bits2prm_sid};
use crate::codec::decode_sub::decod_ld8a;
use crate::codec::decode_sub_helpers::slide_excitation_history;
use crate::codec::state::{DecoderState, EXC_OFFSET};
use crate::constants::{BIT_0, L_FRAME, L_SUBFR, M, MP1, PRM_SIZE, SHARPMIN};
use crate::dsp::shift::shr;
use crate::dsp::types::{DspContext, Word16};
use crate::filter::syn_filt_with_ctx;
use crate::pitch::check_parity_pitch;

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn decode_annex_b_frame_words_impl(
    state: &mut DecoderState,
    cng: &mut CngState,
    frame_type: FrameType,
    words: &[u16],
    bfi_in: i16,
) -> [i16; L_FRAME] {
    let bfi = if bfi_in != 0 { 1 } else { 0 };
    let mut ftyp = match frame_type {
        FrameType::Speech => 1,
        FrameType::Sid => 2,
        FrameType::NoData => 0,
    };

    let mut force_parity_error = false;
    if bfi != 0 {
        if cng.past_ftyp == 1 {
            ftyp = 1;
            force_parity_error = true;
        } else {
            ftyp = 0;
        }
    }

    let mut synth = [0i16; L_FRAME];
    let mut az_dec = [0i16; MP1 * 2];
    let mut t2 = [state.old_t0; 2];

    if ftyp == 1 {
        cng.seed = 11111;

        let mut bits = [BIT_0 as u16; 80];
        let copy_len = bits.len().min(words.len());
        bits[..copy_len].copy_from_slice(&words[..copy_len]);

        let mut parm = [0i16; PRM_SIZE + 1];
        parm[0] = bfi;
        let prm = bits2prm_ld8k(&bits);
        parm[1..].copy_from_slice(&prm);
        parm[4] = check_parity_pitch(w(parm[3]), w(parm[4])).0;
        if force_parity_error {
            parm[4] = 1;
        }

        let mut sid_sav = 0i16;
        let mut sh_sid_sav = 1i16;
        decod_ld8a(
            state,
            &parm,
            &mut synth,
            &mut az_dec,
            &mut t2,
            Some((&mut sid_sav, &mut sh_sid_sav)),
        );
        if bfi == 0 {
            cng.sid_sav = sid_sav;
            cng.sh_sid_sav = sh_sid_sav;
        }
    } else {
        let mut parm = [0i16; 5];
        parm[0] = ftyp;
        if ftyp == 2 {
            let mut sid_bits = [BIT_0 as u16; 16];
            let copy_len = sid_bits.len().min(words.len());
            sid_bits[..copy_len].copy_from_slice(&words[..copy_len]);
            let sid_prm = bits2prm_sid(&sid_bits);
            parm[1] = sid_prm[0];
            parm[2] = sid_prm[1];
            parm[3] = sid_prm[2];
            parm[4] = sid_prm[3];
        }

        cng.dec_cng(
            &parm,
            &mut state.old_exc,
            EXC_OFFSET,
            &mut state.lsp_old,
            &mut state.freq_prev,
        );

        let lsp_old = state.lsp_old;
        crate::lp::interp::int_qlpc(&lsp_old, &cng.lsp_sid, &mut az_dec);
        state.lsp_old = cng.lsp_sid;

        let mut az_idx = 0usize;
        for sf in 0..2 {
            let i_subfr = sf * L_SUBFR;
            let exc_idx = EXC_OFFSET + i_subfr;
            t2[sf] = state.old_t0;

            let mut a = [Word16(0); MP1];
            for i in 0..MP1 {
                a[i] = w(az_dec[az_idx + i]);
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
                let mut ctx = DspContext::default();
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

        state.sharp = SHARPMIN;
        if bfi == 0 {
            let mut exc_frame = [0i16; L_FRAME];
            exc_frame.copy_from_slice(&state.old_exc[EXC_OFFSET..EXC_OFFSET + L_FRAME]);
            cng.update_sid_energy(&exc_frame, 0);
        }

        slide_excitation_history(state);
    }

    cng.past_ftyp = ftyp;

    state.synth_buf[M..M + L_FRAME].copy_from_slice(&synth);
    if state.post_filter_enabled {
        crate::postfilter::pipeline::post_filter(state, &az_dec, &t2, &mut synth, ftyp);
    }
    crate::postproc::post_process(state, &mut synth);
    state.synth_buf[M..M + L_FRAME].copy_from_slice(&synth);

    state.frame_index = state.frame_index.wrapping_add(1);
    let mut out = [0i16; L_FRAME];
    out.copy_from_slice(&state.synth_buf[M..M + L_FRAME]);
    out
}
