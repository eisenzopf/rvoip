//! G.729A codec implementation

#![allow(missing_docs)]

// Public API modules
pub mod codec;
pub mod types;
pub mod constants;

// Public utilities for testing tools
pub mod bitstream_utils;

// Internal modules
mod math;
mod signal;
mod spectral;
mod perception;
mod excitation;
mod synthesis;
mod tables;

// Re-export main codec interfaces
pub use codec::{G729AEncoder, G729ADecoder};
pub use types::{AudioFrame, CodecError};

#[cfg(test)]
mod tests;

/// G.729A codec version information
pub const CODEC_VERSION: &str = "0.1.0";

/// G.729A codec name
pub const CODEC_NAME: &str = "G.729A"; 