use std::collections::VecDeque;
use std::time::Duration;

use tracing::trace;

use crate::error::Result;
use crate::codec::audio::common::{AudioFormat, SampleFormat};

/// Voice activity detection parameters
#[derive(Debug, Clone)]
pub struct VadConfig {
    /// Energy threshold for speech detection (0.0-1.0)
    pub energy_threshold: f32,
    /// Speech hang time in milliseconds
    pub speech_hang_time_ms: u32,
    /// Non-speech hang time in milliseconds
    pub non_speech_hang_time_ms: u32,
    /// Window size for energy calculation in milliseconds
    pub window_size_ms: u32,
    /// Audio format
    pub format: AudioFormat,
}

impl Default for VadConfig {
    fn default() -> Self {
        Self {
            energy_threshold: 0.05,
            speech_hang_time_ms: 300,
            non_speech_hang_time_ms: 100,
            window_size_ms: 20,
            format: AudioFormat::pcm_telephony(),
        }
    }
}

/// Voice activity detection (VAD) state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadState {
    /// Speech detected
    Speech,
    /// No speech detected
    NonSpeech,
}

impl Default for VadState {
    fn default() -> Self {
        Self::NonSpeech
    }
}

/// Voice activity detection statistics
#[derive(Debug, Clone, Default)]
pub struct VadStats {
    /// Current VAD state
    pub state: VadState,
    /// Current energy level (0.0-1.0)
    pub energy: f32,
    /// Time spent in speech state
    pub speech_time: Duration,
    /// Time spent in non-speech state
    pub non_speech_time: Duration,
    /// Number of speech segments detected
    pub speech_segments: u32,
    /// Average energy during speech
    pub avg_speech_energy: f32,
    /// Maximum energy seen during speech
    pub max_speech_energy: f32,
}

/// Voice activity detector
pub struct VoiceActivityDetector {
    /// Configuration
    config: VadConfig,
    /// Current state
    state: VadState,
    /// Time in current state
    time_in_state_ms: u32,
    /// Energy history for smoothing
    energy_history: VecDeque<f32>,
    /// Statistics
    stats: VadStats,
    /// Samples processed
    samples_processed: u64,
    /// Sample rate
    sample_rate: u32,
    /// Current frame size
    frame_size: usize,
    /// Timestamp for statistics
    speech_start_time: Option<Duration>,
}

impl VoiceActivityDetector {
    /// Create a new voice activity detector
    pub fn new(config: VadConfig) -> Self {
        let sample_rate = config.format.sample_rate.as_hz();
        let channels = config.format.channels.channel_count() as usize;
        let window_samples = (sample_rate as u64 * config.window_size_ms as u64 / 1000) as usize;
        let frame_size = window_samples * channels;
        
        Self {
            config,
            state: VadState::NonSpeech,
            time_in_state_ms: 0,
            energy_history: VecDeque::with_capacity(5),
            stats: VadStats::default(),
            samples_processed: 0,
            sample_rate,
            frame_size,
            speech_start_time: None,
        }
    }
    
    /// Create a new voice activity detector with default config
    pub fn new_default() -> Self {
        Self::new(VadConfig::default())
    }
    
