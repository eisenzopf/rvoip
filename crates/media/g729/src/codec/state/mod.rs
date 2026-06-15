//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

mod decoder_state;
mod encoder_state;

/// Public re-export.
pub use decoder_state::{DecoderState, EXC_OFFSET, OLD_EXC_LEN, RES2_BUF_LEN};
/// Public re-export.
pub use encoder_state::{
    EXC_OFFSET as ENC_EXC_OFFSET, EncoderState, NEW_SPEECH_OFFSET, OLD_EXC_LEN as ENC_OLD_EXC_LEN,
    OLD_WSP_LEN, P_WINDOW_OFFSET, SPEECH_OFFSET, WSP_OFFSET,
};
