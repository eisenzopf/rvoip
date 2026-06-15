//! Provenance: Annex B preprocessing path extracted from ITU `COD_LD8A.C`/`DTX.C`.
//! Q-format: LPC/LSP analysis retains Q12/Q13 coefficients and fixed-point filter state.

use crate::codec::state::{
    ENC_EXC_OFFSET, EncoderState, NEW_SPEECH_OFFSET, P_WINDOW_OFFSET, SPEECH_OFFSET, WSP_OFFSET,
};
use crate::constants::{GAMMA1, L_FRAME, L_SUBFR, L_WINDOW, M, MP1, NP};
use crate::dsp::arith::{mult, sub};
use crate::dsp::types::{DspContext, Word16};
use crate::filter::resid::residu_i16;
use crate::filter::syn::syn_filt_i16;

fn copy_window_or_none(old_speech: &[i16]) -> Option<[i16; L_WINDOW]> {
    let mut window = [0i16; L_WINDOW];
    let src = old_speech.get(P_WINDOW_OFFSET..P_WINDOW_OFFSET + L_WINDOW)?;
    window.copy_from_slice(src);
    Some(window)
}

#[derive(Clone)]
pub(super) struct VadAnalysis {
    pub(super) rc1: i16,
    pub(super) lsf_new: [i16; M],
    pub(super) r_h: [i16; NP + 1],
    pub(super) r_l: [i16; NP + 1],
    pub(super) rh_nbe: [i16; MP1],
    pub(super) exp_r0: i16,
    pub(super) sigpp: [i16; L_WINDOW],
}

pub(super) fn preprocess_and_analyze_vad(
    state: &mut EncoderState,
    pcm: &[i16; L_FRAME],
) -> VadAnalysis {
    let mut pre = *pcm;
    crate::preproc::pre_process_frame(state, &mut pre);
    state.old_speech[NEW_SPEECH_OFFSET..NEW_SPEECH_OFFSET + L_FRAME].copy_from_slice(&pre);

    let sigpp = match copy_window_or_none(&state.old_speech) {
        Some(window) => window,
        None => {
            debug_assert!(false, "old_speech window bounds invariant violated");
            return VadAnalysis {
                rc1: 0,
                lsf_new: [0i16; M],
                r_h: [0i16; NP + 1],
                r_l: [0i16; NP + 1],
                rh_nbe: [0i16; MP1],
                exp_r0: 0,
                sigpp: [0i16; L_WINDOW],
            };
        }
    };

    let mut r_h_np = [0i16; NP + 1];
    let mut r_l_np = [0i16; NP + 1];
    let mut exp_r0 = 0i16;
    crate::lp::autocorr::autocorr_np(&sigpp, &mut r_h_np, &mut r_l_np, &mut exp_r0);

    let mut rh_nbe = [0i16; MP1];
    rh_nbe.copy_from_slice(&r_h_np[..MP1]);

    let mut r_h_vad = r_h_np;
    let mut r_l_vad = r_l_np;
    crate::lp::window::lag_window_np(&mut r_h_vad, &mut r_l_vad);

    let mut r_h = [0i16; MP1];
    let mut r_l = [0i16; MP1];
    r_h.copy_from_slice(&r_h_vad[..MP1]);
    r_l.copy_from_slice(&r_l_vad[..MP1]);

    let mut a_lpc = [0i16; MP1];
    let mut rc = [0i16; M];
    crate::lp::levinson::levinson_10(state, &r_h, &r_l, &mut a_lpc, &mut rc, None);

    let mut lsp_new = [0i16; M];
    crate::lp::az_lsp::az_lsp(&a_lpc, &mut lsp_new, &state.lsp_old);
    let mut lsf_new = [0i16; M];
    crate::lp::lsf::lsp_to_lsf_annex_b(&lsp_new, &mut lsf_new);

    VadAnalysis {
        rc1: rc[1],
        lsf_new,
        r_h: r_h_vad,
        r_l: r_l_vad,
        rh_nbe,
        exp_r0,
        sigpp,
    }
}

pub(super) fn update_inactive_wsp_mem(state: &mut EncoderState, aq_t: &[i16; MP1 * 2]) {
    let mut ctx = DspContext::default();
    let speech_base = SPEECH_OFFSET;
    let exc_base = ENC_EXC_OFFSET;

    for sf in 0..2 {
        let i_subfr = sf * L_SUBFR;
        let mut aq = [0i16; MP1];
        aq.copy_from_slice(&aq_t[sf * MP1..sf * MP1 + MP1]);

        let mut xn = [0i16; L_SUBFR];
        residu_i16(
            &aq,
            &state.old_speech,
            speech_base + i_subfr,
            &mut xn,
            L_SUBFR,
        );

        let mut ap_t = [0i16; MP1];
        crate::lp::weight::weight_az(&aq, GAMMA1, &mut ap_t);

        let mut ap = [0i16; MP1];
        ap[0] = 4096;
        for i in 1..=M {
            let prev = mult(&mut ctx, Word16(ap_t[i - 1]), Word16(22938));
            ap[i] = sub(&mut ctx, Word16(ap_t[i]), prev).0;
        }

        let mut wsp_sub = [0i16; L_SUBFR];
        syn_filt_i16(&ap, &xn, &mut wsp_sub, L_SUBFR, &mut state.mem_w, true);
        state.old_wsp[WSP_OFFSET + i_subfr..WSP_OFFSET + i_subfr + L_SUBFR]
            .copy_from_slice(&wsp_sub);

        for (i, x) in xn.iter_mut().enumerate().take(L_SUBFR) {
            *x = sub(
                &mut ctx,
                Word16(*x),
                Word16(state.old_exc[exc_base + i_subfr + i]),
            )
            .0;
        }
        let mut xn_out = [0i16; L_SUBFR];
        syn_filt_i16(&ap_t, &xn, &mut xn_out, L_SUBFR, &mut state.mem_w0, true);
    }
}
