//! Provenance: Frame encode/decode pipeline derived from ITU G.729 Annex A/B reference flow.
//! Q-format: Speech, excitation, and LPC paths follow Q0/Q12/Q13/Q15 fixed-point stages.

/// Public module `decode`.
pub mod decode;
#[cfg(feature = "g729")]
pub(crate) mod decode_annexb;
#[cfg(feature = "g729")]
mod decode_annexb_bits;
/// Public module `decode_sub`.
pub mod decode_sub;
pub(crate) mod decode_sub_helpers;
/// Public module `encode`.
pub mod encode;
#[cfg(feature = "g729")]
pub(crate) mod encode_annexb;
#[cfg(feature = "g729")]
pub(crate) mod encode_annexb_helpers;
pub(crate) mod encode_frame;
/// Public module `encode_sub`.
pub mod encode_sub;
/// Public module `erasure`.
pub mod erasure;
/// Public module `state`.
pub mod state;
