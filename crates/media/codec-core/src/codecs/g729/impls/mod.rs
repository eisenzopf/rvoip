#![allow(dead_code)]
#![allow(unused_imports)]
// This private subtree is a bit-exact fixed-point transcription of the G.729
// reference algorithm. These narrowly enumerated lints conflict with its
// deliberate wrapping casts, reference-style names and layout. Keep the
// exemption here so the public adapter and the rest of codec-core remain under
// the full strict lint policy.
#![allow(
    clippy::bool_to_int_with_if,
    clippy::branches_sharing_code,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::if_not_else,
    clippy::inline_always,
    clippy::many_single_char_names,
    clippy::manual_contains,
    clippy::manual_let_else,
    clippy::match_same_arms,
    clippy::missing_const_for_fn,
    clippy::needless_pass_by_ref_mut,
    clippy::needless_pass_by_value,
    clippy::redundant_pub_crate,
    clippy::similar_names,
    clippy::single_match_else,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::unreadable_literal,
    clippy::unused_self,
    clippy::use_self,
    clippy::useless_let_if_seq,
    clippy::verbose_bit_mask
)]

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
#[cfg(feature = "g729")]
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

    use crate::codecs::g729::impls::codec::state::{DecoderState, EncoderState};
    use crate::codecs::g729::impls::{G729Decoder, G729Encoder};

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
