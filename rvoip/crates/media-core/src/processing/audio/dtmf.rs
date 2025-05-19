//! DTMF (Dual-Tone Multi-Frequency) generation and detection
//!
//! This module provides utilities for generating DTMF tones and detecting
//! DTMF signals in audio streams.

use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};
use std::collections::VecDeque;

use crate::error::{Error, Result};
use crate::{Sample, SampleRate, AudioBuffer, AudioFormat};
use super::common::AudioProcessor;

/// DTMF digit representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtmfDigit {
    /// Digit 0
    D0,
    /// Digit 1
    D1,
    /// Digit 2
    D2,
    /// Digit 3
    D3,
    /// Digit 4
    D4,
    /// Digit 5
    D5,
    /// Digit 6
    D6,
    /// Digit 7
    D7,
    /// Digit 8
    D8,
    /// Digit 9
    D9,
    /// Star (*)
    Star,
    /// Pound (#)
    Pound,
    /// A (Extended DTMF)
    A,
    /// B (Extended DTMF)
    B,
    /// C (Extended DTMF)
    C,
    /// D (Extended DTMF)
    D,
}

impl DtmfDigit {
    /// Convert a char to a DTMF digit
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '0' => Some(Self::D0),
            '1' => Some(Self::D1),
            '2' => Some(Self::D2),
            '3' => Some(Self::D3),
            '4' => Some(Self::D4),
            '5' => Some(Self::D5),
            '6' => Some(Self::D6),
            '7' => Some(Self::D7),
            '8' => Some(Self::D8),
            '9' => Some(Self::D9),
            '*' => Some(Self::Star),
            '#' => Some(Self::Pound),
            'A' | 'a' => Some(Self::A),
            'B' | 'b' => Some(Self::B),
            'C' | 'c' => Some(Self::C),
            'D' | 'd' => Some(Self::D),
            _ => None,
        }
    }
    
    /// Convert a DTMF digit to a char
    pub fn to_char(&self) -> char {
        match self {
            Self::D0 => '0',
            Self::D1 => '1',
            Self::D2 => '2',
            Self::D3 => '3',
            Self::D4 => '4',
            Self::D5 => '5',
            Self::D6 => '6',
            Self::D7 => '7',
            Self::D8 => '8',
            Self::D9 => '9',
            Self::Star => '*',
            Self::Pound => '#',
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::D => 'D',
        }
    }
    
    /// Get the frequencies (low, high) for this digit
    pub fn frequencies(&self) -> (f32, f32) {
        match self {
            Self::D1 => (697.0, 1209.0),
            Self::D2 => (697.0, 1336.0),
            Self::D3 => (697.0, 1477.0),
            Self::A => (697.0, 1633.0),
            Self::D4 => (770.0, 1209.0),
            Self::D5 => (770.0, 1336.0),
            Self::D6 => (770.0, 1477.0),
            Self::B => (770.0, 1633.0),
            Self::D7 => (852.0, 1209.0),
            Self::D8 => (852.0, 1336.0),
            Self::D9 => (852.0, 1477.0),
            Self::C => (852.0, 1633.0),
            Self::Star => (941.0, 1209.0),
            Self::D0 => (941.0, 1336.0),
            Self::Pound => (941.0, 1477.0),
            Self::D => (941.0, 1633.0),
        }
    }
}

/// DTMF event
#[derive(Debug, Clone)]
pub struct DtmfEvent {
    /// The DTMF digit
    pub digit: DtmfDigit,
    /// Whether the event is the start (true) or end (false) of a tone
    pub is_start: bool,
    /// Timestamp in milliseconds
    pub timestamp_ms: u64,
    /// Duration in milliseconds (for end events)
    pub duration_ms: Option<u32>,
}

/// Configuration for DTMF detection
#[derive(Debug, Clone)]
pub struct DtmfDetectorConfig {
    /// Minimum duration for a valid tone (ms)
    pub min_duration_ms: u32,
    /// Maximum duration to report (ms)
    pub max_duration_ms: u32,
    /// Energy threshold for detection
    pub energy_threshold: f32,
    /// Required signal-to-noise ratio (dB)
    pub snr_threshold_db: f32,
    /// Frequency deviation tolerance (Hz)
    pub frequency_tolerance_hz: f32,
}

