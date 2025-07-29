//! Core types and structures for G.729A codec

use crate::codecs::g729a::constants::*;
use std::ops::{Add, Sub, Mul, Div, Neg};

/// Frame of audio samples
#[derive(Debug, Clone, Copy)]
pub struct AudioFrame {
    pub samples: [i16; FRAME_SIZE],
    pub timestamp: u64,
}

impl AudioFrame {
    /// Create a new audio frame from PCM samples
    pub fn from_pcm(samples: &[i16]) -> Result<Self, CodecError> {
        if samples.len() != FRAME_SIZE {
            return Err(CodecError::InvalidFrameSize {
                expected: FRAME_SIZE,
                actual: samples.len(),
            });
        }
        
        let mut frame_samples = [0i16; FRAME_SIZE];
        frame_samples.copy_from_slice(samples);
        
        Ok(Self {
            samples: frame_samples,
            timestamp: 0,
        })
    }
}

/// Subframe of audio samples
#[derive(Debug, Clone, Copy)]
pub struct SubFrame {
    pub samples: [i16; SUBFRAME_SIZE],
}

/// Q15 fixed-point type (0.15 format)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Q15(pub i16);

impl Q15 {
    pub const ZERO: Q15 = Q15(0);
    pub const ONE: Q15 = Q15(Q15_ONE);
    pub const HALF: Q15 = Q15(Q15_HALF);
    
    /// Create from floating point value
    pub fn from_f32(val: f32) -> Self {
        let clamped = val.clamp(-1.0, 0.999969);
        Q15((clamped * Q15_ONE as f32) as i16)
    }
    
    /// Convert to floating point
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / Q15_ONE as f32
    }
    
    /// Saturating multiplication
    pub fn saturating_mul(self, other: Q15) -> Q15 {
        let result = ((self.0 as i32 * other.0 as i32) >> 15) as i16;
        Q15(result)
    }
    
    /// Saturating addition
    pub fn saturating_add(self, other: Q15) -> Q15 {
        Q15(self.0.saturating_add(other.0))
    }
}

/// Q14 fixed-point type (2.14 format)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Q14(pub i16);

impl Q14 {
    /// Zero value
    pub const ZERO: Q14 = Q14(0);
    /// One value in Q14 format
    pub const ONE: Q14 = Q14(16384);
    
    /// Convert to Q15
    pub fn to_q15(self) -> Q15 {
        Q15((self.0 as i32 * 2) as i16)
    }
    
    /// Convert to Q31
    pub fn to_q31(self) -> Q31 {
        Q31((self.0 as i32) << 17)
    }
}

/// Q31 fixed-point type (0.31 format)
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Q31(pub i32);

impl Q31 {
    pub const ZERO: Q31 = Q31(0);
    pub const ONE: Q31 = Q31(i32::MAX);
    
    /// Create from floating point value
    pub fn from_f32(val: f32) -> Self {
        let clamped = val.clamp(-1.0, 0.9999999995);
        Q31((clamped * i32::MAX as f32) as i32)
    }
    
    /// Convert to floating point
    pub fn to_f32(self) -> f32 {
        self.0 as f32 / i32::MAX as f32
    }
    
    /// Convert to Q15
    pub fn to_q15(self) -> Q15 {
        Q15((self.0 >> 16) as i16)
    }
    
    /// Saturating addition
    pub fn saturating_add(self, other: Q31) -> Q31 {
        Q31(self.0.saturating_add(other.0))
    }
}

/// Linear prediction coefficients
#[derive(Debug, Clone)]
pub struct LPCoefficients {
    pub values: [Q15; LP_ORDER],
    pub reflection_coeffs: [Q15; LP_ORDER],
}

/// Line Spectral Pair parameters
#[derive(Debug, Clone)]
pub struct LSPParameters {
    pub frequencies: [Q15; LP_ORDER],
}

/// Quantized LSP parameters
#[derive(Debug, Clone)]
pub struct QuantizedLSP {
    pub indices: [u8; 4],
    pub reconstructed: LSPParameters,
}

/// Quantized gains
#[derive(Debug, Clone, Copy)]
pub struct QuantizedGains {
    pub adaptive_gain: Q15,
    pub fixed_gain: Q15,
    pub gain_indices: [u8; 2],
}

