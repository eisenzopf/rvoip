//! Voice Activity Detection (VAD)
//!
//! This module implements voice activity detection to distinguish between
//! speech and silence in audio streams.

use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

/// Configuration for Voice Activity Detection
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Energy threshold for voice detection (relative to frame RMS)
    pub energy_threshold: f32,
    /// Zero crossing rate threshold 
    pub zcr_threshold: f32,
    /// Minimum frame length for analysis (samples)
    pub min_frame_length: usize,
    /// Smoothing factor for energy calculation (0.0-1.0)
    pub energy_smoothing: f32,
    /// Hangover frames (frames to keep detecting voice after energy drops)
    pub hangover_frames: u32,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.01,     // 1% of max energy
            zcr_threshold: 0.15,        // 15% zero crossing rate
            min_frame_length: 160,      // 20ms at 8kHz
            energy_smoothing: 0.9,      // 90% history, 10% current
            hangover_frames: 5,         // Keep detecting for 5 frames after drop
        }
    }
}

/// Result of voice activity detection
#[derive(Debug, Clone, Copy)]
pub struct VadResult {
    /// Whether voice activity was detected
    pub is_voice: bool,
    /// Energy level of the frame (0.0-1.0)
    pub energy_level: f32,
    /// Zero crossing rate (0.0-1.0)
    pub zero_crossing_rate: f32,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
}

/// Voice Activity Detector
pub struct VoiceActivityDetector {
    /// VAD configuration
    config: VadConfig,
    /// Smoothed energy history
    smoothed_energy: f32,
    /// Background noise estimate
    noise_energy: f32,
    /// Hangover counter
    hangover_count: u32,
    /// Frame count for adaptation
    frame_count: u64,
}

impl VoiceActivityDetector {
    /// Create a new VAD with the given configuration
    pub fn new(config: VadConfig) -> Result<Self> {
        debug!("Creating VoiceActivityDetector with config: {:?}", config);
        
        if config.energy_threshold <= 0.0 || config.energy_threshold >= 1.0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "VAD energy threshold must be between 0.0 and 1.0".to_string(),
            }.into());
        }
        
        if config.energy_smoothing < 0.0 || config.energy_smoothing > 1.0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "VAD energy smoothing must be between 0.0 and 1.0".to_string(),
            }.into());
        }
        
        Ok(Self {
            config,
            smoothed_energy: 0.0,
            noise_energy: 0.0,
            hangover_count: 0,
            frame_count: 0,
        })
    }
    
    /// Analyze an audio frame for voice activity
    pub fn analyze_frame(&mut self, frame: &AudioFrame) -> Result<VadResult> {
        if frame.samples.len() < self.config.min_frame_length {
            return Err(AudioProcessingError::InvalidFormat {
                details: format!(
                    "Frame too short for VAD analysis: {} < {}",
                    frame.samples.len(), self.config.min_frame_length
                ),
            }.into());
        }
        
        // Calculate frame energy (RMS)
        let energy = self.calculate_energy(&frame.samples);
        
        // Calculate zero crossing rate
        let zcr = self.calculate_zero_crossing_rate(&frame.samples);
        
        // Update smoothed energy
        if self.frame_count == 0 {
            self.smoothed_energy = energy;
            self.noise_energy = energy;
        } else {
            self.smoothed_energy = self.config.energy_smoothing * self.smoothed_energy 
                + (1.0 - self.config.energy_smoothing) * energy;
        }
        
        // Detect voice activity
        let is_voice = self.detect_voice_activity(energy, zcr);
        
        // Update noise estimate during silence
        if !is_voice && self.hangover_count == 0 {
            self.noise_energy = 0.95 * self.noise_energy + 0.05 * energy;
        }
        
        // Calculate confidence score
        let confidence = self.calculate_confidence(energy, zcr);
        
        self.frame_count += 1;
        
        trace!("VAD: energy={:.4}, zcr={:.4}, voice={}, confidence={:.2}", 
               energy, zcr, is_voice, confidence);
        
        Ok(VadResult {
            is_voice,
            energy_level: energy,
            zero_crossing_rate: zcr,
            confidence,
        })
    }
    
    /// Reset VAD state
    pub fn reset(&mut self) {
        self.smoothed_energy = 0.0;
        self.noise_energy = 0.0;
        self.hangover_count = 0;
        self.frame_count = 0;
        debug!("VAD state reset");
    }
    
    /// Get current noise energy estimate
    pub fn get_noise_energy(&self) -> f32 {
        self.noise_energy
    }
    
    /// Calculate RMS energy of audio samples
    fn calculate_energy(&self, samples: &[i16]) -> f32 {
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
    
    /// Calculate zero crossing rate
    fn calculate_zero_crossing_rate(&self, samples: &[i16]) -> f32 {
        if samples.len() < 2 {
            return 0.0;
        }
        
        let mut crossings = 0;
        for i in 1..samples.len() {
            if (samples[i-1] >= 0) != (samples[i] >= 0) {
                crossings += 1;
            }
        }
        
        crossings as f32 / (samples.len() - 1) as f32
    }
    
    /// Detect voice activity based on energy and ZCR
    fn detect_voice_activity(&mut self, energy: f32, zcr: f32) -> bool {
        // Energy-based detection
        let energy_above_threshold = energy > self.config.energy_threshold * 
            (self.noise_energy + 0.001); // Add small constant to avoid division by zero
        
        // ZCR-based detection (speech typically has moderate ZCR)
        let zcr_in_speech_range = zcr > 0.05 && zcr < self.config.zcr_threshold;
        
        // Combined decision
        let current_decision = energy_above_threshold && zcr_in_speech_range;
        
        // Apply hangover logic
        if current_decision {
            self.hangover_count = self.config.hangover_frames;
            true
        } else if self.hangover_count > 0 {
            self.hangover_count -= 1;
            true
        } else {
            false
        }
    }
    
    /// Calculate confidence score for the VAD decision
    fn calculate_confidence(&self, energy: f32, zcr: f32) -> f32 {
        // Energy confidence
        let energy_ratio = if self.noise_energy > 0.0 {
            (energy / self.noise_energy).min(10.0) // Cap at 10x noise level
        } else {
            energy * 100.0 // If no noise estimate, use energy directly
        };
        
        let energy_confidence = (energy_ratio / 3.0).min(1.0); // Normalize
        
        // ZCR confidence (speech-like ZCR gets higher confidence)
        let zcr_confidence = if zcr > 0.05 && zcr < 0.5 {
            1.0 - (zcr - 0.15).abs() * 2.0 // Peak confidence around 15% ZCR
        } else {
            0.0
        };
        
        // Combined confidence
        (energy_confidence * 0.7 + zcr_confidence * 0.3).max(0.0).min(1.0)
    }
} 