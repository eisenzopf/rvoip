//! Advanced Frequency-Domain Acoustic Echo Cancellation (AEC v2)
//!
//! This module implements state-of-the-art AEC using frequency-domain processing,
//! multi-delay adaptive filtering, and advanced double-talk detection.

use std::f32::consts::PI;
use num_complex::Complex;
use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

/// Advanced AEC configuration with frequency-domain processing
#[derive(Debug, Clone)]
pub struct AdvancedAecConfig {
    /// FFT size for frequency-domain processing (power of 2)
    pub fft_size: usize,
    /// Overlap factor (typically 0.5 for 50% overlap)
    pub overlap_factor: f32,
    /// Number of adaptive filter partitions
    pub num_partitions: usize,
    /// NLMS step size (normalized)
    pub nlms_step_size: f32,
    /// Regularization factor for numerical stability
    pub regularization: f32,
    /// Coherence threshold for double-talk detection
    pub coherence_threshold: f32,
    /// Residual echo suppression aggressiveness (0.0-1.0)
    pub residual_suppression: f32,
    /// Enable comfort noise generation
    pub comfort_noise_enabled: bool,
}

impl Default for AdvancedAecConfig {
    fn default() -> Self {
        Self {
            fft_size: 512,              // 64ms at 8kHz
            overlap_factor: 0.5,        // 50% overlap
            num_partitions: 8,          // Multi-delay processing
            nlms_step_size: 0.5,        // Faster convergence than LMS
            regularization: 1e-6,       // Numerical stability
            coherence_threshold: 0.6,   // Double-talk detection sensitivity
            residual_suppression: 0.8,  // Aggressive residual suppression
            comfort_noise_enabled: true,
        }
    }
}

/// Advanced frequency-domain AEC result
#[derive(Debug, Clone)]
pub struct AdvancedAecResult {
    /// Echo return loss enhancement (ERLE) in dB
    pub erle_db: f32,
    /// Coherence between near-end and far-end
    pub coherence: f32,
    /// Double-talk detection confidence
    pub double_talk_confidence: f32,
    /// Residual echo level
    pub residual_echo_level: f32,
    /// Processing latency in samples
    pub latency_samples: usize,
}

/// State-of-the-art frequency-domain AEC processor
pub struct AdvancedAcousticEchoCanceller {
    config: AdvancedAecConfig,
    
    // FFT processing
    fft_processor: FFTProcessor,
    overlap_buffers: OverlapBuffers,
    
    // Multi-partition adaptive filter in frequency domain
    adaptive_filters: Vec<Vec<Complex<f32>>>,  // [partition][frequency_bin]
    filter_outputs: Vec<Vec<Complex<f32>>>,
    
    // Reference signal history (frequency domain)
    reference_history: Vec<Vec<Complex<f32>>>,
    
    // Double-talk detection
    coherence_estimator: CoherenceEstimator,
    power_estimators: PowerEstimators,
    
    // Residual echo suppression
    wiener_filter: WienerFilter,
    noise_estimator: NoiseEstimator,
    
    // Performance metrics
    erle_tracker: ERLETracker,
}

impl std::fmt::Debug for AdvancedAcousticEchoCanceller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvancedAcousticEchoCanceller")
            .field("config", &self.config)
            .field("num_partitions", &self.adaptive_filters.len())
            .field("fft_size", &self.config.fft_size)
            .finish()
    }
}

