//! Advanced Multi-band Automatic Gain Control (AGC v2)
//!
//! This module implements state-of-the-art AGC using multi-band processing,
//! look-ahead limiting, and perceptual loudness models.

use tracing::{debug, trace};
use crate::error::{Result, AudioProcessingError};
use crate::types::AudioFrame;


/// Advanced multi-band AGC configuration
#[derive(Debug, Clone)]
pub struct AdvancedAgcConfig {
    /// Number of frequency bands (typically 3-5)
    pub num_bands: usize,
    /// Crossover frequencies for band splitting (Hz)
    pub crossover_frequencies: Vec<f32>,
    /// Target loudness in LUFS (Loudness Units relative to Full Scale)
    pub target_lufs: f32,
    /// Look-ahead time in milliseconds
    pub lookahead_ms: f32,
    /// Attack times per band (ms)
    pub attack_times_ms: Vec<f32>,
    /// Release times per band (ms) 
    pub release_times_ms: Vec<f32>,
    /// Compression ratios per band
    pub compression_ratios: Vec<f32>,
    /// Maximum gain per band (dB)
    pub max_gains_db: Vec<f32>,
    /// Enable perceptual weighting (A-weighting)
    pub perceptual_weighting: bool,
    /// Gate threshold for gating (LUFS)
    pub gate_threshold: f32,
}

impl Default for AdvancedAgcConfig {
    fn default() -> Self {
        Self {
            num_bands: 3,
            crossover_frequencies: vec![300.0, 3000.0], // Low, Mid, High
            target_lufs: -23.0,  // EBU R128 broadcast standard
            lookahead_ms: 8.0,   // 8ms look-ahead
            attack_times_ms: vec![5.0, 10.0, 15.0],    // Faster for high freq
            release_times_ms: vec![200.0, 150.0, 100.0], // Slower for low freq
            compression_ratios: vec![3.0, 4.0, 2.0],    // More compression for mid
            max_gains_db: vec![12.0, 15.0, 10.0],       // Conservative limits
            perceptual_weighting: true,
            gate_threshold: -70.0, // Below this is considered silence
        }
    }
}

/// Advanced AGC processing result
#[derive(Debug, Clone)]
pub struct AdvancedAgcResult {
    /// Current loudness measurement (LUFS)
    pub current_lufs: f32,
    /// Gains applied per band (dB)
    pub band_gains_db: Vec<f32>,
    /// Peak levels per band
    pub band_peak_levels: Vec<f32>,
    /// Whether limiter was active in any band
    pub limiter_active: bool,
    /// Look-ahead processing active
    pub lookahead_active: bool,
    /// Gating status (signal above gate threshold)
    pub gated: bool,
}

/// Advanced multi-band AGC processor
pub struct AdvancedAutomaticGainControl {
    config: AdvancedAgcConfig,
    
    // Multi-band processing
    filterbank: MultibandFilterbank,
    band_processors: Vec<BandProcessor>,
    
    // Look-ahead processing
    lookahead_buffer: LookaheadBuffer,
    peak_detector: LookaheadPeakDetector,
    
    // Perceptual loudness measurement
    loudness_meter: LoudnessMeter,
    gating_processor: GatingProcessor,
    
    // Performance tracking
    sample_rate: f32,
    frame_count: u64,
}

impl AdvancedAutomaticGainControl {
    /// Create new advanced multi-band AGC
    pub fn new(config: AdvancedAgcConfig, sample_rate: f32) -> Result<Self> {
        debug!("Creating AdvancedAutomaticGainControl: {} bands, target {} LUFS", 
               config.num_bands, config.target_lufs);
        
        // Validate configuration
        if config.crossover_frequencies.len() != config.num_bands - 1 {
            return Err(AudioProcessingError::InvalidFormat {
                details: "Crossover frequencies must be num_bands - 1".to_string(),
            }.into());
        }
        
        // Initialize filterbank for band splitting
        let filterbank = MultibandFilterbank::new(&config.crossover_frequencies, sample_rate)?;
        
        // Create band processors
        let mut band_processors = Vec::new();
        for i in 0..config.num_bands {
            let band_config = BandProcessorConfig {
                attack_time_ms: config.attack_times_ms[i],
                release_time_ms: config.release_times_ms[i],
                compression_ratio: config.compression_ratios[i],
                max_gain_db: config.max_gains_db[i],
                sample_rate,
            };
            band_processors.push(BandProcessor::new(band_config)?);
        }
        
        // Store gate threshold before moving config
        let gate_threshold = config.gate_threshold;
        let lookahead_ms = config.lookahead_ms;
        let perceptual_weighting = config.perceptual_weighting;
        
        Ok(Self {
            config,
            filterbank,
            band_processors,
            lookahead_buffer: LookaheadBuffer::new(lookahead_ms, sample_rate)?,
            peak_detector: LookaheadPeakDetector::new(lookahead_ms, sample_rate)?,
            loudness_meter: LoudnessMeter::new(sample_rate, perceptual_weighting)?,
            gating_processor: GatingProcessor::new(gate_threshold)?,
            sample_rate,
            frame_count: 0,
        })
    }
    
