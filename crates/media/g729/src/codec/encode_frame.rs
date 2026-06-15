//! Encoder per-frame orchestration for active speech.
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::bitstream::itu_params::pack_speech_params;
use crate::codec::encode_sub::{encode_subframe, synthesize_weighted_speech_subframes};
use crate::codec::state::{
    ENC_EXC_OFFSET, EncoderState, NEW_SPEECH_OFFSET, P_WINDOW_OFFSET, SPEECH_OFFSET, WSP_OFFSET,
};
use crate::constants::{GAMMA1, L_FRAME, L_SUBFR, L_TOTAL, L_WINDOW, M, MP1, PIT_MAX, PIT_MIN};
use crate::dsp::arith::{add, sub};
use crate::dsp::types::{DspContext, Word16};
use crate::filter::resid::residu_i16;

fn copy_window_or_none(old_speech: &[i16]) -> Option<[i16; L_WINDOW]> {
    let mut window = [0i16; L_WINDOW];
    let src = old_speech.get(P_WINDOW_OFFSET..P_WINDOW_OFFSET + L_WINDOW)?;
    window.copy_from_slice(src);
    Some(window)
}

pub(crate) fn advance_frame_counter(frame: &mut i16) {
    if *frame == 32767 {
        *frame = 256;
    } else {
        *frame = (*frame).wrapping_add(1);
    }
}

pub(crate) fn encode_speech_frame_impl(state: &mut EncoderState, pcm: &[i16; L_FRAME]) -> [u8; 10] {
    let mut ctx = DspContext::default();
    let mut pre = *pcm;
    crate::preproc::pre_process_frame(state, &mut pre);
    state.old_speech[NEW_SPEECH_OFFSET..NEW_SPEECH_OFFSET + L_FRAME].copy_from_slice(&pre);

    let p_window = match copy_window_or_none(&state.old_speech) {
        Some(window) => window,
        None => {
            debug_assert!(false, "old_speech window bounds invariant violated");
            return [0u8; 10];
        }
    };

    let mut r_h = [0i16; M + 1];
    let mut r_l = [0i16; M + 1];
    let mut exp_r0 = 0i16;
    crate::lp::autocorr::autocorr_10(&p_window, &mut r_h, &mut r_l, &mut exp_r0);
    crate::lp::window::lag_window_10(&mut r_h, &mut r_l);

    let mut a_lpc = [0i16; MP1];
    let mut rc = [0i16; M];
    crate::lp::levinson::levinson_10(state, &r_h, &r_l, &mut a_lpc, &mut rc, None);

    let mut lsp_new = [0i16; M];
    crate::lp::az_lsp::az_lsp(&a_lpc, &mut lsp_new, &state.lsp_old);
    let mut lsp_new_q = [0i16; M];
    let mut lsp_codes = [0i16; 2];
    crate::lsp_quant::encode::qua_lsp(state, &lsp_new, &mut lsp_new_q, &mut lsp_codes);

    let mut aq_t = [0i16; MP1 * 2];
    crate::lp::interp::int_qlpc(&state.lsp_old_q, &lsp_new_q, &mut aq_t);
    let mut ap_t = [0i16; MP1 * 2];
    let mut aq0 = [0i16; MP1];
    aq0.copy_from_slice(&aq_t[..MP1]);
    let mut aq1 = [0i16; MP1];
    aq1.copy_from_slice(&aq_t[MP1..]);
    let mut ap0 = [0i16; MP1];
    let mut ap1 = [0i16; MP1];
    crate::lp::weight::weight_az(&aq0, GAMMA1, &mut ap0);
    crate::lp::weight::weight_az(&aq1, GAMMA1, &mut ap1);
    ap_t[..MP1].copy_from_slice(&ap0);
    ap_t[MP1..].copy_from_slice(&ap1);

    state.lsp_old = lsp_new;
    state.lsp_old_q = lsp_new_q;

    let speech_base = SPEECH_OFFSET;
    let exc_base = ENC_EXC_OFFSET;
    let mut resid = [0i16; L_SUBFR];
    residu_i16(&aq0, &state.old_speech, speech_base, &mut resid, L_SUBFR);
    state.old_exc[exc_base..exc_base + L_SUBFR].copy_from_slice(&resid);
    residu_i16(
        &aq1,
        &state.old_speech,
        speech_base + L_SUBFR,
        &mut resid,
        L_SUBFR,
    );
    state.old_exc[exc_base + L_SUBFR..exc_base + L_FRAME].copy_from_slice(&resid);

    synthesize_weighted_speech_subframes(state, &ap_t);

    let t_op =
        crate::pitch::open_loop::search_open_loop(&state.old_wsp, WSP_OFFSET, PIT_MAX, L_FRAME);
    let mut t0_min = sub(&mut ctx, Word16(t_op), Word16(3)).0;
    if sub(&mut ctx, Word16(t0_min), Word16(PIT_MIN)).0 < 0 {
        t0_min = PIT_MIN;
    }
    let mut t0_max = add(&mut ctx, Word16(t0_min), Word16(6)).0;
    if sub(&mut ctx, Word16(t0_max), Word16(PIT_MAX)).0 > 0 {
        t0_max = PIT_MAX;
        t0_min = sub(&mut ctx, Word16(t0_max), Word16(6)).0;
    }

    let mut params = [Word16(0); 11];
    params[0] = Word16(lsp_codes[0]);
    params[1] = Word16(lsp_codes[1]);
    let mut p = 2usize;

    for sf in 0..2 {
        let i_subfr = sf * L_SUBFR;
        let mut ap = [0i16; MP1];
        ap.copy_from_slice(&ap_t[sf * MP1..sf * MP1 + MP1]);
        encode_subframe(
            state,
            i_subfr,
            &ap,
            &mut t0_min,
            &mut t0_max,
            &mut params,
            &mut p,
        );
    }

    state.old_speech.copy_within(L_FRAME..L_TOTAL, 0);
    state
        .old_wsp
        .copy_within(L_FRAME..(L_FRAME + PIT_MAX as usize), 0);
    state.old_exc.copy_within(
        L_FRAME..(L_FRAME + PIT_MAX as usize + crate::constants::L_INTERPOL),
        0,
    );

    advance_frame_counter(&mut state.frame);
    pack_speech_params(&params)
}
