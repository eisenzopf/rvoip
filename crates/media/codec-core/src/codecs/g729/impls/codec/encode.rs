//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

#[cfg(feature = "g729")]
use crate::codecs::g729::impls::annex_b::{dtx::DtxState, vad::VadState};
#[cfg(feature = "g729")]
use crate::codecs::g729::impls::api::FrameType;
use crate::codecs::g729::impls::codec::state::EncoderState;
use crate::codecs::g729::impls::constants::L_FRAME;

/// Public function `encode_speech_frame`.
pub fn encode_speech_frame(state: &mut EncoderState, pcm: &[i16; L_FRAME]) -> [u8; 10] {
    crate::codecs::g729::impls::codec::encode_frame::encode_speech_frame_impl(state, pcm)
}

#[cfg(feature = "g729")]
pub(crate) fn encode_annex_b_frame(
    state: &mut EncoderState,
    vad: &mut VadState,
    dtx: &mut DtxState,
    pcm: &[i16; L_FRAME],
) -> (FrameType, [u8; 10]) {
    crate::codecs::g729::impls::codec::encode_annexb::encode_annex_b_frame_impl(
        state, vad, dtx, pcm,
    )
}
