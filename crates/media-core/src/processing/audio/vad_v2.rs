//! Advanced Voice Activity Detection (VAD v2)
//!
//! This module implements state-of-the-art VAD using spectral feature extraction,
//! frequency-domain analysis, and ensemble detection methods.

use rustfft::{FftPlanner, num_complex::Complex};
use apodize::hanning_iter;
use std::f32::consts::PI;
use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;

/// Advanced VAD configuration with spectral analysis
#[derive(Debug, Clone)]
pub struct AdvancedVadConfig {
    /// FFT size for spectral analysis (power of 2)
    pub fft_size: usize,
    /// Energy threshold (relative to noise floor)
    pub energy_threshold: f32,
    /// Zero crossing rate threshold
    pub zcr_threshold: f32,
    /// Spectral centroid threshold (Hz)
    pub spectral_centroid_threshold: f32,
    /// Spectral rolloff threshold (0.0-1.0)
    pub spectral_rolloff_threshold: f32,
    /// Spectral flux threshold (frame-to-frame spectral change)
    pub spectral_flux_threshold: f32,
    /// Fundamental frequency range for speech (Hz)
    pub f0_range: (f32, f32),
    /// Energy smoothing factor
    pub energy_smoothing: f32,
    /// Noise floor adaptation rate
    pub noise_adaptation_rate: f32,
    /// Hangover frames
    pub hangover_frames: u32,
    /// Minimum speech duration (frames)
    pub min_speech_duration: u32,
    /// Enable ensemble voting
    pub ensemble_voting: bool,
}

impl Default for AdvancedVadConfig {
    fn default() -> Self {
        Self {
            fft_size: 512,                    // 64ms at 8kHz
            energy_threshold: 2.0,            // 2x noise floor
            zcr_threshold: 0.15,              // 15% zero crossings
            spectral_centroid_threshold: 2000.0, // 2kHz (speech range)
            spectral_rolloff_threshold: 0.85, // 85% of energy below this point
            spectral_flux_threshold: 0.02,    // 2% spectral change
            f0_range: (80.0, 400.0),          // Typical human F0 range
            energy_smoothing: 0.9,            // 90% history
            noise_adaptation_rate: 0.01,      // 1% adaptation per frame
            hangover_frames: 8,               // Longer hangover for better detection
            min_speech_duration: 3,           // Minimum 3 frames for speech
            ensemble_voting: true,            // Use multiple detection methods
        }
    }
}

/// Advanced VAD result with detailed analysis
#[derive(Debug, Clone)]
pub struct AdvancedVadResult {
    /// Voice activity decision
    pub is_voice: bool,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
    /// Energy level
    pub energy_level: f32,
    /// Zero crossing rate
    pub zero_crossing_rate: f32,
    /// Spectral centroid (Hz)
    pub spectral_centroid: f32,
    /// Spectral rolloff (Hz)
    pub spectral_rolloff: f32,
    /// Spectral flux (frame-to-frame change)
    pub spectral_flux: f32,
    /// Detected fundamental frequency (Hz, 0 if not detected)
    pub fundamental_frequency: f32,
    /// Individual detector scores
    pub detector_scores: DetectorScores,
}

/// Scores from individual detection methods
#[derive(Debug, Clone)]
pub struct DetectorScores {
    pub energy_score: f32,
    pub zcr_score: f32,
    pub spectral_score: f32,
    pub pitch_score: f32,
    pub flux_score: f32,
}

/// Advanced Voice Activity Detector with spectral analysis
pub struct AdvancedVoiceActivityDetector {
    config: AdvancedVadConfig,
    
    // FFT processing
    fft_planner: FftPlanner<f32>,
    fft_buffer: Vec<Complex<f32>>,
    window: Vec<f32>,
    
    // Spectral analysis state
    prev_spectrum: Vec<f32>,
    noise_spectrum: Vec<f32>,
    smoothed_spectrum: Vec<f32>,
    
    // Detection state
    smoothed_energy: f32,
    noise_energy: f32,
    hangover_count: u32,
    speech_frame_count: u32,
    frame_count: u64,
    