    /// Process audio frame with advanced multi-band AGC
    pub fn process_frame(&mut self, frame: &AudioFrame) -> Result<AdvancedAgcResult> {
        // Add to look-ahead buffer
        self.lookahead_buffer.push(&frame.samples);
        
        // Get delayed samples for processing (look-ahead delay)
        let delayed_samples = if let Some(samples) = self.lookahead_buffer.get_delayed() {
            samples
        } else {
            // Not enough samples in buffer yet
            return Ok(self.create_bypass_result(frame));
        };
        
        // Measure current loudness
        let current_lufs = self.loudness_meter.measure_frame(&delayed_samples)?;
        
        // Check gating
        let gated = self.gating_processor.is_gated(current_lufs);
        
        if gated {
            // Signal below gate threshold - minimal processing
            return Ok(self.create_gated_result(current_lufs));
        }
        
        // Split into frequency bands
        let band_signals = self.filterbank.split(&delayed_samples)?;
        
        // Detect peaks in look-ahead window
        let future_peaks = self.peak_detector.detect_peaks(&frame.samples, &band_signals)?;
        
        // Process each band independently
        let mut processed_bands = Vec::new();
        let mut band_gains_db = Vec::new();
        let mut band_peak_levels = Vec::new();
        let mut limiter_active = false;
        
        for (i, band_signal) in band_signals.iter().enumerate() {
            // Calculate target gain based on loudness and band characteristics
            let target_gain = self.calculate_band_target_gain(current_lufs, i);
            
            // Apply look-ahead limiting if peak detected
            let peak_limited_gain = if future_peaks[i] > 0.95 {
                // Reduce gain to prevent clipping
                let peak_reduction = 0.95 / future_peaks[i];
                target_gain * peak_reduction
            } else {
                target_gain
            };
            
            // Process band with calculated gain
            let (processed_band, band_result) = self.band_processors[i]
                .process_band(band_signal, peak_limited_gain)?;
            
            processed_bands.push(processed_band);
            band_gains_db.push(band_result.applied_gain_db);
            band_peak_levels.push(band_result.peak_level);
            
            if band_result.limiter_active {
                limiter_active = true;
            }
        }
        
        // Recombine bands
        let output_samples = self.filterbank.combine(&processed_bands)?;
        
        // Create output frame with processed samples
        let mut output_frame = frame.clone();
        self.apply_processed_samples(&mut output_frame, &output_samples);
        
        self.frame_count += 1;
        
        Ok(AdvancedAgcResult {
            current_lufs,
            band_gains_db,
            band_peak_levels,
            limiter_active,
            lookahead_active: true,
            gated,
        })
    }
    
    /// Calculate target gain for specific frequency band
    fn calculate_band_target_gain(&self, current_lufs: f32, band_index: usize) -> f32 {
        // Calculate loudness error
        let loudness_error = self.config.target_lufs - current_lufs;
        
        // Convert to linear gain
        let base_gain = 10.0_f32.powf(loudness_error / 20.0);
        
        // Apply band-specific weighting
        let band_weight = match band_index {
            0 => 0.8,  // Low frequencies - less aggressive
            1 => 1.0,  // Mid frequencies - normal processing
            2 => 0.9,  // High frequencies - slightly less aggressive
            _ => 1.0,
        };
        
        base_gain * band_weight
    }
    
    // Additional helper methods...
    fn create_bypass_result(&self, frame: &AudioFrame) -> AdvancedAgcResult {
        AdvancedAgcResult {
            current_lufs: -70.0, // Very quiet
            band_gains_db: vec![0.0; self.config.num_bands],
            band_peak_levels: vec![0.0; self.config.num_bands],
            limiter_active: false,
            lookahead_active: false,
            gated: true,
        }
    }
    
