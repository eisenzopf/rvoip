//! ITU-T G.729 Annex BA (Reduced Complexity + VAD/DTX/CNG) Codec Implementation
//!
//! This module provides a Rust implementation of the ITU-T G.729 Annex BA speech codec,
//! based on the official ITU reference implementation from G.729 Release 3.
//!
//! G.729 Annex BA combines the reduced complexity features of Annex A with the
//! Voice Activity Detection (VAD), Discontinuous Transmission (DTX), and 
//! Comfort Noise Generation (CNG) features of Annex B.
//!
//! ## Features
//! - 8 kbit/s speech coding with reduced computational complexity (Annex A)
//! - Voice Activity Detection for silence suppression (Annex B)
//! - Discontinuous Transmission for bandwidth savings (Annex B) 
//! - Comfort Noise Generation for natural silence (Annex B)
//! - Frame size: 80 samples (10ms at 8kHz)
//! - Compatible with standard G.729 bitstreams
//!
//! ## Reference Implementation
//! Based on ITU-T G.729 Release 3 Annex BA reference code (c_codeBA):
//! - cod_ld8a.c - Main encoder with VAD/DTX
//! - dec_ld8a.c - Main decoder with CNG
//! - vad.c - Voice Activity Detection
//! - dtx.c - Discontinuous Transmission  
//! - dec_sid.c - Comfort Noise Generation

pub mod types;
pub mod basic_ops;

#[cfg(test)]
pub mod tests;

// Re-export main types and functions
pub use types::*;

use crate::error::CodecError;

/// G.729BA codec information
pub fn codec_info() -> &'static str {
    "ITU-T G.729 Annex BA - 8 kbit/s speech codec with reduced complexity, VAD, DTX, and CNG"
}

/// TODO: Create a G.729BA encoder (not yet implemented)
pub fn create_encoder() -> Result<(), CodecError> {
    Err(CodecError::unsupported_codec("G.729BA encoder not yet implemented"))
}

/// TODO: Create a G.729BA decoder (not yet implemented)
pub fn create_decoder() -> Result<(), CodecError> {
    Err(CodecError::unsupported_codec("G.729BA decoder not yet implemented"))
} 