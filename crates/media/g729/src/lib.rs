#![cfg_attr(not(feature = "std"), no_std)]

/// Public API layer (`G729Encoder`, `G729Decoder`, configs, and frame types).
pub mod api;
/// Public bitstream utilities.
pub mod bitstream;
/// Public constants used by API consumers.
pub mod constants;
/// Public error type.
pub mod error;

/// Internal codec pipeline modules (kept public-for-testing but hidden from docs).
#[doc(hidden)]
pub mod codec;
/// Internal DSP helpers.
#[doc(hidden)]
pub mod dsp;
/// Internal filter helpers.
#[doc(hidden)]
pub mod filter;
/// Internal fixed codebook helpers.
#[doc(hidden)]
pub mod fixed_cb;
/// Internal gain helpers.
#[doc(hidden)]
pub mod gain;
/// Internal LP analysis helpers.
#[doc(hidden)]
pub mod lp;
/// Internal LSP quantization helpers.
#[doc(hidden)]
pub mod lsp_quant;
/// Internal pitch helpers.
#[doc(hidden)]
pub mod pitch;
/// Internal post-filter helpers.
#[doc(hidden)]
pub mod postfilter;
/// Internal post-processing helpers.
#[doc(hidden)]
pub mod postproc;
/// Internal pre-processing helpers.
#[doc(hidden)]
pub mod preproc;
/// Internal codec tables.
#[doc(hidden)]
pub mod tables;

/// Internal Annex B helpers.
#[cfg(feature = "annex_b")]
#[doc(hidden)]
pub mod annex_b;

/// Public encoder/decoder runtime configuration types.
pub use api::{DecoderConfig, EncoderConfig, FrameType, G729Config, G729Decoder, G729Encoder};
/// Public re-export.
pub use error::CodecError;
/// Backward-compatible alias.
pub type G729Error = CodecError;

/// Number of PCM samples per 10 ms frame.
pub const FRAME_SAMPLES: usize = 80;
/// Packed speech frame size in bytes.
pub const SPEECH_FRAME_BYTES: usize = 10;
/// Packed SID frame size in bytes.
pub const SID_FRAME_BYTES: usize = 2;

#[cfg(test)]
mod tests {
    use core::mem::size_of;

    use crate::codec::state::{DecoderState, EncoderState};
    use crate::{G729Decoder, G729Encoder};

    #[test]
    fn send_bounds_compile_for_public_types() {
        fn assert_send<T: Send>() {}
        assert_send::<G729Encoder>();
        assert_send::<G729Decoder>();
        assert_send::<EncoderState>();
        assert_send::<DecoderState>();
    }

    #[test]
    fn size_assertions_encoder_decoder_state() {
        assert!(size_of::<EncoderState>() < 8 * 1024);
        assert!(size_of::<DecoderState>() < 4 * 1024);
        assert!(size_of::<EncoderState>() + size_of::<DecoderState>() < 64 * 1024);
    }
}