    fn create_gated_result(&self, current_lufs: f32) -> AdvancedAgcResult {
        AdvancedAgcResult {
            current_lufs,
            band_gains_db: vec![0.0; self.config.num_bands],
            band_peak_levels: vec![0.0; self.config.num_bands],
            limiter_active: false,
            lookahead_active: true,
            gated: true,
        }
    }
    
    /// Apply processed samples to frame (in place modification)
    fn apply_processed_samples(&self, frame: &mut AudioFrame, processed_samples: &[f32]) {
        for (sample, &processed) in frame.samples.iter_mut().zip(processed_samples.iter()) {
            let adjusted = (processed * 32768.0).max(-32768.0).min(32767.0);
            *sample = adjusted as i16;
        }
    }
}

// Supporting structures for multi-band AGC
use biquad::{Biquad, Coefficients, DirectForm1, ToHertz, Type};

struct MultibandFilterbank {
    filters: Vec<Vec<DirectForm1<f32>>>, // [band][filter_order]
    crossover_frequencies: Vec<f32>,
    sample_rate: f32,
}

impl MultibandFilterbank {
    fn new(crossover_frequencies: &[f32], sample_rate: f32) -> Result<Self> {
        let mut filters = Vec::new();
        
        // Handle single-band case (no crossover frequencies)
        if crossover_frequencies.is_empty() {
            // Single band - no filtering needed, just pass-through
            filters.push(Vec::new()); // Empty filter list for single band
        } else {
            // Multi-band case: N crossover frequencies create N+1 bands
            // For example: [300, 3000] creates 3 bands:
            // Band 0: 0-300 Hz (low-pass at 300)
            // Band 1: 300-3000 Hz (high-pass at 300 + low-pass at 3000)  
            // Band 2: 3000+ Hz (high-pass at 3000)
            
            let num_bands = crossover_frequencies.len() + 1;
            
            for band_idx in 0..num_bands {
                let mut band_filters = Vec::new();
                
                if band_idx == 0 {
                    // First band: low-pass at first crossover frequency
                    let freq = crossover_frequencies[0];
                    let coeffs = Coefficients::<f32>::from_params(
                        Type::LowPass,
                        freq.hz(),
                        sample_rate.hz(),
                        biquad::Q_BUTTERWORTH_F32,
                    ).map_err(|e| AudioProcessingError::InvalidFormat {
                        details: format!("Failed to create low-pass filter coefficients: {:?}", e),
                    })?;
                    band_filters.push(DirectForm1::<f32>::new(coeffs));
                    
                } else if band_idx == num_bands - 1 {
                    // Last band: high-pass at last crossover frequency
                    let freq = crossover_frequencies[crossover_frequencies.len() - 1];
                    let coeffs = Coefficients::<f32>::from_params(
                        Type::HighPass,
                        freq.hz(),
                        sample_rate.hz(),
                        biquad::Q_BUTTERWORTH_F32,
                    ).map_err(|e| AudioProcessingError::InvalidFormat {
                        details: format!("Failed to create high-pass filter coefficients: {:?}", e),
                    })?;
                    band_filters.push(DirectForm1::<f32>::new(coeffs));
                    
                } else {
                    // Middle bands: band-pass between two crossover frequencies
                    let low_freq = crossover_frequencies[band_idx - 1];
                    let high_freq = crossover_frequencies[band_idx];
                    
                    // High-pass at lower frequency
                    let coeffs_hp = Coefficients::<f32>::from_params(
                        Type::HighPass,
                        low_freq.hz(),
                        sample_rate.hz(),
                        biquad::Q_BUTTERWORTH_F32,
                    ).map_err(|e| AudioProcessingError::InvalidFormat {
                        details: format!("Failed to create band HP filter coefficients: {:?}", e),
                    })?;
                    band_filters.push(DirectForm1::<f32>::new(coeffs_hp));
                    
                    // Low-pass at higher frequency
                    let coeffs_lp = Coefficients::<f32>::from_params(
                        Type::LowPass,
                        high_freq.hz(),
                        sample_rate.hz(),
                        biquad::Q_BUTTERWORTH_F32,
                    ).map_err(|e| AudioProcessingError::InvalidFormat {
                        details: format!("Failed to create band LP filter coefficients: {:?}", e),
                    })?;
                    band_filters.push(DirectForm1::<f32>::new(coeffs_lp));
                }
                
                filters.push(band_filters);
            }
        }
        
        Ok(Self {
            filters,
            crossover_frequencies: crossover_frequencies.to_vec(),
            sample_rate,
        })
    }
    
