//! Encoder per-subframe helpers.
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::codec::state::{ENC_EXC_OFFSET, EncoderState, WSP_OFFSET};
use crate::constants::{GPCLIP, L_SUBFR, M, MP1, SHARPMAX, SHARPMIN};
use crate::dsp::arith::{add, extract_h, mult, negate, round, sub};
use crate::dsp::arith32::{l_mac, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16};
use crate::filter::syn::syn_filt_i16;
use crate::pitch::parity_pitch;

/// Build weighted speech memory for both subframes after LPC interpolation.
pub(crate) fn synthesize_weighted_speech_subframes(
    state: &mut EncoderState,
    ap_t: &[i16; MP1 * 2],
) {
    let mut ctx = DspContext::default();
    let exc_base = ENC_EXC_OFFSET;

    for sf in 0..2 {
        let i_subfr = sf * L_SUBFR;
        let mut ap = [0i16; MP1];
        ap.copy_from_slice(&ap_t[sf * MP1..sf * MP1 + MP1]);
        let mut ap1 = [0i16; MP1];
        ap1[0] = 4096;
        for i in 1..=M {
            let m = mult(&mut ctx, Word16(ap[i - 1]), Word16(22938));
            ap1[i] = sub(&mut ctx, Word16(ap[i]), m).0;
        }
        let mut x = [0i16; L_SUBFR];
        x.copy_from_slice(&state.old_exc[exc_base + i_subfr..exc_base + i_subfr + L_SUBFR]);
        let mut wsp_sub = [0i16; L_SUBFR];
        syn_filt_i16(&ap1, &x, &mut wsp_sub, L_SUBFR, &mut state.mem_w, true);
        state.old_wsp[WSP_OFFSET + i_subfr..WSP_OFFSET + i_subfr + L_SUBFR]
            .copy_from_slice(&wsp_sub);
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_subframe(
    state: &mut EncoderState,
    i_subfr: usize,
    ap: &[i16; MP1],
    t0_min: &mut i16,
    t0_max: &mut i16,
    params: &mut [Word16; 11],
    p: &mut usize,
) {
    let mut ctx = DspContext::default();
    let exc_base = ENC_EXC_OFFSET;

    let mut h1 = [0i16; L_SUBFR];
    h1[0] = 4096;
    let h1_in = h1;
    let mut hmem = [0i16; M];
    syn_filt_i16(ap, &h1_in, &mut h1, L_SUBFR, &mut hmem, false);

    let mut exc_sf = [0i16; L_SUBFR];
    exc_sf.copy_from_slice(&state.old_exc[exc_base + i_subfr..exc_base + i_subfr + L_SUBFR]);
    let mut xn = [0i16; L_SUBFR];
    let mut mem_w0_tmp = state.mem_w0;
    syn_filt_i16(ap, &exc_sf, &mut xn, L_SUBFR, &mut mem_w0_tmp, false);

    let mut t0_frac = 0i16;
    let t0 = crate::pitch::closed_loop::search_closed_loop(
        &mut state.old_exc,
        exc_base + i_subfr,
        &xn,
        &h1,
        *t0_min,
        *t0_max,
        i_subfr,
        &mut t0_frac,
    );

    let index = crate::pitch::lag_encode::encode_lag3(
        t0,
        t0_frac,
        t0_min,
        t0_max,
        crate::constants::PIT_MIN,
        crate::constants::PIT_MAX,
        i_subfr as i16,
    );
    params[*p] = Word16(index);
    *p += 1;
    if i_subfr == 0 {
        params[*p] = parity_pitch(Word16(index));
        *p += 1;
    }

    let mut y1 = [0i16; L_SUBFR];
    let mut mem_zero_tmp = state.mem_zero;
    let mut exc_sf = [0i16; L_SUBFR];
    exc_sf.copy_from_slice(&state.old_exc[exc_base + i_subfr..exc_base + i_subfr + L_SUBFR]);
    syn_filt_i16(ap, &exc_sf, &mut y1, L_SUBFR, &mut mem_zero_tmp, false);

    let mut g_coeff = [0i16; 4];
    let mut gain_pit = crate::pitch::closed_loop::g_pitch(&xn, &y1, &mut g_coeff);
    let taming = crate::gain::taming::test_excitation_error(state, t0, t0_frac);
    if taming == 1 && sub(&mut ctx, Word16(gain_pit), Word16(GPCLIP)).0 > 0 {
        gain_pit = GPCLIP;
    }

    let mut xn2 = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        let mut l_temp = l_mult(&mut ctx, Word16(y1[i]), Word16(gain_pit));
        l_temp = l_shl(&mut ctx, l_temp, 1);
        xn2[i] = sub(&mut ctx, Word16(xn[i]), extract_h(l_temp)).0;
    }

    let mut code = [0i16; L_SUBFR];
    let mut y2 = [0i16; L_SUBFR];
    let mut sign = 0i16;
    let mut h1_work = h1;
    let cb_index = crate::fixed_cb::search::search_acelp_codebook(
        &xn2,
        &mut h1_work,
        t0,
        state.sharp,
        &mut code,
        &mut y2,
        &mut sign,
    );
    params[*p] = Word16(cb_index);
    *p += 1;
    params[*p] = Word16(sign);
    *p += 1;

    let mut g_coeff_cs = [0i16; 5];
    let mut exp_g_coeff_cs = [0i16; 5];
    g_coeff_cs[0] = g_coeff[0];
    exp_g_coeff_cs[0] = negate(&mut ctx, Word16(g_coeff[1])).0;
    g_coeff_cs[1] = negate(&mut ctx, Word16(g_coeff[2])).0;
    let tmp = add(&mut ctx, Word16(g_coeff[3]), Word16(1));
    exp_g_coeff_cs[1] = negate(&mut ctx, tmp).0;
    crate::fixed_cb::correlation::corr_xy2(&xn, &y1, &y2, &mut g_coeff_cs, &mut exp_g_coeff_cs);

    let mut gain_code = 0i16;
    let gain_index = crate::gain::quantize::quantize_gain(
        state,
        &code,
        &g_coeff_cs,
        &exp_g_coeff_cs,
        &mut gain_pit,
        &mut gain_code,
        taming,
    );
    params[*p] = Word16(gain_index);
    *p += 1;

    state.sharp = gain_pit;
    if sub(&mut ctx, Word16(state.sharp), Word16(SHARPMAX)).0 > 0 {
        state.sharp = SHARPMAX;
    }
    if sub(&mut ctx, Word16(state.sharp), Word16(SHARPMIN)).0 < 0 {
        state.sharp = SHARPMIN;
    }

    for (i, &code_i) in code.iter().enumerate().take(L_SUBFR) {
        let idx = exc_base + i_subfr + i;
        let mut l_temp = l_mult(&mut ctx, Word16(state.old_exc[idx]), Word16(gain_pit));
        l_temp = l_mac(&mut ctx, l_temp, Word16(code_i), Word16(gain_code));
        l_temp = l_shl(&mut ctx, l_temp, 1);
        state.old_exc[idx] = round(&mut ctx, l_temp).0;
    }
    crate::gain::taming::update_excitation_error(state, gain_pit, t0);

    for (j, i) in ((L_SUBFR - M)..L_SUBFR).enumerate() {
        let l_temp = l_mult(&mut ctx, Word16(y1[i]), Word16(gain_pit));
        let l_temp = l_shl(&mut ctx, l_temp, 1);
        let temp = extract_h(l_temp).0;
        let l_k = l_mult(&mut ctx, Word16(y2[i]), Word16(gain_code));
        let l_k = l_shl(&mut ctx, l_k, 2);
        let k = extract_h(l_k).0;
        let ak = add(&mut ctx, Word16(temp), Word16(k));
        state.mem_w0[j] = sub(&mut ctx, Word16(xn[i]), ak).0;
    }
}
