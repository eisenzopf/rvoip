//! Noise Suppression (NS) implementation
//!
//! This module provides noise suppression capabilities for reducing background noise
//! in audio streams while preserving speech quality.

use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::{Sample, SampleRate, AudioBuffer, AudioFormat};
use super::common::AudioProcessor;

/// Noise suppression level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseSuppressionLevel {
    /// Low suppression
    Low,
    /// Moderate suppression
    Moderate,
    /// High suppression
    High,
    /// Very high suppression
    VeryHigh,
}

impl Default for NoiseSuppressionLevel {
    fn default() -> Self {
        Self::Moderate
    }
}

/// Configuration options for noise suppression
#[derive(Debug, Clone)]
pub struct NoiseSuppressionConfig {
    /// Suppression level
    pub level: NoiseSuppressionLevel,
    
    /// Enable VAD-based gating
    pub vad_enabled: bool,
    
    /// Preserve speech harmonics
    pub preserve_harmonics: bool,
}

impl Default for NoiseSuppressionConfig {
    fn default() -> Self {
        Self {
            level: NoiseSuppressionLevel::default(),
            vad_enabled: true,
            preserve_harmonics: true,
        }
    }
}

/// Noise suppression implementation
#[derive(Debug)]
pub struct NoiseSuppressor {
    /// Configuration options
    config: NoiseSuppressionConfig,
    
    /// Sample rate
    sample_rate: SampleRate,
    
    /// Current noise floor estimate in dBFS
    noise_floor_dbfs: f32,
    
    /// Speech probability (0.0 - 1.0)
    speech_probability: f32,
    
    /// Frame size in samples
    frame_size: usize,
}

impl NoiseSuppressor {
    /// Create a new noise suppressor with the given configuration
    pub fn new(config: NoiseSuppressionConfig, sample_rate: SampleRate) -> Self {
        // Calculate optimal frame size (10ms chunks)
        let frame_size = (sample_rate.as_hz() as usize * 10) / 1000;
        
        Self {
            config,
            sample_rate,
            noise_floor_dbfs: -70.0,
            speech_probability: 0.0,
            frame_size,
        }
    }
    
    /// Create a new noise suppressor with default configuration
    pub fn default_for_rate(sample_rate: SampleRate) -> Self {
        Self::new(NoiseSuppressionConfig::default(), sample_rate)
    }
    
    /// Get the current noise floor estimate in dBFS
    pub fn noise_floor_dbfs(&self) -> f32 {
        self.noise_floor_dbfs
    }
    
    /// Get the current speech probability
    pub fn speech_probability(&self) -> f32 {
        self.speech_probability
    }
    
    /// Set the suppression level
    pub fn set_level(&mut self, level: NoiseSuppressionLevel) {
        self.config.level = level;
    }
    
    /// Get the frame size in samples
    pub fn frame_size(&self) -> usize {
        self.frame_size
    }
}

impl AudioProcessor for NoiseSuppressor {
    fn process(&mut self, input: &AudioBuffer) -> Result<AudioBuffer> {
        if input.format.sample_rate != self.sample_rate {
            return Err(Error::InvalidArgument(format!(
                "Buffer sample rate ({:?}) doesn't match processor sample rate ({:?})",
                input.format.sample_rate, self.sample_rate
            )));
        }
        
        // Stub implementation
        Err(Error::NotImplemented("Noise suppression not yet implemented".to_string()))
    }
    
    fn process_samples(&mut self, input: &[Sample]) -> Result<Vec<Sample>> {
        if input.len() % self.frame_size != 0 {
            return Err(Error::InvalidArgument(format!(
                "Input length ({}) is not a multiple of frame size ({})",
                input.len(), self.frame_size
            )));
        }
        
        // Stub implementation
        Err(Error::NotImplemented("Noise suppression not yet implemented".to_string()))
    }
    
    fn reset(&mut self) -> Result<()> {
        self.noise_floor_dbfs = -70.0;
        self.speech_probability = 0.0;
        Ok(())
    }
    
    fn name(&self) -> &str {
        "NoiseSuppressor"
    }
}

/// Builder for creating noise suppressor instances
pub struct NoiseSuppressionBuilder {
    config: NoiseSuppressionConfig,
    sample_rate: SampleRate,
}

impl NoiseSuppressionBuilder {
    /// Create a new noise suppression builder
    pub fn new() -> Self {
        Self {
            config: NoiseSuppressionConfig::default(),
            sample_rate: SampleRate::Rate16000,
        }
    }
    
    /// Set the suppression level
    pub fn with_level(mut self, level: NoiseSuppressionLevel) -> Self {
        self.config.level = level;
        self
    }
    
    /// Enable or disable VAD-based gating
    pub fn with_vad(mut self, enabled: bool) -> Self {
        self.config.vad_enabled = enabled;
        self
    }
    
    /// Enable or disable harmonic preservation
    pub fn with_harmonic_preservation(mut self, enabled: bool) -> Self {
        self.config.preserve_harmonics = enabled;
        self
    }
    
    /// Set the sample rate
    pub fn with_sample_rate(mut self, rate: SampleRate) -> Self {
        self.sample_rate = rate;
        self
    }
    
    /// Build the noise suppressor
    pub fn build(self) -> NoiseSuppressor {
        NoiseSuppressor::new(self.config, self.sample_rate)
    }
} 