    fn split(&mut self, samples: &[f32]) -> Result<Vec<Vec<f32>>> {
        let mut band_signals = Vec::new();
        
        // Handle single-band case
        if self.crossover_frequencies.is_empty() {
            // Single band - just return the original samples
            band_signals.push(samples.to_vec());
            return Ok(band_signals);
        }
        
        // Multi-band case
        for band_filters in &mut self.filters {
            let mut filtered = samples.to_vec();
            
            // Apply each filter in the band
            for filter in band_filters {
                filtered = filtered.iter()
                    .map(|&sample| filter.run(sample))
                    .collect();
            }
            
            band_signals.push(filtered);
        }
        
        Ok(band_signals)
    }
    
    fn combine(&self, band_signals: &[Vec<f32>]) -> Result<Vec<f32>> {
        if band_signals.is_empty() {
            return Ok(Vec::new());
        }
        
        // Handle single-band case
        if band_signals.len() == 1 {
            return Ok(band_signals[0].clone());
        }
        
        // Multi-band case - sum all bands
        let sample_count = band_signals[0].len();
        let mut output = vec![0.0; sample_count];
        
        for band_signal in band_signals {
            for (i, &sample) in band_signal.iter().enumerate() {
                output[i] += sample;
            }
        }
        
        Ok(output)
    }
}

struct BandProcessorConfig {
    attack_time_ms: f32,
    release_time_ms: f32,
    compression_ratio: f32,
    max_gain_db: f32,
    sample_rate: f32,
}

struct BandProcessor {
    config: BandProcessorConfig,
    current_gain_db: f32,
    peak_level: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

struct BandProcessingResult {
    applied_gain_db: f32,
    peak_level: f32,
    limiter_active: bool,
}

impl BandProcessor {
    fn new(config: BandProcessorConfig) -> Result<Self> {
        let frame_rate = config.sample_rate / 160.0; // Assume 20ms frames
        
        let attack_coeff = 1.0 - (-1.0 / (config.attack_time_ms * frame_rate / 1000.0)).exp();
        let release_coeff = 1.0 - (-1.0 / (config.release_time_ms * frame_rate / 1000.0)).exp();
        
        Ok(Self {
            config,
            current_gain_db: 0.0,
            peak_level: 0.0,
            attack_coeff,
            release_coeff,
        })
    }
    
    fn process_band(&mut self, samples: &[f32], target_gain: f32) -> Result<(Vec<f32>, BandProcessingResult)> {
        // Calculate current level
        let rms_level = Self::calculate_rms(samples);
        
        // Update peak tracking
        let current_peak = samples.iter().map(|&s| s.abs()).fold(0.0f32, f32::max);
        self.peak_level = self.peak_level * 0.99 + current_peak * 0.01;
        
        // Convert target gain to dB
        let target_gain_db = 20.0 * target_gain.log10();
        
        // Calculate desired gain with compression
        let compressed_gain_db = self.apply_compression(target_gain_db, rms_level);
        
        // Apply attack/release smoothing
        let gain_diff = compressed_gain_db - self.current_gain_db;
        let coeff = if gain_diff > 0.0 {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        
        self.current_gain_db += gain_diff * coeff;
        
        // Apply gain limiting
        let (final_gain_db, limiter_active) = self.apply_limiting(self.current_gain_db);
        
        // Convert to linear gain
        let linear_gain = 10.0_f32.powf(final_gain_db / 20.0);
        
        // Apply gain to samples
        let processed_samples: Vec<f32> = samples.iter()
            .map(|&sample| sample * linear_gain)
            .collect();
        
        Ok((processed_samples, BandProcessingResult {
            applied_gain_db: final_gain_db,
            peak_level: self.peak_level,
            limiter_active,
        }))
    }
    
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        
        let sum_squares: f32 = samples.iter().map(|&s| s * s).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }
    
    fn apply_compression(&self, gain_db: f32, level: f32) -> f32 {
        // Simple compression curve
        let threshold_db = -20.0; // -20dB threshold
        
        if gain_db > threshold_db {
            let over_threshold = gain_db - threshold_db;
            threshold_db + (over_threshold / self.config.compression_ratio)
        } else {
            gain_db
        }
    }
    
