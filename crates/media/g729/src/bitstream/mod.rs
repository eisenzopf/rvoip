/// Public module `itu_params`.
pub mod itu_params;
/// Internal packed-bit helpers for legacy code paths.
#[doc(hidden)]
pub mod pack;
/// Internal packed-bit helpers for legacy code paths.
#[doc(hidden)]
pub mod unpack;

/// Public module `itu_serial`.
#[cfg(feature = "itu_serial")]
pub mod itu_serial;

pub use itu_params::{
    BITSNO, BITSNO2, pack_sid_params, pack_speech_params, unpack_sid_params, unpack_speech_params,
};
