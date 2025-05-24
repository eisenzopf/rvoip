//! Acoustic Echo Cancellation (AEC)
//!
//! This module implements acoustic echo cancellation to remove echoes from
//! audio signals using adaptive filtering techniques.

use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

/// Configuration for Acoustic Echo Cancellation
#[derive(Debug, Clone)]
pub struct AecConfig {
    /// Filter length (number of taps in adaptive filter)
    pub filter_length: usize,
    /// Adaptation step size (learning rate)
    pub step_size: f32,
    /// Echo suppression factor (0.0-1.0)
    pub suppression_factor: f32,
    /// Minimum echo level before suppression
    pub min_echo_level: f32,
    /// Enable comfort noise generation
    pub comfort_noise: bool,
    /// Double-talk detection threshold
    pub double_talk_threshold: f32,
}

impl Default for AecConfig {
    fn default() -> Self {
        Self {
            filter_length: 256,         // 32ms at 8kHz (reasonable echo delay)
            step_size: 0.01,             // Conservative learning rate
            suppression_factor: 0.7,     // 70% echo suppression
            min_echo_level: 0.001,       // Minimum level to process
            comfort_noise: true,         // Generate comfort noise
            double_talk_threshold: 0.5,  // Threshold for double-talk detection
        }
    }
}

/// Result of AEC processing
#[derive(Debug, Clone, Copy)]
pub struct AecResult {
    /// Echo suppression applied (0.0-1.0)
    pub echo_suppression: f32,
    /// Input signal level
    pub input_level: f32,
    /// Output signal level after AEC
    pub output_level: f32,
    /// Whether double-talk was detected
    pub double_talk_detected: bool,
    /// Echo estimate level
    pub echo_estimate: f32,
}

/// Acoustic Echo Canceller using adaptive filtering
pub struct AcousticEchoCanceller {
    /// AEC configuration
    config: AecConfig,
    /// Adaptive filter coefficients
    filter_coeffs: Vec<f32>,
    /// Reference signal buffer (far-end/playback)
    reference_buffer: Vec<f32>,
    /// Echo estimate buffer
    echo_estimate_buffer: Vec<f32>,
    /// Previous near-end signal level
    prev_near_level: f32,
    /// Previous far-end signal level
    prev_far_level: f32,
    /// Frame count for adaptation
    frame_count: u64,
}