impl Default for DtmfDetectorConfig {
    fn default() -> Self {
        Self {
            min_duration_ms: 40,
            max_duration_ms: 5000,
            energy_threshold: 0.01,
            snr_threshold_db: 15.0,
            frequency_tolerance_hz: 20.0,
        }
    }
}

/// DTMF detector implementation
#[derive(Debug)]
pub struct DtmfDetector {
    /// Configuration
    config: DtmfDetectorConfig,
    
    /// Sample rate
    sample_rate: SampleRate,
    
    /// Frame size in samples
    frame_size: usize,
    
    /// Current active digit
    current_digit: Option<DtmfDigit>,
    
    /// Current duration (frames)
    current_duration: u32,
    
    /// Start timestamp of current digit
    current_start_ms: u64,
    
    /// Current timestamp
    current_time_ms: u64,
    
    /// Event queue
    events: VecDeque<DtmfEvent>,
}

impl DtmfDetector {
    /// Create a new DTMF detector with the given configuration
    pub fn new(config: DtmfDetectorConfig, sample_rate: SampleRate, frame_size: usize) -> Self {
        Self {
            config,
            sample_rate,
            frame_size,
            current_digit: None,
            current_duration: 0,
            current_start_ms: 0,
            current_time_ms: 0,
            events: VecDeque::new(),
        }
    }
    
    /// Create a new DTMF detector with default configuration
    pub fn default_for_rate(sample_rate: SampleRate, frame_size: usize) -> Self {
        Self::new(DtmfDetectorConfig::default(), sample_rate, frame_size)
    }
    
    /// Process a frame of audio, returning true if a DTMF digit was detected
    pub fn process_frame(&mut self, frame: &[Sample], timestamp_ms: u64) -> Result<bool> {
        if frame.len() != self.frame_size {
            return Err(Error::InvalidArgument(format!(
                "Frame size ({}) doesn't match expected size ({})",
                frame.len(), self.frame_size
            )));
        }
        
        self.current_time_ms = timestamp_ms;
        
        // Stub implementation - in a real implementation, we would:
        // 1. Run FFT or Goertzel algorithm to detect DTMF frequencies
        // 2. Check if the detected frequencies match any DTMF digit
        // 3. Apply timing and energy thresholds
        // 4. Generate start/stop events
        
        // For this stub, we'll just always return false
        Ok(false)
    }
    
    /// Get any pending DTMF events
    pub fn get_events(&mut self) -> Vec<DtmfEvent> {
        let mut events = Vec::new();
        while let Some(event) = self.events.pop_front() {
            events.push(event);
        }
        events
    }
    
    /// Check if a DTMF digit is currently active
    pub fn is_active(&self) -> bool {
        self.current_digit.is_some()
    }
    
    /// Get the current active digit, if any
    pub fn active_digit(&self) -> Option<DtmfDigit> {
        self.current_digit
    }
    
    /// Reset the detector state
    pub fn reset(&mut self) {
        self.current_digit = None;
        self.current_duration = 0;
        self.current_start_ms = 0;
        self.events.clear();
    }
}

impl AudioProcessor for DtmfDetector {
    fn process(&mut self, input: &AudioBuffer) -> Result<AudioBuffer> {
        // We don't modify the audio in the detector, just analyze it
        self.process_frame(
            &input.data.as_ref(), 
            self.current_time_ms
        )?;
        
        // Return the input unchanged
        Ok(input.clone())
    }
    
    fn process_samples(&mut self, input: &[Sample]) -> Result<Vec<Sample>> {
        // We don't modify the audio in the detector, just analyze it
        self.process_frame(input, self.current_time_ms)?;
        
        // Return the input unchanged
        Ok(input.to_vec())
    }
    
    fn reset(&mut self) -> Result<()> {
        self.reset();
        Ok(())
    }
    
    fn name(&self) -> &str {
        "DtmfDetector"
    }
}

/// DTMF generator configuration
#[derive(Debug, Clone)]
pub struct DtmfGeneratorConfig {
    /// Tone amplitude (0.0-1.0)
    pub amplitude: f32,
    