    // Performance tracking
    sample_rate: f32,
}

impl std::fmt::Debug for AdvancedVoiceActivityDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdvancedVoiceActivityDetector")
            .field("config", &self.config)
            .field("sample_rate", &self.sample_rate)
            .field("frame_count", &self.frame_count)
            .field("noise_energy", &self.noise_energy)
            .field("smoothed_energy", &self.smoothed_energy)
            .finish()
    }
}

impl AdvancedVoiceActivityDetector {
    /// Create new advanced VAD
    pub fn new(config: AdvancedVadConfig, sample_rate: f32) -> Result<Self> {
        debug!("Creating AdvancedVoiceActivityDetector: FFT size={}, sample_rate={}", 
               config.fft_size, sample_rate);
        
        // Validate FFT size is power of 2
        if !config.fft_size.is_power_of_two() || config.fft_size < 64 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "FFT size must be power of 2 and >= 64".to_string(),
            }.into());
        }
        
        // Create FFT planner
        let mut fft_planner = FftPlanner::new();
        let fft_buffer = vec![Complex::new(0.0, 0.0); config.fft_size];
        
        // Generate Hanning window
        let window: Vec<f32> = hanning_iter(config.fft_size)
            .map(|x| x as f32)
            .collect();
        
        // Initialize spectral buffers
        let spectrum_size = config.fft_size / 2 + 1;
        
        Ok(Self {
            config,
            fft_planner,
            fft_buffer,
            window,
            prev_spectrum: vec![0.0; spectrum_size],
            noise_spectrum: vec![1e-10; spectrum_size], // Small initial values
            smoothed_spectrum: vec![0.0; spectrum_size],
            smoothed_energy: 0.0,
            noise_energy: 1e-10,
            hangover_count: 0,
            speech_frame_count: 0,
            frame_count: 0,
            sample_rate,
        })
    }
    
    /// Analyze audio frame with advanced spectral VAD
    pub fn analyze_frame(&mut self, frame: &AudioFrame) -> Result<AdvancedVadResult> {
        if frame.samples.len() < self.config.fft_size {
            return Err(AudioProcessingError::InvalidFormat {
                details: format!("Frame too short: {} < {}", frame.samples.len(), self.config.fft_size),
            }.into());
        }
        
        // Basic time-domain features
        let energy = self.calculate_energy(&frame.samples);
        let zcr = self.calculate_zero_crossing_rate(&frame.samples);
        
        // Spectral analysis
        let spectrum = self.calculate_spectrum(&frame.samples)?;
        let spectral_centroid = self.calculate_spectral_centroid(&spectrum);
        let spectral_rolloff = self.calculate_spectral_rolloff(&spectrum);
        let spectral_flux = self.calculate_spectral_flux(&spectrum);
        
        // Pitch detection (simplified)
        let fundamental_frequency = self.detect_fundamental_frequency(&spectrum);
        
        // Update smoothed values
        self.update_smoothed_values(energy, &spectrum);
        
        // Individual detector scores
        let detector_scores = self.calculate_detector_scores(
            energy, zcr, spectral_centroid, spectral_rolloff, spectral_flux, fundamental_frequency
        );
        
        // Ensemble decision
        let (is_voice, confidence) = if self.config.ensemble_voting {
            self.ensemble_decision(&detector_scores)
        } else {
            self.simple_decision(&detector_scores)
        };
        
        // Apply hangover and minimum duration logic
        let final_decision = self.apply_temporal_logic(is_voice);
        
        // Update noise estimation during silence
        if !final_decision && self.hangover_count == 0 {
            self.update_noise_estimation(energy, &spectrum);
        }
        
        self.frame_count += 1;
        
        trace!("AdvancedVAD: voice={}, conf={:.2}, energy={:.4}, centroid={:.0}Hz", 
               final_decision, confidence, energy, spectral_centroid);
        
        Ok(AdvancedVadResult {
            is_voice: final_decision,
            confidence,
            energy_level: energy,
            zero_crossing_rate: zcr,
            spectral_centroid,
            spectral_rolloff,
            spectral_flux,
            fundamental_frequency,
            detector_scores,
        })
    }
    
    /// Calculate power spectrum using FFT
    fn calculate_spectrum(&mut self, samples: &[i16]) -> Result<Vec<f32>> {
        // Convert to float and apply window
        for (i, (&sample, &window_val)) in samples.iter()
            .zip(self.window.iter())
            .enumerate()
            .take(self.config.fft_size) 
        {
            let normalized = sample as f32 / 32768.0;
            self.fft_buffer[i] = Complex::new(normalized * window_val, 0.0);
        }
        
        // Pad with zeros if needed
        for i in samples.len()..self.config.fft_size {
            self.fft_buffer[i] = Complex::new(0.0, 0.0);
        }
        
        // Perform FFT
        let fft = self.fft_planner.plan_fft_forward(self.config.fft_size);
        fft.process(&mut self.fft_buffer);
        
        // Calculate power spectrum (only positive frequencies)
        let mut spectrum = Vec::with_capacity(self.config.fft_size / 2 + 1);
        for i in 0..=self.config.fft_size / 2 {
            let magnitude = self.fft_buffer[i].norm();
            spectrum.push(magnitude * magnitude);
        }
        
        Ok(spectrum)
    }
    
    /// Calculate spectral centroid (brightness measure)
    fn calculate_spectral_centroid(&self, spectrum: &[f32]) -> f32 {
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        
        for (i, &magnitude) in spectrum.iter().enumerate() {
            let frequency = i as f32 * self.sample_rate / (2.0 * spectrum.len() as f32);
            numerator += frequency * magnitude;
            denominator += magnitude;
        }
        
        if denominator > 0.0 {
            numerator / denominator
        } else {
            0.0
        }
    }
    
    /// Calculate spectral rolloff (frequency below which 85% of energy lies)
    fn calculate_spectral_rolloff(&self, spectrum: &[f32]) -> f32 {
        let total_energy: f32 = spectrum.iter().sum();
        let threshold = total_energy * self.config.spectral_rolloff_threshold;
        
        let mut cumulative_energy = 0.0;
        for (i, &magnitude) in spectrum.iter().enumerate() {
            cumulative_energy += magnitude;
            if cumulative_energy >= threshold {
                return i as f32 * self.sample_rate / (2.0 * spectrum.len() as f32);
            }
        }
        
        self.sample_rate / 2.0 // Nyquist frequency
    }
    
    /// Calculate spectral flux (frame-to-frame spectral change)
    fn calculate_spectral_flux(&self, spectrum: &[f32]) -> f32 {
        if self.frame_count == 0 {
            return 0.0;
        }
        
        let mut flux = 0.0;
        for (current, &previous) in spectrum.iter().zip(self.prev_spectrum.iter()) {
            let diff = current - previous;
            if diff > 0.0 {
                flux += diff;
            }
        }
        
        flux / spectrum.len() as f32
    }
    
    /// Simple fundamental frequency detection using spectral peaks
    fn detect_fundamental_frequency(&self, spectrum: &[f32]) -> f32 {
        let min_bin = ((self.config.f0_range.0 / self.sample_rate) * 2.0 * spectrum.len() as f32) as usize;
        let max_bin = ((self.config.f0_range.1 / self.sample_rate) * 2.0 * spectrum.len() as f32) as usize;
        
        if min_bin >= spectrum.len() || max_bin >= spectrum.len() || min_bin >= max_bin {
            return 0.0;
        }
        
        // Find peak in F0 range
        let mut max_magnitude = 0.0;
        let mut peak_bin = 0;
        
        for i in min_bin..=max_bin.min(spectrum.len() - 1) {
            if spectrum[i] > max_magnitude {
                max_magnitude = spectrum[i];
                peak_bin = i;
            }
        }
        
        if max_magnitude > 0.0 {
            peak_bin as f32 * self.sample_rate / (2.0 * spectrum.len() as f32)
        } else {
            0.0
        }
    }
    
    /// Calculate individual detector scores
    fn calculate_detector_scores(
        &self, energy: f32, zcr: f32, spectral_centroid: f32, 
        spectral_rolloff: f32, spectral_flux: f32, f0: f32
    ) -> DetectorScores {
        // Energy score
        let energy_score = if self.noise_energy > 0.0 {
            ((energy / self.noise_energy) / self.config.energy_threshold).min(1.0)
        } else {
            0.0
        };
        
        // ZCR score (speech has moderate ZCR)
        let zcr_score = if zcr > 0.05 && zcr < self.config.zcr_threshold {
            1.0 - (zcr - 0.1).abs() / 0.1 // Peak at 10% ZCR
        } else {
            0.0
        };
        
        // Spectral score (speech has mid-range centroid)
        let spectral_score = if spectral_centroid > 500.0 && spectral_centroid < self.config.spectral_centroid_threshold {
            1.0
        } else {
            0.0
        };
        
        // Pitch score (presence of F0 in speech range)
        let pitch_score = if f0 >= self.config.f0_range.0 && f0 <= self.config.f0_range.1 {
            1.0
        } else {
            0.0
        };
        
        // Flux score (speech has moderate spectral change)
        let flux_score = if spectral_flux > self.config.spectral_flux_threshold {
            (spectral_flux / (self.config.spectral_flux_threshold * 5.0)).min(1.0)
        } else {
            0.0
        };
        
        DetectorScores {
            energy_score,
            zcr_score,
            spectral_score,
            pitch_score,
            flux_score,
        }
    }
    
    /// Ensemble decision using weighted voting
    fn ensemble_decision(&self, scores: &DetectorScores) -> (bool, f32) {
        // Weighted ensemble
        let weights = [0.3, 0.2, 0.25, 0.15, 0.1]; // Energy, ZCR, Spectral, Pitch, Flux
        let score_values = [
            scores.energy_score,
            scores.zcr_score, 
            scores.spectral_score,
            scores.pitch_score,
            scores.flux_score,
        ];
        
        let weighted_score: f32 = weights.iter()
            .zip(score_values.iter())
            .map(|(w, s)| w * s)
            .sum();
        
        let is_voice = weighted_score > 0.5;
        (is_voice, weighted_score)
    }
    
    /// Simple decision using primary features
    fn simple_decision(&self, scores: &DetectorScores) -> (bool, f32) {
        let combined_score = (scores.energy_score + scores.spectral_score) / 2.0;
        let is_voice = combined_score > 0.5 && scores.zcr_score > 0.0;
        (is_voice, combined_score)
    }
    
    /// Apply temporal logic (hangover, minimum duration)
    fn apply_temporal_logic(&mut self, current_decision: bool) -> bool {
        if current_decision {
            self.hangover_count = self.config.hangover_frames;
            self.speech_frame_count += 1;
            
            // Require minimum speech duration
            self.speech_frame_count >= self.config.min_speech_duration
        } else if self.hangover_count > 0 {
            self.hangover_count -= 1;
            true // Continue voice detection during hangover
        } else {
            self.speech_frame_count = 0;
            false
        }
    }
    
    // Additional helper methods...
    fn calculate_energy(&self, samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum_squares: f64 = samples.iter()
            .map(|&s| (s as f64).powi(2))
            .sum();
        
        ((sum_squares / samples.len() as f64).sqrt() / 32768.0) as f32
    }
    
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
    
    fn update_smoothed_values(&mut self, energy: f32, spectrum: &[f32]) {
        // Update smoothed energy
        if self.frame_count == 0 {
            self.smoothed_energy = energy;
        } else {
            self.smoothed_energy = self.config.energy_smoothing * self.smoothed_energy 
                + (1.0 - self.config.energy_smoothing) * energy;
        }
        
        // Store current spectrum for next frame flux calculation
        self.prev_spectrum.clone_from_slice(spectrum);
    }
    
    fn update_noise_estimation(&mut self, energy: f32, spectrum: &[f32]) {
        // Update noise energy
        self.noise_energy = (1.0 - self.config.noise_adaptation_rate) * self.noise_energy 
            + self.config.noise_adaptation_rate * energy;
        
        // Update noise spectrum
        for (noise_bin, &spectrum_bin) in self.noise_spectrum.iter_mut().zip(spectrum.iter()) {
            *noise_bin = (1.0 - self.config.noise_adaptation_rate) * *noise_bin
                + self.config.noise_adaptation_rate * spectrum_bin;
        }
    }
    
    /// Reset detector state
    pub fn reset(&mut self) {
        self.smoothed_energy = 0.0;
        self.noise_energy = 1e-10;
        self.hangover_count = 0;
        self.speech_frame_count = 0;
        self.frame_count = 0;
        self.prev_spectrum.fill(0.0);
        self.noise_spectrum.fill(1e-10);
        debug!("Advanced VAD state reset");
    }
    
    /// Get current noise level estimation
    pub fn get_noise_energy(&self) -> f32 {
        self.noise_energy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::AudioFrame;

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
    
    fn create_noise_frame(amplitude: f32, sample_rate: u32, duration_ms: u32) -> AudioFrame {
        let samples_per_ms = sample_rate / 1000;
        let total_samples = (duration_ms * samples_per_ms) as usize;
        let mut samples = Vec::with_capacity(total_samples);
        
        for _ in 0..total_samples {
            let noise = (rand::random::<f32>() - 0.5) * amplitude;
            samples.push((noise * 16384.0) as i16);
        }
        
        AudioFrame::new(samples, sample_rate, 1, 0)
    }

    #[test]
    fn test_advanced_vad_creation() {
        let config = AdvancedVadConfig::default();
        let vad = AdvancedVoiceActivityDetector::new(config, 8000.0);
        assert!(vad.is_ok());
        
        // Test invalid FFT size
        let mut invalid_config = AdvancedVadConfig::default();
        invalid_config.fft_size = 100; // Not power of 2
        let vad = AdvancedVoiceActivityDetector::new(invalid_config, 8000.0);
        assert!(vad.is_err());
    }

    #[test]
    fn test_speech_detection() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Test with speech-like signal (fundamental frequency in range)
        let speech_frame = create_test_frame(200.0, 0.5, 8000, 64); // 200Hz tone, should be detected as speech
        let result = vad.analyze_frame(&speech_frame);
        assert!(result.is_ok());
        
        let vad_result = result.unwrap();
        println!("Speech detection - Voice: {}, Confidence: {:.2}, F0: {:.1}Hz, Centroid: {:.1}Hz", 
                 vad_result.is_voice, vad_result.confidence, 
                 vad_result.fundamental_frequency, vad_result.spectral_centroid);
        
        // After sufficient frames, should detect speech
        for _ in 0..10 {
            let _ = vad.analyze_frame(&speech_frame);
        }
        
        let final_result = vad.analyze_frame(&speech_frame).unwrap();
        assert!(final_result.is_voice || final_result.confidence > 0.3);
    }

    #[test]
    fn test_noise_rejection() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Test with noise
        let noise_frame = create_noise_frame(0.1, 8000, 64);
        
        // Process several noise frames
        for _ in 0..10 {
            let result = vad.analyze_frame(&noise_frame).unwrap();
            println!("Noise frame - Voice: {}, Confidence: {:.2}, Energy: {:.4}", 
                     result.is_voice, result.confidence, result.energy_level);
        }
        
        let result = vad.analyze_frame(&noise_frame).unwrap();
        // Noise should not be detected as voice (though it might be initially)
        assert!(result.confidence < 0.8); // Should have low confidence
    }

    #[test]
    fn test_spectral_features() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Test with different frequency content
        let low_freq = create_test_frame(100.0, 0.5, 8000, 64);
        let mid_freq = create_test_frame(1000.0, 0.5, 8000, 64);
        let high_freq = create_test_frame(3000.0, 0.5, 8000, 64);
        
        let low_result = vad.analyze_frame(&low_freq).unwrap();
        let mid_result = vad.analyze_frame(&mid_freq).unwrap();
        let high_result = vad.analyze_frame(&high_freq).unwrap();
        
        // Mid frequency should have higher spectral centroid than low frequency
        assert!(mid_result.spectral_centroid > low_result.spectral_centroid);
        assert!(high_result.spectral_centroid > mid_result.spectral_centroid);
        
        println!("Spectral centroids - Low: {:.1}Hz, Mid: {:.1}Hz, High: {:.1}Hz",
                 low_result.spectral_centroid, mid_result.spectral_centroid, high_result.spectral_centroid);
    }

    #[test]
    fn test_detector_scores() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Create speech-like signal
        let speech_frame = create_test_frame(150.0, 0.3, 8000, 64);
        
        // Process a few frames to establish noise floor
        for _ in 0..5 {
            let _ = vad.analyze_frame(&speech_frame);
        }
        
        let result = vad.analyze_frame(&speech_frame).unwrap();
        let scores = &result.detector_scores;
        
        println!("Detector scores - Energy: {:.2}, ZCR: {:.2}, Spectral: {:.2}, Pitch: {:.2}, Flux: {:.2}",
                 scores.energy_score, scores.zcr_score, scores.spectral_score, 
                 scores.pitch_score, scores.flux_score);
        
        // All scores should be between 0 and 1
        assert!(scores.energy_score >= 0.0 && scores.energy_score <= 1.0);
        assert!(scores.zcr_score >= 0.0 && scores.zcr_score <= 1.0);
        assert!(scores.spectral_score >= 0.0 && scores.spectral_score <= 1.0);
        assert!(scores.pitch_score >= 0.0 && scores.pitch_score <= 1.0);
        assert!(scores.flux_score >= 0.0 && scores.flux_score <= 1.0);
    }

    #[test]
    fn test_ensemble_vs_simple_decision() {
        let mut ensemble_config = AdvancedVadConfig::default();
        ensemble_config.ensemble_voting = true;
        
        let mut simple_config = AdvancedVadConfig::default();
        simple_config.ensemble_voting = false;
        
        let mut ensemble_vad = AdvancedVoiceActivityDetector::new(ensemble_config, 8000.0).unwrap();
        let mut simple_vad = AdvancedVoiceActivityDetector::new(simple_config, 8000.0).unwrap();
        
        let speech_frame = create_test_frame(200.0, 0.4, 8000, 64);
        
        // Process several frames
        for _ in 0..5 {
            let _ = ensemble_vad.analyze_frame(&speech_frame);
            let _ = simple_vad.analyze_frame(&speech_frame);
        }
        
        let ensemble_result = ensemble_vad.analyze_frame(&speech_frame).unwrap();
        let simple_result = simple_vad.analyze_frame(&speech_frame).unwrap();
        
        println!("Ensemble confidence: {:.2}, Simple confidence: {:.2}",
                 ensemble_result.confidence, simple_result.confidence);
        
        // Both should produce valid results
        assert!(ensemble_result.confidence >= 0.0 && ensemble_result.confidence <= 1.0);
        assert!(simple_result.confidence >= 0.0 && simple_result.confidence <= 1.0);
    }

    #[test]
    fn test_frame_too_short() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Create frame that's too short for FFT
        let short_frame = AudioFrame::new(vec![100; 100], 8000, 1, 0); // 100 samples < 512 FFT size
        
        let result = vad.analyze_frame(&short_frame);
        assert!(result.is_err());
    }

    #[test]
    fn test_reset_functionality() {
        let config = AdvancedVadConfig::default();
        let mut vad = AdvancedVoiceActivityDetector::new(config, 8000.0).unwrap();
        
        // Process some frames
        let frame = create_test_frame(200.0, 0.5, 8000, 64);
        for _ in 0..5 {
            let _ = vad.analyze_frame(&frame);
        }
        
        let noise_energy_before = vad.get_noise_energy();
        
        // Reset
        vad.reset();
        
        let noise_energy_after = vad.get_noise_energy();
        assert!(noise_energy_after < noise_energy_before || noise_energy_after == 1e-10);
    }
} 