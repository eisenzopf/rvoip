//! G.729A speech codec implementation
//! 
//! This module provides a complete implementation of the ITU-T G.729 Annex A
//! speech codec, operating at 8 kbit/s with reduced complexity.

#![warn(missing_docs)]

pub mod constants;
pub mod types;
pub mod math;
pub mod signal;
pub mod spectral;
pub mod perception;
pub mod excitation;
pub mod synthesis;
pub mod codec;
pub mod tables;

// Re-export main codec interfaces
pub use codec::{G729AEncoder, G729ADecoder};
pub use types::{AudioFrame, CodecError};

#[cfg(test)]
mod tests;

/// G.729A codec version information
pub const CODEC_VERSION: &str = "0.1.0";

/// G.729A codec name
pub const CODEC_NAME: &str = "G.729A"; 