    /// Default tone duration in milliseconds
    pub default_duration_ms: u32,
    
    /// Default gap between tones in milliseconds
    pub default_gap_ms: u32,
    
    /// Fade in/out duration in milliseconds
    pub fade_ms: u32,
}

impl Default for DtmfGeneratorConfig {
    fn default() -> Self {
        Self {
            amplitude: 0.3,        // 30% amplitude to avoid clipping
            default_duration_ms: 100,
            default_gap_ms: 50,
            fade_ms: 5,
        }
    }
}

/// DTMF tone generator
#[derive(Debug)]
pub struct DtmfGenerator {
    /// Configuration
    config: DtmfGeneratorConfig,
    
    /// Sample rate
    sample_rate: SampleRate,
    
    /// Current phase low frequency
    phase_low: f32,
    
    /// Current phase high frequency
    phase_high: f32,
    
    /// Current digit being generated
    current_digit: Option<DtmfDigit>,
    
    /// Samples remaining in current tone
    samples_remaining: usize,
    
    /// Whether we're in the fade-in part
    in_fade_in: bool,
    
    /// Whether we're in the fade-out part
    in_fade_out: bool,
    
    /// Current sample position in fade
    fade_position: usize,
    
    /// Total fade samples
    fade_samples: usize,
}

impl DtmfGenerator {
    /// Create a new DTMF generator with the given configuration
    pub fn new(config: DtmfGeneratorConfig, sample_rate: SampleRate) -> Self {
        let fade_samples = (config.fade_ms as f32 * sample_rate.as_hz() as f32 / 1000.0) as usize;
        Self {
            config,
            sample_rate,
            phase_low: 0.0,
            phase_high: 0.0,
            current_digit: None,
            samples_remaining: 0,
            in_fade_in: false,
            in_fade_out: false,
            fade_position: 0,
            fade_samples,
        }
    }
    
    /// Create a new DTMF generator with default configuration
    pub fn default_for_rate(sample_rate: SampleRate) -> Self {
        Self::new(DtmfGeneratorConfig::default(), sample_rate)
    }
    
    /// Start generating a DTMF tone
    pub fn start_tone(&mut self, digit: DtmfDigit, duration_ms: Option<u32>) -> Result<()> {
        // Reset state
        self.phase_low = 0.0;
        self.phase_high = 0.0;
        
        // Set up new tone
        self.current_digit = Some(digit);
        let duration = duration_ms.unwrap_or(self.config.default_duration_ms);
        self.samples_remaining = (duration as f32 * self.sample_rate.as_hz() as f32 / 1000.0) as usize;
        
        // Start fade in
        self.in_fade_in = true;
        self.in_fade_out = false;
        self.fade_position = 0;
        
        Ok(())
    }
    
    /// Stop the current tone
    pub fn stop_tone(&mut self) -> Result<()> {
        if self.current_digit.is_none() {
            return Err(Error::InvalidState("No tone is currently active".to_string()));
        }
        
        // Start fade out
        self.in_fade_out = true;
        self.fade_position = 0;
        
        // Only keep enough samples for fade out
        if self.samples_remaining > self.fade_samples {
            self.samples_remaining = self.fade_samples;
        }
        
        Ok(())
    }
    
    /// Generate a batch of samples
    pub fn generate(&mut self, out_buffer: &mut [Sample]) -> Result<usize> {
        if self.current_digit.is_none() {
            // Fill with zeros if no active tone
            for sample in out_buffer.iter_mut() {
                *sample = 0;
            }
            return Ok(out_buffer.len());
        }
        
        // Stub implementation - in a real implementation, we would:
        // 1. Generate sine waves at the correct frequencies
        // 2. Apply amplitude and fade in/out
        // 3. Track remaining samples and phase
        
        // For this stub, we'll just generate silence
        for sample in out_buffer.iter_mut() {
            *sample = 0;
        }
        
        // Update state
        let generated = out_buffer.len().min(self.samples_remaining);
        self.samples_remaining -= generated;
        
        // Check if we've finished the tone
        if self.samples_remaining == 0 {
            self.current_digit = None;
        }
        
        Ok(generated)
    }
    
