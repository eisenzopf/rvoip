//! Automatic Gain Control (AGC)
//!
//! This module implements automatic gain control to maintain consistent audio levels
//! by dynamically adjusting the gain based on input signal characteristics.

use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

/// Configuration for Automatic Gain Control
#[derive(Debug, Clone)]
pub struct AgcConfig {
    /// Target audio level (0.0-1.0, where 1.0 is maximum)
    pub target_level: f32,
    /// Attack time in milliseconds (how quickly gain increases)
    pub attack_time_ms: f32,
    /// Release time in milliseconds (how quickly gain decreases) 
    pub release_time_ms: f32,
    /// Minimum gain multiplier
    pub min_gain: f32,
    /// Maximum gain multiplier
    pub max_gain: f32,
    /// Compression ratio (1.0 = no compression, higher = more compression)
    pub compression_ratio: f32,
    /// Enable limiter to prevent clipping
    pub enable_limiter: bool,
}

impl Default for AgcConfig {
    fn default() -> Self {
        Self {
            target_level: 0.25,        // 25% of maximum level
            attack_time_ms: 10.0,      // Fast attack (10ms)
            release_time_ms: 100.0,    // Slower release (100ms)
            min_gain: 0.1,             // -20dB minimum
            max_gain: 10.0,            // +20dB maximum
            compression_ratio: 3.0,    // 3:1 compression
            enable_limiter: true,      // Prevent clipping
        }
    }
}

/// Result of AGC processing
#[derive(Debug, Clone, Copy)]
pub struct AgcResult {
    /// Applied gain multiplier
    pub applied_gain: f32,
    /// Input level (RMS)
    pub input_level: f32,
    /// Output level (RMS)
    pub output_level: f32,
    /// Whether limiting was applied
    pub limiter_active: bool,
}

/// Automatic Gain Control processor
pub struct AutomaticGainControl {
    /// AGC configuration
    config: AgcConfig,
    /// Current gain value (smoothed)
    current_gain: f32,
    /// Attack coefficient for gain increases
    attack_coeff: f32,
    /// Release coefficient for gain decreases
    release_coeff: f32,
    /// Peak level tracker for limiter
    peak_level: f32,
    /// Frame count for initialization
    frame_count: u64,
}

impl AutomaticGainControl {
    /// Create a new AGC with the given configuration
    pub fn new(config: AgcConfig) -> Result<Self> {
        debug!("Creating AutomaticGainControl with config: {:?}", config);
        
        // Validate configuration
        if config.target_level <= 0.0 || config.target_level > 1.0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "AGC target level must be between 0.0 and 1.0".to_string(),
            }.into());
        }
        
        if config.min_gain <= 0.0 || config.min_gain > config.max_gain {
            return Err(AudioProcessingError::InvalidFormat {
                details: "AGC min_gain must be positive and <= max_gain".to_string(),
            }.into());
        }
        
        // Calculate attack and release coefficients (assuming 8kHz, 20ms frames)
        let sample_rate = 8000.0;
        let frame_size = 160.0; // 20ms at 8kHz
        
        let attack_coeff = Self::calculate_time_constant(config.attack_time_ms, sample_rate, frame_size);
        let release_coeff = Self::calculate_time_constant(config.release_time_ms, sample_rate, frame_size);
        
        Ok(Self {
            config,
            current_gain: 1.0,  // Start with unity gain
            attack_coeff,
            release_coeff,
            peak_level: 0.0,
            frame_count: 0,
        })
    }
    
    /// Process an audio frame with AGC
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Result<AgcResult> {
        if frame.samples.is_empty() {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Audio frame has no samples".to_string(),
            }.into());
        }
        
        // Calculate input level (RMS)
        let input_level = self.calculate_rms_level(&frame.samples);
        
        // Calculate desired gain
        let desired_gain = if input_level > 0.0 {
            (self.config.target_level / input_level).min(self.config.max_gain).max(self.config.min_gain)
        } else {
            self.config.max_gain // Boost very quiet signals
        };
        
        // Apply attack/release smoothing
        let gain_diff = desired_gain - self.current_gain;
        let coeff = if gain_diff > 0.0 {
            self.attack_coeff  // Gain increase
        } else {
            self.release_coeff // Gain decrease
        };
        
        self.current_gain += gain_diff * coeff;
        
        // Apply compression if configured
        let compressed_gain = if self.config.compression_ratio > 1.0 {
            self.apply_compression(self.current_gain, input_level)
        } else {
            self.current_gain
        };
        
        // Apply limiter if enabled
        let (final_gain, limiter_active) = if self.config.enable_limiter {
            self.apply_limiter(compressed_gain, input_level)
        } else {
            (compressed_gain, false)
        };
        
        // Calculate output level
        let output_level = input_level * final_gain;
        
        self.frame_count += 1;
        
        trace!("AGC: input={:.4}, gain={:.2}, output={:.4}, limiter={}", 
               input_level, final_gain, output_level, limiter_active);
        
        Ok(AgcResult {
            applied_gain: final_gain,
            input_level,
            output_level,
            limiter_active,
        })
    }
    
    /// Apply AGC gain to audio samples
    pub fn apply_gain(&self, samples: &mut [i16], gain: f32) {
        for sample in samples.iter_mut() {
            let adjusted = (*sample as f32) * gain;
            // Clamp to 16-bit range
            *sample = adjusted.max(i16::MIN as f32).min(i16::MAX as f32) as i16;
        }
    }
    
    /// Reset AGC state
    pub fn reset(&mut self) {
        self.current_gain = 1.0;
        self.peak_level = 0.0;
        self.frame_count = 0;
        debug!("AGC state reset");
    }
    
    /// Get current gain value
    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
    
    /// Calculate RMS level of audio samples
    fn calculate_rms_level(&self, samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum_squares: f64 = samples.iter()
            .map(|&sample| (sample as f64).powi(2))
            .sum();
        
        let rms = (sum_squares / samples.len() as f64).sqrt();
        
        // Normalize to 0.0-1.0 range (assuming 16-bit samples)
        (rms / 32768.0) as f32
    }
    
    /// Apply dynamic range compression
    fn apply_compression(&self, gain: f32, input_level: f32) -> f32 {
        // Simple compression: reduce gain for louder signals
        let threshold = self.config.target_level * 0.8; // 80% of target
        
        if input_level > threshold {
            let over_threshold = input_level - threshold;
            let compression_reduction = over_threshold / self.config.compression_ratio;
            gain * (1.0 - compression_reduction).max(0.1)
        } else {
            gain
        }
    }
    
    /// Apply peak limiter to prevent clipping
    fn apply_limiter(&mut self, gain: f32, input_level: f32) -> (f32, bool) {
        let predicted_peak = input_level * gain;
        let limit_threshold = 0.95; // 95% of maximum to leave headroom
        
        // Update peak tracker
        self.peak_level = self.peak_level * 0.999 + predicted_peak * 0.001;
        
        if predicted_peak > limit_threshold {
            let limited_gain = limit_threshold / input_level.max(0.001);
            (limited_gain.min(gain), true)
        } else {
            (gain, false)
        }
    }
    
    /// Calculate exponential time constant for attack/release
    fn calculate_time_constant(time_ms: f32, sample_rate: f32, frame_size: f32) -> f32 {
        let frames_per_second = sample_rate / frame_size;
        let time_in_frames = (time_ms / 1000.0) * frames_per_second;
        
        // Exponential decay coefficient
        if time_in_frames > 0.0 {
            1.0 - (-1.0 / time_in_frames).exp()
        } else {
            1.0
        }
    }
} 