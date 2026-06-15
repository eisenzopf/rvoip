/// Public module `itu_params`.
pub mod itu_params;
/// Internal packed-bit helpers for legacy code paths.
#[doc(hidden)]
pub mod pack;
/// Internal packed-bit helpers for legacy code paths.
#[doc(hidden)]
pub mod unpack;

/// Public module `itu_serial`.
#[cfg(any())]
pub mod itu_serial;

pub use itu_params::{
    pack_sid_params, pack_speech_params, unpack_sid_params, unpack_speech_params, BITSNO, BITSNO2,
};