    /// Generate a complete DTMF tone and return the samples
    pub fn generate_tone(&mut self, digit: DtmfDigit, duration_ms: u32) -> Result<Vec<Sample>> {
        let num_samples = (duration_ms as f32 * self.sample_rate.as_hz() as f32 / 1000.0) as usize;
        let mut buffer = vec![0; num_samples];
        
        self.start_tone(digit, Some(duration_ms))?;
        self.generate(&mut buffer)?;
        
        Ok(buffer)
    }
    
    /// Check if a tone is currently being generated
    pub fn is_active(&self) -> bool {
        self.current_digit.is_some()
    }
    
    /// Get the current digit being generated, if any
    pub fn current_digit(&self) -> Option<DtmfDigit> {
        self.current_digit
    }
    
    /// Reset the generator state
    pub fn reset(&mut self) {
        self.current_digit = None;
        self.samples_remaining = 0;
        self.phase_low = 0.0;
        self.phase_high = 0.0;
        self.in_fade_in = false;
        self.in_fade_out = false;
    }
}

/// Builder for creating DTMF detector instances
pub struct DtmfDetectorBuilder {
    config: DtmfDetectorConfig,
    sample_rate: SampleRate,
    frame_size: usize,
}

impl DtmfDetectorBuilder {
    /// Create a new DTMF detector builder
    pub fn new() -> Self {
        Self {
            config: DtmfDetectorConfig::default(),
            sample_rate: SampleRate::Rate8000,
            frame_size: 160, // 20ms at 8kHz
        }
    }
    
    /// Set the minimum duration for a valid tone
    pub fn with_min_duration_ms(mut self, duration: u32) -> Self {
        self.config.min_duration_ms = duration;
        self
    }
    
    /// Set the energy threshold for detection
    pub fn with_energy_threshold(mut self, threshold: f32) -> Self {
        self.config.energy_threshold = threshold;
        self
    }
    
    /// Set the required signal-to-noise ratio
    pub fn with_snr_threshold_db(mut self, snr_db: f32) -> Self {
        self.config.snr_threshold_db = snr_db;
        self
    }
    
    /// Set the frequency tolerance
    pub fn with_frequency_tolerance_hz(mut self, tolerance: f32) -> Self {
        self.config.frequency_tolerance_hz = tolerance;
        self
    }
    
    /// Set the sample rate
    pub fn with_sample_rate(mut self, rate: SampleRate) -> Self {
        self.sample_rate = rate;
        self
    }
    
    /// Set the frame size in samples
    pub fn with_frame_size(mut self, size: usize) -> Self {
        self.frame_size = size;
        self
    }
    
    /// Build the DTMF detector
    pub fn build(self) -> DtmfDetector {
        DtmfDetector::new(self.config, self.sample_rate, self.frame_size)
    }
}

/// Builder for creating DTMF generator instances
pub struct DtmfGeneratorBuilder {
    config: DtmfGeneratorConfig,
    sample_rate: SampleRate,
}

impl DtmfGeneratorBuilder {
    /// Create a new DTMF generator builder
    pub fn new() -> Self {
        Self {
            config: DtmfGeneratorConfig::default(),
            sample_rate: SampleRate::Rate8000,
        }
    }
    
    /// Set the tone amplitude
    pub fn with_amplitude(mut self, amplitude: f32) -> Self {
        self.config.amplitude = amplitude.max(0.0).min(1.0);
        self
    }
    
    /// Set the default tone duration
    pub fn with_default_duration_ms(mut self, duration: u32) -> Self {
        self.config.default_duration_ms = duration;
        self
    }
    
    /// Set the default gap between tones
    pub fn with_default_gap_ms(mut self, gap: u32) -> Self {
        self.config.default_gap_ms = gap;
        self
    }
    
    /// Set the fade in/out duration
    pub fn with_fade_ms(mut self, fade: u32) -> Self {
        self.config.fade_ms = fade;
        self
    }
    
    /// Set the sample rate
    pub fn with_sample_rate(mut self, rate: SampleRate) -> Self {
        self.sample_rate = rate;
        self
    }
    
    /// Build the DTMF generator
    pub fn build(self) -> DtmfGenerator {
        DtmfGenerator::new(self.config, self.sample_rate)
    }
} 