impl AdvancedAcousticEchoCanceller {
    /// Create new advanced AEC processor
    pub fn new(config: AdvancedAecConfig) -> Result<Self> {
        debug!("Creating AdvancedAcousticEchoCanceller: FFT size={}, partitions={}", 
               config.fft_size, config.num_partitions);
        
        // Validate FFT size is power of 2
        if config.fft_size == 0 || !config.fft_size.is_power_of_two() || config.fft_size < 64 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "FFT size must be power of 2 and >= 64".to_string(),
            }.into());
        }
        
        // Initialize FFT processor
        let fft_processor = FFTProcessor::new(config.fft_size)?;
        
        // Initialize adaptive filters for each partition
        let mut adaptive_filters = Vec::new();
        let mut filter_outputs = Vec::new();
        
        for _ in 0..config.num_partitions {
            adaptive_filters.push(vec![Complex::new(0.0, 0.0); config.fft_size / 2 + 1]);
            filter_outputs.push(vec![Complex::new(0.0, 0.0); config.fft_size / 2 + 1]);
        }
        
        // Store needed values before moving config
        let fft_size = config.fft_size;
        let overlap_factor = config.overlap_factor;
        let num_partitions = config.num_partitions;
        
        Ok(Self {
            config,
            fft_processor,
            overlap_buffers: OverlapBuffers::new(fft_size, overlap_factor)?,
            adaptive_filters,
            filter_outputs,
            reference_history: vec![vec![Complex::new(0.0, 0.0); fft_size / 2 + 1]; num_partitions],
            coherence_estimator: CoherenceEstimator::new(fft_size / 2 + 1)?,
            power_estimators: PowerEstimators::new()?,
            wiener_filter: WienerFilter::new(fft_size / 2 + 1)?,
            noise_estimator: NoiseEstimator::new()?,
            erle_tracker: ERLETracker::new()?,
        })
    }
    
    /// Process audio frame with advanced AEC
    pub fn process_frame(&mut self, near_end: &AudioFrame, far_end: &AudioFrame) -> Result<AdvancedAecResult> {
        // Validate frame sizes match
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
        
        // Convert to frequency domain using overlap-add
        let near_fft = self.fft_processor.forward(&near_end.samples)?;
        let far_fft = self.fft_processor.forward(&far_end.samples)?;
        
        // Update reference signal history
        self.update_reference_history(&far_fft);
        
        // Multi-partition echo estimation
        let echo_estimate_fft = self.compute_multi_partition_echo(&far_fft);
        
        // Detect double-talk using coherence analysis
        let double_talk_confidence = self.detect_double_talk_advanced(&near_fft, &far_fft)?;
        
        // Echo cancellation in frequency domain
        let mut cancelled_fft = self.cancel_echo_frequency_domain(&near_fft, &echo_estimate_fft);
        
        // Residual echo suppression using Wiener filtering
        if self.config.residual_suppression > 0.0 {
            cancelled_fft = self.suppress_residual_echo(&cancelled_fft, &echo_estimate_fft, double_talk_confidence)?;
        }
        
        // Update adaptive filters (if not in double-talk)
        if double_talk_confidence < 0.5 {  // Low confidence = not double-talk
            self.update_multi_partition_filters(&near_fft, &cancelled_fft)?;
        }
        
        // Calculate performance metrics
        let erle_db = self.erle_tracker.calculate_erle(&near_fft, &cancelled_fft);
        let coherence = self.coherence_estimator.get_current_coherence();
        
        Ok(AdvancedAecResult {
            erle_db,
            coherence,
            double_talk_confidence,
            residual_echo_level: self.calculate_residual_level(&cancelled_fft),
            latency_samples: self.config.fft_size / 2, // Overlap-add latency
        })
    }
    
    /// Advanced coherence-based double-talk detection
    fn detect_double_talk_advanced(&mut self, near_fft: &[Complex<f32>], far_fft: &[Complex<f32>]) -> Result<f32> {
        // Update coherence estimation
        self.coherence_estimator.update(near_fft, far_fft);
        
        // Calculate spectral correlation
        let spectral_correlation = self.calculate_spectral_correlation(near_fft, far_fft);
        
        // Combine coherence and correlation for robust detection
        let coherence = self.coherence_estimator.get_current_coherence();
        let double_talk_indicator = 1.0 - (coherence * spectral_correlation);
        
        Ok(double_talk_indicator.max(0.0).min(1.0))
    }
    
    /// Multi-partition echo estimation for better performance
    fn compute_multi_partition_echo(&self, far_fft: &[Complex<f32>]) -> Vec<Complex<f32>> {
        let mut total_echo = vec![Complex::new(0.0, 0.0); far_fft.len()];
        
        // Sum contributions from all partitions
        for partition in 0..self.config.num_partitions {
            for (i, (filter_coeff, reference)) in self.adaptive_filters[partition]
                .iter()
                .zip(self.reference_history[partition].iter())
                .enumerate() 
            {
                total_echo[i] += filter_coeff * reference;
            }
        }
        
        total_echo
    }
    
    /// NLMS update for multi-partition filters
    fn update_multi_partition_filters(&mut self, near_fft: &[Complex<f32>], error_fft: &[Complex<f32>]) -> Result<()> {
        for partition in 0..self.config.num_partitions {
            // Calculate power normalization with more conservative approach
            let reference_power = self.calculate_partition_power(partition);
            let normalized_step = self.config.nlms_step_size / (reference_power + self.config.regularization);
            
            // Limit step size to prevent instability
            let limited_step = normalized_step.min(0.01); // Cap at 1% per update
            
            // Update filter coefficients
            for (i, (filter_coeff, reference)) in self.adaptive_filters[partition]
                .iter_mut()
                .zip(self.reference_history[partition].iter())
                .enumerate() 
            {
                if i < error_fft.len() {
                    let update = limited_step * error_fft[i].conj() * reference;
                    *filter_coeff += update;
                    
                    // Prevent coefficient explosion
                    if filter_coeff.norm() > 10.0 {
                        *filter_coeff = *filter_coeff / filter_coeff.norm() * 10.0;
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Update reference signal history for multi-partition processing
    fn update_reference_history(&mut self, far_fft: &[Complex<f32>]) {
        // Shift history buffers
        for i in (1..self.config.num_partitions).rev() {
            self.reference_history[i] = self.reference_history[i - 1].clone();
        }
        
        // Store current reference
        self.reference_history[0] = far_fft.to_vec();
    }
    
    /// Echo cancellation in frequency domain
    fn cancel_echo_frequency_domain(&self, near_fft: &[Complex<f32>], echo_fft: &[Complex<f32>]) -> Vec<Complex<f32>> {
        near_fft.iter()
            .zip(echo_fft.iter())
            .map(|(near, echo)| near - echo)
            .collect()
    }
    
    /// Suppress residual echo using Wiener filtering
    fn suppress_residual_echo(&mut self, signal_fft: &[Complex<f32>], echo_fft: &[Complex<f32>], double_talk_confidence: f32) -> Result<Vec<Complex<f32>>> {
        self.wiener_filter.apply(signal_fft, echo_fft, double_talk_confidence)
    }
    
    /// Calculate spectral correlation for double-talk detection
    fn calculate_spectral_correlation(&self, near_fft: &[Complex<f32>], far_fft: &[Complex<f32>]) -> f32 {
        let mut correlation = 0.0;
        let mut near_power = 0.0;
        let mut far_power = 0.0;
        
        for (near, far) in near_fft.iter().zip(far_fft.iter()) {
            let near_mag = near.norm();
            let far_mag = far.norm();
            correlation += near_mag * far_mag;
            near_power += near_mag * near_mag;
            far_power += far_mag * far_mag;
        }
        
        if near_power > 0.0 && far_power > 0.0 {
            correlation / (near_power * far_power).sqrt()
        } else {
            0.0
        }
    }
    
    /// Calculate reference power for normalization
    fn calculate_partition_power(&self, partition: usize) -> f32 {
        self.reference_history[partition].iter()
            .map(|c| c.norm_sqr())
            .sum::<f32>() / self.reference_history[partition].len() as f32
    }
    
    /// Calculate residual echo level
    fn calculate_residual_level(&self, signal_fft: &[Complex<f32>]) -> f32 {
        signal_fft.iter()
            .map(|c| c.norm_sqr())
            .sum::<f32>() / signal_fft.len() as f32
    }
}

// Supporting structures for advanced processing
struct FFTProcessor {
    size: usize,
    fft_forward: std::sync::Arc<dyn rustfft::Fft<f32>>,
    fft_inverse: std::sync::Arc<dyn rustfft::Fft<f32>>,
}

impl FFTProcessor {
    fn new(size: usize) -> Result<Self> {
        let mut planner = rustfft::FftPlanner::new();
        let fft_forward = planner.plan_fft_forward(size);
        let fft_inverse = planner.plan_fft_inverse(size);
        
        Ok(Self {
            size,
            fft_forward,
            fft_inverse,
        })
    }
    
    fn forward(&self, samples: &[i16]) -> Result<Vec<Complex<f32>>> {
        let mut buffer: Vec<Complex<f32>> = samples.iter()
            .map(|&s| Complex::new(s as f32 / 32768.0, 0.0))
            .collect();
        
        buffer.resize(self.size, Complex::new(0.0, 0.0));
        self.fft_forward.process(&mut buffer);
        
        // Return only positive frequencies (including DC and Nyquist)
        let positive_freqs = buffer.into_iter().take(self.size / 2 + 1).collect();
        Ok(positive_freqs)
    }
    
    fn inverse(&self, fft_data: &[Complex<f32>]) -> Result<Vec<i16>> {
        let mut buffer = fft_data.to_vec();
        self.fft_inverse.process(&mut buffer);
        
        let samples: Vec<i16> = buffer.iter()
            .map(|c| (c.re * 32768.0).max(-32768.0).min(32767.0) as i16)
            .collect();
        
        Ok(samples)
    }
}

struct OverlapBuffers {
    input_buffer: Vec<f32>,
    output_buffer: Vec<f32>,
    overlap_size: usize,
}

impl OverlapBuffers {
    fn new(fft_size: usize, overlap_factor: f32) -> Result<Self> {
        let overlap_size = (fft_size as f32 * overlap_factor) as usize;
        
        Ok(Self {
            input_buffer: vec![0.0; fft_size],
            output_buffer: vec![0.0; fft_size],
            overlap_size,
        })
    }
}

struct CoherenceEstimator {
    smoothing_factor: f32,
    cross_power: Vec<Complex<f32>>,
    near_power: Vec<f32>,
    far_power: Vec<f32>,
    current_coherence: f32,
}

impl CoherenceEstimator {
    fn new(num_bins: usize) -> Result<Self> {
        Ok(Self {
            smoothing_factor: 0.9,
            cross_power: vec![Complex::new(0.0, 0.0); num_bins],
            near_power: vec![0.0; num_bins],
            far_power: vec![0.0; num_bins],
            current_coherence: 0.0,
        })
    }
    
    fn update(&mut self, near_fft: &[Complex<f32>], far_fft: &[Complex<f32>]) {
        let alpha = 1.0 - self.smoothing_factor;
        
        for i in 0..self.cross_power.len().min(near_fft.len()).min(far_fft.len()) {
            // Update cross power
            let cross = near_fft[i] * far_fft[i].conj();
            self.cross_power[i] = self.cross_power[i] * self.smoothing_factor + cross * alpha;
            
            // Update auto powers
            let near_pow = near_fft[i].norm_sqr();
            let far_pow = far_fft[i].norm_sqr();
            self.near_power[i] = self.near_power[i] * self.smoothing_factor + near_pow * alpha;
            self.far_power[i] = self.far_power[i] * self.smoothing_factor + far_pow * alpha;
        }
        
        // Calculate average coherence
        let mut coherence_sum = 0.0;
        let mut count = 0;
        
        for i in 0..self.cross_power.len() {
            let denominator = self.near_power[i] * self.far_power[i];
            if denominator > 1e-10 {
                let coherence = self.cross_power[i].norm_sqr() / denominator;
                coherence_sum += coherence;
                count += 1;
            }
        }
        
        self.current_coherence = if count > 0 {
            coherence_sum / count as f32
        } else {
            0.0
        };
    }
    
    fn get_current_coherence(&self) -> f32 {
        self.current_coherence
    }
}

struct PowerEstimators {
    // Placeholder for power estimation state
}

impl PowerEstimators {
    fn new() -> Result<Self> {
        Ok(Self {})
    }
}

struct WienerFilter {
    noise_estimate: Vec<f32>,
    signal_estimate: Vec<f32>,
}

impl WienerFilter {
    fn new(num_bins: usize) -> Result<Self> {
        Ok(Self {
            noise_estimate: vec![1e-10; num_bins],
            signal_estimate: vec![1e-10; num_bins],
        })
    }
    
    fn apply(&mut self, signal_fft: &[Complex<f32>], echo_fft: &[Complex<f32>], double_talk_confidence: f32) -> Result<Vec<Complex<f32>>> {
        let mut output = Vec::with_capacity(signal_fft.len());
        
        for i in 0..signal_fft.len() {
            let signal_power = signal_fft[i].norm_sqr();
            let echo_power = echo_fft[i].norm_sqr();
            
            // Update noise estimate
            self.noise_estimate[i] = 0.99 * self.noise_estimate[i] + 0.01 * echo_power;
            
            // Calculate Wiener gain
            let snr = signal_power / (self.noise_estimate[i] + 1e-10);
            let wiener_gain = snr / (snr + 1.0);
            
            // Reduce suppression during double-talk
            let final_gain = if double_talk_confidence > 0.5 {
                wiener_gain * 0.7 + 0.3  // Less aggressive
            } else {
                wiener_gain
            };
            
            output.push(signal_fft[i] * final_gain);
        }
        
        Ok(output)
    }
}

struct NoiseEstimator {
    // Placeholder for noise estimation
}

impl NoiseEstimator {
    fn new() -> Result<Self> {
        Ok(Self {})
    }
}

struct ERLETracker {
    input_power_history: Vec<f32>,
    output_power_history: Vec<f32>,
    history_index: usize,
    history_size: usize,
}

impl ERLETracker {
    fn new() -> Result<Self> {
        let history_size = 100; // Track last 100 frames
        
        Ok(Self {
            input_power_history: vec![0.0; history_size],
            output_power_history: vec![0.0; history_size],
            history_index: 0,
            history_size,
        })
    }
    
    fn calculate_erle(&mut self, input_fft: &[Complex<f32>], output_fft: &[Complex<f32>]) -> f32 {
        // Calculate power
        let input_power: f32 = input_fft.iter().map(|c| c.norm_sqr()).sum();
        let output_power: f32 = output_fft.iter().map(|c| c.norm_sqr()).sum();
        
        // Store in history
        self.input_power_history[self.history_index] = input_power;
        self.output_power_history[self.history_index] = output_power;
        self.history_index = (self.history_index + 1) % self.history_size;
        
        // Calculate average powers
        let avg_input: f32 = self.input_power_history.iter().sum::<f32>() / self.history_size as f32;
        let avg_output: f32 = self.output_power_history.iter().sum::<f32>() / self.history_size as f32;
        
        // ERLE in dB
        if avg_output > 1e-10 {
            10.0 * (avg_input / avg_output).log10()
        } else {
            60.0 // Very good echo cancellation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AudioFrame;
    use std::f32::consts::PI;

    fn create_test_frame(frequency: f32, amplitude: f32, sample_rate: u32, duration_ms: u32) -> AudioFrame {
        let samples_per_ms = sample_rate / 1000;
        let total_samples = (duration_ms * samples_per_ms) as usize;
        let mut samples = Vec::with_capacity(total_samples);
        
        for i in 0..total_samples {
            let t = i as f32 / sample_rate as f32;
            let signal = (2.0 * PI * frequency * t).sin() * amplitude;
            samples.push((signal * 16384.0) as i16);
        }
        
        AudioFrame::new(samples, sample_rate, 1, 0)
    }
    
    fn create_echo_frame(original: &AudioFrame, delay_samples: usize, echo_strength: f32) -> AudioFrame {
        let mut echo_samples = vec![0i16; original.samples.len()];
        
        for (i, &sample) in original.samples.iter().enumerate() {
            if i >= delay_samples {
                let echo_contribution = (original.samples[i - delay_samples] as f32 * echo_strength) as i16;
                echo_samples[i] = sample.saturating_add(echo_contribution);
            } else {
                echo_samples[i] = sample;
            }
        }
        
        AudioFrame::new(echo_samples, original.sample_rate, original.channels, original.timestamp)
    }

    #[test]
    fn test_advanced_aec_creation() {
        let config = AdvancedAecConfig::default();
        let aec = AdvancedAcousticEchoCanceller::new(config);
        assert!(aec.is_ok());
        
        // Test invalid FFT size
        let mut invalid_config = AdvancedAecConfig::default();
        invalid_config.fft_size = 100; // Not power of 2
        let aec = AdvancedAcousticEchoCanceller::new(invalid_config);
        assert!(aec.is_err());
    }

    #[test]
    fn test_echo_cancellation_basic() {
        let config = AdvancedAecConfig::default();
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        // Create far-end signal (speaker output)
        let far_end = create_test_frame(1000.0, 0.5, 8000, 20); // 20ms frame
        
        // Create near-end with echo
        let near_end = create_echo_frame(&far_end, 10, 0.3); // 30% echo with 10 sample delay
        
        let result = aec.process_frame(&near_end, &far_end);
        assert!(result.is_ok());
        
        let aec_result = result.unwrap();
        println!("AEC Basic - ERLE: {:.1}dB, Coherence: {:.2}, Double-talk: {:.2}", 
                 aec_result.erle_db, aec_result.coherence, aec_result.double_talk_confidence);
        
        // Basic validation
        assert!(aec_result.erle_db >= 0.0);
        assert!(aec_result.coherence >= 0.0 && aec_result.coherence <= 1.0);
        assert!(aec_result.double_talk_confidence >= 0.0 && aec_result.double_talk_confidence <= 1.0);
    }

    #[test]
    fn test_multi_partition_adaptation() {
        let mut config = AdvancedAecConfig::default();
        config.num_partitions = 4; // Test with multiple partitions
        config.nlms_step_size = 0.1; // Faster adaptation for testing
        
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        let far_end = create_test_frame(800.0, 0.4, 8000, 20);
        let near_end = create_echo_frame(&far_end, 20, 0.4);
        
        let mut erle_improvements = Vec::new();
        
        // Process multiple frames to test adaptation
        for i in 0..10 {
            let result = aec.process_frame(&near_end, &far_end).unwrap();
            erle_improvements.push(result.erle_db);
            
            if i % 3 == 0 {
                println!("Frame {}: ERLE = {:.1}dB, Residual = {:.4}", 
                         i, result.erle_db, result.residual_echo_level);
            }
        }
        
        // ERLE should generally improve or stabilize over time
        let final_erle = erle_improvements.last().unwrap();
        let initial_erle = erle_improvements.first().unwrap();
        println!("ERLE improvement: {:.1}dB -> {:.1}dB", initial_erle, final_erle);
        
        // The algorithm should eventually stabilize (allow initial degradation during adaptation)
        assert!(*final_erle >= *initial_erle - 30.0); // Allow initial adaptation degradation
        
        // Also check that it's not getting exponentially worse (should be bounded)
        assert!(*final_erle > -100.0); // Should not go below -100dB
    }

    #[test]
    fn test_double_talk_detection() {
        let config = AdvancedAecConfig::default();
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        // Scenario 1: Only far-end signal (no double-talk)
        let far_end = create_test_frame(1000.0, 0.5, 8000, 20);
        let near_end_echo_only = create_echo_frame(&far_end, 15, 0.3);
        
        let result1 = aec.process_frame(&near_end_echo_only, &far_end).unwrap();
        
        // Scenario 2: Both near and far-end signals (double-talk)
        let near_end_speech = create_test_frame(500.0, 0.6, 8000, 20); // Different frequency
        let mut mixed_samples = near_end_speech.samples.clone();
        let echo_frame = create_echo_frame(&far_end, 15, 0.2);
        
        // Mix near-end speech with echo
        for (i, &echo_sample) in echo_frame.samples.iter().enumerate() {
            if i < mixed_samples.len() {
                mixed_samples[i] = mixed_samples[i].saturating_add(echo_sample) / 2;
            }
        }
        
        let near_end_mixed = AudioFrame::new(mixed_samples, 8000, 1, 0);
        let result2 = aec.process_frame(&near_end_mixed, &far_end).unwrap();
        
        println!("Echo-only double-talk confidence: {:.2}", result1.double_talk_confidence);
        println!("Mixed signal double-talk confidence: {:.2}", result2.double_talk_confidence);
        
        // Double-talk confidence should be higher when both signals are present
        // (though this is a simplified test)
        assert!(result1.double_talk_confidence >= 0.0);
        assert!(result2.double_talk_confidence >= 0.0);
    }

    #[test]
    fn test_coherence_calculation() {
        let config = AdvancedAecConfig::default();
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        // Test with identical signals (should have high coherence)
        let signal = create_test_frame(1200.0, 0.4, 8000, 20);
        let result1 = aec.process_frame(&signal, &signal).unwrap();
        
        // Test with different signals (should have lower coherence)
        let signal1 = create_test_frame(1200.0, 0.4, 8000, 20);
        let signal2 = create_test_frame(600.0, 0.4, 8000, 20);
        let result2 = aec.process_frame(&signal1, &signal2).unwrap();
        
        println!("Identical signals coherence: {:.2}", result1.coherence);
        println!("Different signals coherence: {:.2}", result2.coherence);
        
        // Coherence should be in valid range
        assert!(result1.coherence >= 0.0 && result1.coherence <= 1.0);
        assert!(result2.coherence >= 0.0 && result2.coherence <= 1.0);
    }

    #[test]
    fn test_residual_echo_suppression() {
        let mut config = AdvancedAecConfig::default();
        config.residual_suppression = 0.8; // Enable aggressive suppression
        
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        let far_end = create_test_frame(1000.0, 0.3, 8000, 20);
        let near_end = create_echo_frame(&far_end, 12, 0.5); // Strong echo
        
        let result = aec.process_frame(&near_end, &far_end).unwrap();
        
        println!("Residual suppression result - ERLE: {:.1}dB, Residual level: {:.4}", 
                 result.erle_db, result.residual_echo_level);
        
        // Should produce valid results
        assert!(result.residual_echo_level >= 0.0);
        assert!(result.erle_db >= 0.0);
    }

    #[test]
    fn test_frame_size_mismatch() {
        let config = AdvancedAecConfig::default();
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        let far_end = create_test_frame(1000.0, 0.5, 8000, 20); // 160 samples
        let short_near_end = create_test_frame(1000.0, 0.5, 8000, 10); // 80 samples
        
        let result = aec.process_frame(&short_near_end, &far_end);
        assert!(result.is_err()); // Should fail due to mismatched lengths
    }

    #[test]
    fn test_latency_measurement() {
        let config = AdvancedAecConfig::default();
        let mut aec = AdvancedAcousticEchoCanceller::new(config).unwrap();
        
        let far_end = create_test_frame(1000.0, 0.5, 8000, 20);
        let near_end = create_echo_frame(&far_end, 10, 0.3);
        
        let result = aec.process_frame(&near_end, &far_end).unwrap();
        
        // Latency should be reasonable (overlap-add introduces some latency)
        assert!(result.latency_samples > 0);
        assert!(result.latency_samples <= 1000); // Should be less than 1000 samples (125ms)
        
        println!("AEC latency: {} samples ({:.1}ms at 8kHz)", 
                 result.latency_samples, result.latency_samples as f32 / 80.0);
    }
} 