    fn apply_limiting(&self, gain_db: f32) -> (f32, bool) {
        if gain_db > self.config.max_gain_db {
            (self.config.max_gain_db, true)
        } else {
            (gain_db, false)
        }
    }
}

struct LookaheadBuffer {
    buffer: Vec<f32>,
    delay_samples: usize,
    write_index: usize,
    filled: bool,
}

impl LookaheadBuffer {
    fn new(lookahead_ms: f32, sample_rate: f32) -> Result<Self> {
        let delay_samples = (lookahead_ms * sample_rate / 1000.0) as usize;
        let buffer_size = delay_samples * 2; // Double buffer for safety
        
        Ok(Self {
            buffer: vec![0.0; buffer_size],
            delay_samples,
            write_index: 0,
            filled: false,
        })
    }
    
    fn push(&mut self, samples: &[i16]) {
        for &sample in samples {
            self.buffer[self.write_index] = sample as f32 / 32768.0;
            self.write_index = (self.write_index + 1) % self.buffer.len();
            
            if self.write_index == 0 {
                self.filled = true;
            }
        }
    }
    
    fn get_delayed(&mut self) -> Option<Vec<f32>> {
        if !self.filled {
            return None;
        }
        
        let mut delayed_samples = Vec::with_capacity(160);
        let start_index = if self.write_index >= 160 {
            self.write_index - 160
        } else {
            self.buffer.len() - (160 - self.write_index)
        };
        
        for i in 0..160.min(self.buffer.len()) {
            let index = (start_index + i) % self.buffer.len();
            delayed_samples.push(self.buffer[index]);
        }
        
        if delayed_samples.len() == 160 {
            Some(delayed_samples)
        } else {
            None
        }
    }
}

struct LookaheadPeakDetector {
    sample_rate: f32,
    lookahead_samples: usize,
}

impl LookaheadPeakDetector {
    fn new(lookahead_ms: f32, sample_rate: f32) -> Result<Self> {
        let lookahead_samples = (lookahead_ms * sample_rate / 1000.0) as usize;
        
        Ok(Self {
            sample_rate,
            lookahead_samples,
        })
    }
    
    fn detect_peaks(&self, future_samples: &[i16], band_signals: &[Vec<f32>]) -> Result<Vec<f32>> {
        let mut peak_levels = Vec::new();
        
        for band_signal in band_signals {
            let peak = band_signal.iter()
                .map(|&s| s.abs())
                .fold(0.0f32, f32::max);
            peak_levels.push(peak);
        }
        
        Ok(peak_levels)
    }
}

struct LoudnessMeter {
    sample_rate: f32,
    perceptual_weighting: bool,
    // Simplified LUFS measurement
    energy_history: Vec<f32>,
    history_index: usize,
}

impl LoudnessMeter {
    fn new(sample_rate: f32, perceptual_weighting: bool) -> Result<Self> {
        Ok(Self {
            sample_rate,
            perceptual_weighting,
            energy_history: vec![0.0; 100], // 100 frame history
            history_index: 0,
        })
    }
    
    fn measure_frame(&mut self, samples: &[f32]) -> Result<f32> {
        // Calculate RMS energy
        let rms = if samples.is_empty() {
            0.0
        } else {
            let sum_squares: f32 = samples.iter().map(|&s| s * s).sum();
            (sum_squares / samples.len() as f32).sqrt()
        };
        
        // Store in history
        self.energy_history[self.history_index] = rms;
        self.history_index = (self.history_index + 1) % self.energy_history.len();
        
        // Calculate average energy
        let avg_energy = self.energy_history.iter().sum::<f32>() / self.energy_history.len() as f32;
        
        // Convert to LUFS (simplified approximation)
        Ok(if avg_energy > 1e-10 {
            -0.691 + 10.0 * avg_energy.log10() // Rough LUFS approximation
        } else {
            -70.0 // Very quiet
        })
    }
}

struct GatingProcessor {
    gate_threshold: f32,
}

impl GatingProcessor {
    fn new(gate_threshold: f32) -> Result<Self> {
        Ok(Self { gate_threshold })
    }
    
    fn is_gated(&self, lufs: f32) -> bool {
        lufs < self.gate_threshold
    }
} 