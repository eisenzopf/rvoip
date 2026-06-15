//! Annex B encode orchestration (VAD/DTX path).
//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

use crate::codecs::g729::impls::annex_b::{dtx::DtxState, vad::VadState};
use crate::codecs::g729::impls::api::FrameType;
use crate::codecs::g729::impls::bitstream::itu_params::pack_sid_params;
use crate::codecs::g729::impls::codec::encode_annexb_helpers::{
    preprocess_and_analyze_vad, update_inactive_wsp_mem,
};
use crate::codecs::g729::impls::codec::encode_frame::{
    advance_frame_counter, encode_speech_frame_impl,
};
use crate::codecs::g729::impls::codec::state::EncoderState;
use crate::codecs::g729::impls::constants::{L_FRAME, L_TOTAL, MP1, PIT_MAX, SHARPMIN};
use crate::codecs::g729::impls::dsp::types::Word16;

pub(crate) fn encode_annex_b_frame_impl(
    state: &mut EncoderState,
    vad: &mut VadState,
    dtx: &mut DtxState,
    pcm: &[i16; L_FRAME],
) -> (FrameType, [u8; 10]) {
    let mut probe = state.clone();
    let analysis = preprocess_and_analyze_vad(&mut probe, pcm);
    let frame_count = if state.frame == 32767 {
        256
    } else {
        state.frame.wrapping_add(1)
    };
    let vad_dec = vad.detect_from_analysis(
        analysis.rc1,
        &analysis.lsf_new,
        &analysis.r_h,
        &analysis.r_l,
        analysis.exp_r0,
        &analysis.sigpp,
        frame_count,
        state.past_vad,
        state.ppast_vad,
    );

    dtx.update_cng(&analysis.rh_nbe, analysis.exp_r0, vad_dec);

    if vad_dec != 0 {
        state.ppast_vad = state.past_vad;
        state.past_vad = vad_dec;
        state.seed = 11111;
        let bits = encode_speech_frame_impl(state, pcm);
        return (FrameType::Speech, bits);
    }

    *state = probe;

    let past_vad = state.past_vad;
    let mut aq_t = [0i16; MP1 * 2];
    let mut ana = [0i16; 5];
    dtx.cod_cng(state, past_vad, &mut aq_t, &mut ana);

    state.ppast_vad = state.past_vad;
    state.past_vad = vad_dec;

    update_inactive_wsp_mem(state, &aq_t);
    state.sharp = SHARPMIN;

    state.old_speech.copy_within(L_FRAME..L_TOTAL, 0);
    state
        .old_wsp
        .copy_within(L_FRAME..(L_FRAME + PIT_MAX as usize), 0);
    state.old_exc.copy_within(
        L_FRAME..(L_FRAME + PIT_MAX as usize + crate::codecs::g729::impls::constants::L_INTERPOL),
        0,
    );
    advance_frame_counter(&mut state.frame);

    let mut out = [0u8; 10];
    if ana[0] == 2 {
        let sid = pack_sid_params(&[
            Word16(ana[1]),
            Word16(ana[2]),
            Word16(ana[3]),
            Word16(ana[4]),
        ]);
        out[..2].copy_from_slice(&sid);
        (FrameType::Sid, out)
    } else {
        (FrameType::NoData, out)
    }
}
