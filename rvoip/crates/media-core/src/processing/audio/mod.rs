//! Audio processing components for enhancing call quality
//!
//! This module contains implementations of audio processing algorithms 
//! that enhance call quality in VoIP applications.

// Acoustic Echo Cancellation (AEC)
pub mod aec;

// Automatic Gain Control (AGC)
pub mod agc;

// Voice Activity Detection (VAD)
pub mod vad;

// Noise Suppression
pub mod ns;

// Packet Loss Concealment
pub mod plc;

// DTMF Detection and Generation
pub mod dtmf;

// Common types and utilities for audio processing
mod common;
pub use common::*;

/// Trait for audio processing components
pub trait AudioProcessor: Send + Sync {
    /// Process an audio buffer in-place
    /// 
    /// Returns true if the buffer was modified
    fn process(&self, buffer: &mut crate::AudioBuffer) -> crate::error::Result<bool>;
    
    /// Reset the processor state
    fn reset(&mut self);
    
    /// Update processor configuration
    fn configure(&mut self, config: &std::collections::HashMap<String, String>) -> crate::error::Result<()>;
} 