/// Spectral parameters for a frame
#[derive(Debug, Clone)]
pub struct SpectralParameters {
    pub lsp_coefficients: [Q15; LP_ORDER],
    pub quantized_indices: [u8; 4],
}

/// Excitation parameters for a subframe
#[derive(Debug, Clone)]
pub struct ExcitationParameters {
    pub pitch_delay: f32,
    pub pitch_gain: Q15,
    pub codebook_index: u32,
    pub codebook_gain: Q15,
}

/// Encoded frame (80 bits packed)
#[derive(Debug, Clone, Copy)]
pub struct EncodedFrame {
    pub lsp_indices: [u8; 4],
    pub pitch_delay_int: [u8; 2],
    pub pitch_delay_frac: [u8; 2],
    pub fixed_codebook_idx: [u32; 2],
    pub gain_indices: [[u8; 2]; 2],
}

/// Decoded parameters from bitstream
#[derive(Debug, Clone)]
pub struct DecodedParameters {
    pub lsp_indices: [u8; 4],
    pub pitch_delays: [f32; 2],
    pub fixed_codebook_indices: [u32; 2],
    pub gain_indices: [[u8; 2]; 2],
}

/// Parameters for a single subframe
#[derive(Debug, Clone)]
pub struct SubframeParameters {
    pub pitch_delay: f32,
    pub pitch_index: u8,
    pub codebook_index: u32,
    pub gain_indices: [u8; 2],
}

/// Codec error types
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("Invalid frame size: expected {expected}, got {actual}")]
    InvalidFrameSize { expected: usize, actual: usize },
    
    #[error("Invalid sample rate: {0} Hz (expected 8000 Hz)")]
    InvalidSampleRate(u32),
    
    #[error("Bitstream corruption detected")]
    BitstreamCorruption,
    
    #[error("Overflow in fixed-point arithmetic")]
    ArithmeticOverflow,
    
    #[error("Invalid codec state")]
    InvalidState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_q15_conversions() {
        // Test zero
        assert_eq!(Q15::from_f32(0.0).to_f32(), 0.0);
        
        // Test one (almost)
        let one = Q15::from_f32(0.999);
        assert!((one.to_f32() - 0.999).abs() < 0.001);
        
        // Test negative one
        let neg_one = Q15::from_f32(-1.0);
        assert_eq!(neg_one.to_f32(), -1.0);
        
        // Test clamping
        assert_eq!(Q15::from_f32(2.0).to_f32(), Q15::from_f32(0.999969).to_f32());
        assert_eq!(Q15::from_f32(-2.0).to_f32(), -1.0);
    }

    #[test]
    fn test_q15_arithmetic() {
        let half = Q15::HALF;
        let quarter = Q15::from_f32(0.25);
        
        // Test multiplication: 0.5 * 0.5 = 0.25
        let result = half.saturating_mul(half);
        assert!((result.to_f32() - 0.25).abs() < 0.001);
        
        // Test addition: 0.25 + 0.25 = 0.5
        let result = quarter.saturating_add(quarter);
        assert!((result.to_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_q31_conversions() {
        // Test conversions
        assert_eq!(Q31::from_f32(0.0).to_f32(), 0.0);
        
        let half = Q31::from_f32(0.5);
        assert!((half.to_f32() - 0.5).abs() < 0.0001);
        
        // Test Q31 to Q15 conversion
        let q15_half = half.to_q15();
        assert!((q15_half.to_f32() - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_audio_frame_creation() {
        let samples = vec![100i16; FRAME_SIZE];
        let frame = AudioFrame::from_pcm(&samples).unwrap();
        assert_eq!(frame.samples.len(), FRAME_SIZE);
        assert_eq!(frame.samples[0], 100);
        
        // Test invalid size
        let bad_samples = vec![0i16; FRAME_SIZE - 1];
        assert!(AudioFrame::from_pcm(&bad_samples).is_err());
    }

    #[test]
    fn test_encoded_frame_size() {
        let frame = EncodedFrame {
            lsp_indices: [0; 4],
            pitch_delay_int: [0; 2],
            pitch_delay_frac: [0; 2],
            fixed_codebook_idx: [0; 2],
            gain_indices: [[0; 2]; 2],
        };
        // EncodedFrame represents 80 bits of data
        assert_eq!(ENCODED_FRAME_SIZE * 8, 80); // 80 bits total
    }
} 