    /// Process audio frame and detect voice activity
    pub fn process<T: AsRef<[u8]>>(&mut self, frame: T) -> Result<VadState> {
        let frame = frame.as_ref();
        
        // Calculate energy of the frame
        let energy = self.calculate_energy(frame);
        
        // Update energy history
        self.energy_history.push_back(energy);
        if self.energy_history.len() > 5 {
            self.energy_history.pop_front();
        }
        
        // Get smoothed energy
        let smoothed_energy = self.energy_history.iter().sum::<f32>() / self.energy_history.len() as f32;
        
        // Determine if this is speech
        let is_speech = smoothed_energy > self.config.energy_threshold;
        
        // Update state machine
        let frame_time_ms = (frame.len() as u64 * 1000 / self.sample_rate as u64 / 
            self.config.format.bytes_per_sample() as u64 / 
            self.config.format.channels.channel_count() as u64) as u32;
        
        self.time_in_state_ms += frame_time_ms;
        
        // Determine state transitions
        match (self.state, is_speech) {
            (VadState::NonSpeech, true) => {
                // Transition to speech if we've been above threshold long enough
                if self.time_in_state_ms >= self.config.non_speech_hang_time_ms {
                    self.state = VadState::Speech;
                    self.time_in_state_ms = 0;
                    self.stats.speech_segments += 1;
                    self.speech_start_time = Some(Duration::from_millis(self.samples_processed as u64 * 1000 / self.sample_rate as u64));
                    trace!("VAD: Speech detected, energy={:.3}", smoothed_energy);
                }
            },
            (VadState::Speech, false) => {
                // Transition to non-speech if we've been below threshold long enough
                if self.time_in_state_ms >= self.config.speech_hang_time_ms {
                    // Update speech time statistics
                    if let Some(start_time) = self.speech_start_time {
                        let now = Duration::from_millis(self.samples_processed as u64 * 1000 / self.sample_rate as u64);
                        let duration = now - start_time;
                        self.stats.speech_time += duration;
                    }
                    
                    self.state = VadState::NonSpeech;
                    self.time_in_state_ms = 0;
                    self.speech_start_time = None;
                    trace!("VAD: Silence detected, energy={:.3}", smoothed_energy);
                }
            },
            _ => {
                // Stay in the same state, just update time
            }
        }
        
        // Update statistics
        self.stats.energy = smoothed_energy;
        self.stats.state = self.state;
        
        if self.state == VadState::Speech {
            // Update speech energy statistics
            self.stats.avg_speech_energy = (self.stats.avg_speech_energy * 0.95) + (smoothed_energy * 0.05);
            if smoothed_energy > self.stats.max_speech_energy {
                self.stats.max_speech_energy = smoothed_energy;
            }
        } else {
            // Update non-speech time
            self.stats.non_speech_time += Duration::from_millis(frame_time_ms as u64);
        }
        
        // Update samples processed
        self.samples_processed += (frame.len() / self.config.format.bytes_per_sample() / 
            self.config.format.channels.channel_count() as usize) as u64;
        
        Ok(self.state)
    }
    
    /// Calculate energy of a frame (RMS)
    fn calculate_energy(&self, frame: &[u8]) -> f32 {
        let format = &self.config.format;
        let bytes_per_sample = format.bytes_per_sample();
        let channels = format.channels.channel_count() as usize;
        
        if frame.is_empty() {
            return 0.0;
        }
        
        let num_samples = frame.len() / bytes_per_sample / channels;
        
        match format.format {
            SampleFormat::S16 => {
                let samples = unsafe {
                    std::slice::from_raw_parts(
                        frame.as_ptr() as *const i16,
                        frame.len() / 2
                    )
                };
                
                // Sum squares of samples
                let mut sum_squares = 0.0;
                for ch in 0..channels {
                    for i in 0..num_samples {
                        let sample = samples[i * channels + ch] as f32 / 32768.0;
                        sum_squares += sample * sample;
                    }
                }
                
                // Calculate RMS
                let mean_square = sum_squares / (num_samples * channels) as f32;
                mean_square.sqrt()
            },
            SampleFormat::U8 => {
                // Sum squares of samples
                let mut sum_squares = 0.0;
                for ch in 0..channels {
                    for i in 0..num_samples {
                        let sample = ((frame[i * channels + ch] as f32) - 128.0) / 128.0;
                        sum_squares += sample * sample;
                    }
                }
                
                // Calculate RMS
                let mean_square = sum_squares / (num_samples * channels) as f32;
                mean_square.sqrt()
            },
            SampleFormat::F32 => {
                let samples = unsafe {
                    std::slice::from_raw_parts(
                        frame.as_ptr() as *const f32,
                        frame.len() / 4
                    )
                };
                
                // Sum squares of samples
                let mut sum_squares = 0.0;
                for ch in 0..channels {
                    for i in 0..num_samples {
                        let sample = samples[i * channels + ch];
                        sum_squares += sample * sample;
                    }
                }
                
                // Calculate RMS
                let mean_square = sum_squares / (num_samples * channels) as f32;
                mean_square.sqrt()
            },
            // For other formats, we'll need to convert to a common format first
            _ => 0.0,
        }
    }
    
    /// Get current VAD state
    pub fn state(&self) -> VadState {
        self.state
    }
    
    /// Get VAD statistics
    pub fn stats(&self) -> &VadStats {
        &self.stats
    }
    
    /// Reset VAD state
    pub fn reset(&mut self) {
        self.state = VadState::NonSpeech;
        self.time_in_state_ms = 0;
        self.energy_history.clear();
        self.stats = VadStats::default();
        self.samples_processed = 0;
        self.speech_start_time = None;
    }
    
    /// Set energy threshold
    pub fn set_energy_threshold(&mut self, threshold: f32) {
        self.config.energy_threshold = threshold.clamp(0.0, 1.0);
    }
    
    /// Get recommended frame size for this VAD
    pub fn recommended_frame_size(&self) -> usize {
        self.frame_size
    }
} 