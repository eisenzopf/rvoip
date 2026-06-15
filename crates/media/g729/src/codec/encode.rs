//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

#[cfg(feature = "annex_b")]
use crate::annex_b::{dtx::DtxState, vad::VadState};
#[cfg(feature = "annex_b")]
use crate::api::FrameType;
use crate::codec::state::EncoderState;
use crate::constants::L_FRAME;

/// Public function `encode_speech_frame`.
pub fn encode_speech_frame(state: &mut EncoderState, pcm: &[i16; L_FRAME]) -> [u8; 10] {
    crate::codec::encode_frame::encode_speech_frame_impl(state, pcm)
}

#[cfg(feature = "annex_b")]
pub(crate) fn encode_annex_b_frame(
    state: &mut EncoderState,
    vad: &mut VadState,
    dtx: &mut DtxState,
    pcm: &[i16; L_FRAME],
) -> (FrameType, [u8; 10]) {
    crate::codec::encode_annexb::encode_annex_b_frame_impl(state, vad, dtx, pcm)
}