impl AcousticEchoCanceller {
    /// Create a new AEC with the given configuration
    pub fn new(config: AecConfig) -> Result<Self> {
        debug!("Creating AcousticEchoCanceller with config: {:?}", config);
        
        // Validate configuration
        if config.filter_length == 0 || config.filter_length > 1024 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "AEC filter length must be between 1 and 1024".to_string(),
            }.into());
        }
        
        if config.step_size <= 0.0 || config.step_size > 1.0 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "AEC step size must be between 0.0 and 1.0".to_string(),
            }.into());
        }
        
        // Store filter length before moving config
        let filter_length = config.filter_length;
        
        Ok(Self {
            config,
            filter_coeffs: vec![0.0; filter_length],
            reference_buffer: vec![0.0; filter_length],
            echo_estimate_buffer: vec![0.0; 160], // Assume 20ms frames
            prev_near_level: 0.0,
            prev_far_level: 0.0,
            frame_count: 0,
        })
    }
    
    /// Process audio frame with AEC
    /// 
    /// # Arguments
    /// * `near_end` - The captured audio (microphone input with echo)
    /// * `far_end` - The reference audio (speaker output causing echo)
    pub fn process_frame(&mut self, near_end: &AudioFrame, far_end: &AudioFrame) -> Result<AecResult> {
        if near_end.samples.len() != far_end.samples.len() {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Near-end and far-end frames must have same length".to_string(),
            }.into());
        }
        
        if near_end.samples.is_empty() {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Audio frames cannot be empty".to_string(),
            }.into());
        }
        
        // Convert samples to floating point
        let near_samples: Vec<f32> = near_end.samples.iter()
            .map(|&s| s as f32 / 32768.0)
            .collect();
        
        let far_samples: Vec<f32> = far_end.samples.iter()
            .map(|&s| s as f32 / 32768.0)
            .collect();
        
        // Calculate signal levels
        let near_level = self.calculate_rms(&near_samples);
        let far_level = self.calculate_rms(&far_samples);
        
        // Detect double-talk (both near and far-end active)
        let double_talk_detected = self.detect_double_talk(near_level, far_level);
        
        // Update reference buffer with far-end signal
        self.update_reference_buffer(&far_samples);
        
        // Generate echo estimate using adaptive filter
        let echo_estimate = self.generate_echo_estimate();
        let echo_level = self.calculate_rms(&echo_estimate);
        
        // Apply echo cancellation
        let (output_samples, suppression_applied) = self.cancel_echo(&near_samples, &echo_estimate, double_talk_detected);
        
        // Update adaptive filter (only if not double-talk)
        if !double_talk_detected && far_level > self.config.min_echo_level {
            self.update_adaptive_filter(&near_samples, &output_samples);
        }
        
        // Store for next frame
        self.echo_estimate_buffer = echo_estimate;
        self.prev_near_level = near_level;
        self.prev_far_level = far_level;
        self.frame_count += 1;
        
        trace!("AEC: near={:.4}, far={:.4}, echo={:.4}, suppression={:.2}, double_talk={}", 
               near_level, far_level, echo_level, suppression_applied, double_talk_detected);
        
        Ok(AecResult {
            echo_suppression: suppression_applied,
            input_level: near_level,
            output_level: self.calculate_rms(&output_samples),
            double_talk_detected,
            echo_estimate: echo_level,
        })
    }
    
    /// Apply echo cancellation to audio samples
    pub fn apply_cancellation(&self, samples: &mut [i16], output_samples: &[f32]) {
        for (sample, &output) in samples.iter_mut().zip(output_samples.iter()) {
            let adjusted = (output * 32768.0).max(-32768.0).min(32767.0);
            *sample = adjusted as i16;
        }
    }
    
    /// Reset AEC state
    pub fn reset(&mut self) {
        self.filter_coeffs.fill(0.0);
        self.reference_buffer.fill(0.0);
        self.echo_estimate_buffer.fill(0.0);
        self.prev_near_level = 0.0;
        self.prev_far_level = 0.0;
        self.frame_count = 0;
        debug!("AEC state reset");
    }
    
    /// Calculate RMS level of samples
    fn calculate_rms(&self, samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum_squares: f32 = samples.iter().map(|&s| s * s).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }
    
    /// Detect double-talk situation
    fn detect_double_talk(&self, near_level: f32, far_level: f32) -> bool {
        // Simple double-talk detection: both signals above threshold
        near_level > self.config.double_talk_threshold && 
        far_level > self.config.double_talk_threshold
    }
    
    /// Update reference signal buffer
    fn update_reference_buffer(&mut self, far_samples: &[f32]) {
        // Shift buffer and add new samples
        let shift_amount = far_samples.len().min(self.reference_buffer.len());
        
        // Shift existing samples
        for i in 0..(self.reference_buffer.len() - shift_amount) {
            self.reference_buffer[i] = self.reference_buffer[i + shift_amount];
        }
        
        // Add new samples
        let start_idx = self.reference_buffer.len() - shift_amount;
        for (i, &sample) in far_samples.iter().take(shift_amount).enumerate() {
            self.reference_buffer[start_idx + i] = sample;
        }
    }
    
    /// Generate echo estimate using adaptive filter
    fn generate_echo_estimate(&self) -> Vec<f32> {
        let frame_size = self.echo_estimate_buffer.len();
        let mut echo_estimate = vec![0.0; frame_size];
        
        // Convolve filter coefficients with reference buffer
        for i in 0..frame_size {
            let mut sum = 0.0;
            
            for j in 0..self.config.filter_length.min(self.reference_buffer.len()) {
                if i + j < self.reference_buffer.len() {
                    sum += self.filter_coeffs[j] * self.reference_buffer[self.reference_buffer.len() - 1 - i - j];
                }
            }
            
            echo_estimate[i] = sum;
        }
        
        echo_estimate
    }
    
    /// Cancel echo from near-end signal
    fn cancel_echo(&self, near_samples: &[f32], echo_estimate: &[f32], double_talk: bool) -> (Vec<f32>, f32) {
        let mut output = Vec::with_capacity(near_samples.len());
        let mut total_suppression = 0.0;
        
        for (i, (&near, &echo)) in near_samples.iter().zip(echo_estimate.iter()).enumerate() {
            let echo_cancelled = if double_talk {
                // Reduce suppression during double-talk
                near - echo * (self.config.suppression_factor * 0.3)
            } else {
                // Full echo suppression
                near - echo * self.config.suppression_factor
            };
            
            // Apply comfort noise if signal is very quiet
            let final_output = if echo_cancelled.abs() < self.config.min_echo_level && self.config.comfort_noise {
                echo_cancelled + (rand::random::<f32>() - 0.5) * 0.001 // Small comfort noise
            } else {
                echo_cancelled
            };
            
            output.push(final_output);
            total_suppression += (near - final_output).abs();
        }
        
        let avg_suppression = if !near_samples.is_empty() {
            total_suppression / near_samples.len() as f32
        } else {
            0.0
        };
        
        (output, avg_suppression)
    }
    
    /// Update adaptive filter coefficients
    fn update_adaptive_filter(&mut self, near_samples: &[f32], output_samples: &[f32]) {
        // LMS (Least Mean Squares) adaptation
        for i in 0..near_samples.len().min(output_samples.len()) {
            let error = output_samples[i]; // Residual echo after cancellation
            
            // Update filter coefficients
            for j in 0..self.config.filter_length.min(self.reference_buffer.len()) {
                if i + j < self.reference_buffer.len() {
                    let reference_sample = self.reference_buffer[self.reference_buffer.len() - 1 - i - j];
                    self.filter_coeffs[j] += self.config.step_size * error * reference_sample;
                }
            }
        }
    }
} 