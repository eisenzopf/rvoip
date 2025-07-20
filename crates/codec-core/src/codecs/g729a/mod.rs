//! ITU-T G.729 Annex A (Reduced Complexity) Codec Implementation
//!
//! This module provides a Rust implementation of the ITU-T G.729 Annex A speech codec,
//! based on the official ITU reference implementation from G.729 Release 3.
//!
//! G.729 Annex A provides approximately 30% complexity reduction compared to the full
//! G.729 implementation while maintaining excellent speech quality and compatibility.
//!
//! ## Features
//! - 8 kbit/s speech coding with reduced computational complexity
//! - Frame size: 80 samples (10ms at 8kHz)
//! - Compatible with standard G.729 bitstreams
//! - Optimized for embedded and real-time applications
//!
//! ## Reference Implementation
//! Based on ITU-T G.729 Release 3 Annex A reference code:
//! - COD_LD8A.C - Main encoder
//! - DEC_LD8A.C - Main decoder  
//! - ACELP_CA.C - Reduced complexity ACELP
//! - PITCH_A.C - Reduced complexity pitch analysis
//! - POSTFILT.C - Reduced complexity postfilter

pub mod types;
pub mod basic_ops;
pub mod lpc;
pub mod filtering;
pub mod encoder;
pub mod decoder;
pub mod tables;
pub mod quantization;
pub mod pitch;
pub mod acelp;
pub mod gain;

#[cfg(test)]
pub mod tests;

// Re-export main types and functions
pub use types::*;
pub use encoder::G729AEncoder;
pub use decoder::G729ADecoder;

use crate::error::CodecError;

/// G.729A codec information
pub fn codec_info() -> &'static str {
    "ITU-T G.729A (Annex A) - 8 kbit/s speech codec with reduced complexity"
}

/// Create a G.729A encoder
pub fn create_encoder() -> Result<G729AEncoder, CodecError> {
    Ok(G729AEncoder::new())
}

/// Create a G.729A decoder
pub fn create_decoder() -> Result<G729ADecoder, CodecError> {
    Ok(G729ADecoder::new())
}

// Removed duplicate inline tests - using external tests module instead 