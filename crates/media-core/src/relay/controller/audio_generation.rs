//! Audio generation and transmission functionality
//!
//! This module provides audio generation capabilities for testing and
//! audio transmission management for RTP sessions with support for multiple audio sources.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Mutex};
use tokio::time::interval;
use tracing::{debug, error, info, warn};
use bytes::Bytes;

use rvoip_rtp_core::RtpSession;

/// Audio source types supported by the audio transmitter
#[derive(Debug, Clone)]
pub enum AudioSource {
    /// Generate a sine wave tone
    Tone { frequency: f64, amplitude: f64 },
    /// Use custom audio samples (repeating)
    CustomSamples { samples: Vec<u8>, repeat: bool },
    /// Pass-through mode (no audio generation)
    PassThrough,
}

/// Audio generator for creating test tones and audio streams
pub struct AudioGenerator {
    /// Sample rate (Hz)
    sample_rate: u32,
    /// Current phase for sine wave generation
    phase: f64,
    /// Audio source configuration
    source: AudioSource,
    /// Current position in custom samples (if using custom samples)
    sample_position: usize,
}

impl AudioGenerator {
    /// Create a new audio generator with tone generation
    pub fn new(sample_rate: u32, frequency: f64, amplitude: f64) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            source: AudioSource::Tone { frequency, amplitude },
            sample_position: 0,
        }
    }
    
    /// Create a new audio generator with custom audio source
    pub fn new_with_source(sample_rate: u32, source: AudioSource) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            source,
            sample_position: 0,
        }
    }
    
    /// Generate audio samples for PCMU (G.711 μ-law) encoding
    pub fn generate_pcmu_samples(&mut self, num_samples: usize) -> Vec<u8> {
        match self.source.clone() {
            AudioSource::Tone { frequency, amplitude } => {
                self.generate_tone_samples(num_samples, frequency, amplitude)
            }
            AudioSource::CustomSamples { samples, repeat } => {
                self.generate_custom_samples(num_samples, &samples, repeat)
            }
            AudioSource::PassThrough => {
                // Return silence for pass-through mode
                vec![0x7F; num_samples] // μ-law silence
            }
        }
    }
    
    /// Generate tone samples
    fn generate_tone_samples(&mut self, num_samples: usize, frequency: f64, amplitude: f64) -> Vec<u8> {
        let mut samples = Vec::with_capacity(num_samples);
        let phase_increment = 2.0 * std::f64::consts::PI * frequency / self.sample_rate as f64;
        
        for _ in 0..num_samples {
            // Generate sine wave sample
            let sample = (self.phase.sin() * amplitude * 32767.0) as i16;
            
            // Convert to μ-law
            let pcmu_sample = Self::linear_to_ulaw(sample);
            samples.push(pcmu_sample);
            
            // Update phase
            self.phase += phase_increment;
            if self.phase >= 2.0 * std::f64::consts::PI {
                self.phase -= 2.0 * std::f64::consts::PI;
            }
        }
        
        samples
    }
    
    /// Generate samples from custom audio data
    fn generate_custom_samples(&mut self, num_samples: usize, samples: &[u8], repeat: bool) -> Vec<u8> {
        let mut result = Vec::with_capacity(num_samples);
        
        if samples.is_empty() {
            // Return silence if no custom samples
            return vec![0x7F; num_samples]; // μ-law silence
        }
        
        for _ in 0..num_samples {
            if self.sample_position >= samples.len() {
                if repeat {
                    self.sample_position = 0;
                } else {
                    // End of samples, return silence
                    result.push(0x7F); // μ-law silence
                    continue;
                }
            }
            
            result.push(samples[self.sample_position]);
            self.sample_position += 1;
        }
        
        result
    }
    
    /// Convert linear PCM to μ-law (G.711)
    pub fn linear_to_ulaw(pcm: i16) -> u8 {
        // Simplified μ-law encoding
        let sign = if pcm < 0 { 0x80u8 } else { 0x00u8 };
        let magnitude = pcm.abs() as u16;
        
        // Find the segment
        let mut segment = 0u8;
        let mut temp = magnitude >> 5;
        while temp != 0 && segment < 7 {
            segment += 1;
            temp >>= 1;
        }
        
        // Calculate quantization value
        let quantization = if segment == 0 {
            (magnitude >> 1) as u8
        } else {
            (((magnitude >> (segment + 1)) & 0x0F) + 0x10) as u8
        };
        
        // Combine sign, segment, and quantization
        sign | (segment << 4) | (quantization & 0x0F)
    }
    
    /// Convert PCM samples to μ-law
    pub fn pcm_to_mulaw(pcm_samples: &[i16]) -> Vec<u8> {
        pcm_samples.iter().map(|&sample| Self::linear_to_ulaw(sample)).collect()
    }
    
    /// Update the audio source
    pub fn set_source(&mut self, source: AudioSource) {
        self.source = source;
        self.sample_position = 0; // Reset position for custom samples
    }
}

/// Audio transmission configuration
#[derive(Debug, Clone)]
pub struct AudioTransmitterConfig {
    /// Audio source type
    pub source: AudioSource,
    /// Transmission interval (default: 20ms)
    pub interval: Duration,
    /// Samples per packet (default: 160 for 20ms at 8kHz)
    pub samples_per_packet: usize,
    /// Sample rate (default: 8000 Hz)
    pub sample_rate: u32,
}

impl Default for AudioTransmitterConfig {
    fn default() -> Self {
        Self {
            source: AudioSource::PassThrough, // Default to pass-through mode
            interval: Duration::from_millis(20),
            samples_per_packet: 160,
            sample_rate: 8000,
        }
    }
}

/// Audio transmission task for RTP sessions
pub struct AudioTransmitter {
    /// RTP session for transmission
    rtp_session: Arc<Mutex<RtpSession>>,
    /// Audio generator
    audio_generator: Arc<Mutex<AudioGenerator>>,
    /// Transmission configuration
    config: AudioTransmitterConfig,
    /// Current RTP timestamp
    timestamp: Arc<Mutex<u32>>,
    /// Whether transmission is active
    is_active: Arc<RwLock<bool>>,
}

impl AudioTransmitter {
    /// Create a new audio transmitter with default configuration (pass-through mode)
    pub fn new(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        let config = AudioTransmitterConfig::default();
        Self::new_with_config(rtp_session, config)
    }
    
    /// Create a new audio transmitter with tone generation (for backward compatibility)
    pub fn new_with_tone(rtp_session: Arc<Mutex<RtpSession>>) -> Self {
        let config = AudioTransmitterConfig {
            source: AudioSource::Tone { frequency: 440.0, amplitude: 0.5 },
            ..Default::default()
        };
        Self::new_with_config(rtp_session, config)
    }
    
    /// Create a new audio transmitter with custom configuration
    pub fn new_with_config(rtp_session: Arc<Mutex<RtpSession>>, config: AudioTransmitterConfig) -> Self {
        let audio_generator = AudioGenerator::new_with_source(config.sample_rate, config.source.clone());
        
        Self {
            rtp_session,
            audio_generator: Arc::new(Mutex::new(audio_generator)),
            config,
            timestamp: Arc::new(Mutex::new(0)),
            is_active: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Start audio transmission
    pub async fn start(&mut self) {
        *self.is_active.write().await = true;
        
        let source_desc = match &self.config.source {
            AudioSource::Tone { frequency, amplitude } => {
                format!("{}Hz tone (amplitude: {})", frequency, amplitude)
            }
            AudioSource::CustomSamples { samples, repeat } => {
                format!("custom audio ({} samples, repeat: {})", samples.len(), repeat)
            }
            AudioSource::PassThrough => "pass-through mode".to_string(),
        };
        
        info!("🎵 Started audio transmission ({}, {}ms packets)", source_desc, self.config.interval.as_millis());
        
        let rtp_session = self.rtp_session.clone();
        let is_active = self.is_active.clone();
        let audio_generator = self.audio_generator.clone();
        let timestamp = self.timestamp.clone();
        let mut interval_timer = interval(self.config.interval);
        let samples_per_packet = self.config.samples_per_packet;
        
        tokio::spawn(async move {
            while *is_active.read().await {
                interval_timer.tick().await;
                
                // Generate audio samples
                let audio_samples = {
                    let mut generator = audio_generator.lock().await;
                    generator.generate_pcmu_samples(samples_per_packet)
                };
                
                // Send RTP packet (only if not in pass-through mode)
                if !matches!(audio_samples.as_slice(), [0x7F, ..] if audio_samples.iter().all(|&x| x == 0x7F)) {
                    let current_timestamp = {
                        let mut ts = timestamp.lock().await;
                        let current = *ts;
                        *ts = ts.wrapping_add(samples_per_packet as u32);
                        current
                    };
                    
                    let mut session = rtp_session.lock().await;
                    if let Err(e) = session.send_packet(current_timestamp, Bytes::from(audio_samples), false).await {
                        error!("Failed to send RTP audio packet: {}", e);
                    } else {
                        debug!("📡 Sent RTP audio packet (timestamp: {}, {} samples)", current_timestamp, samples_per_packet);
                    }
                }
            }
            
            info!("🛑 Stopped audio transmission");
        });
    }
    
    /// Stop audio transmission
    pub async fn stop(&self) {
        *self.is_active.write().await = false;
        info!("🛑 Stopping audio transmission");
    }
    
    /// Check if transmission is active
    pub async fn is_active(&self) -> bool {
        *self.is_active.read().await
    }
    
    /// Update the audio source during transmission
    pub async fn set_audio_source(&self, source: AudioSource) {
        let mut generator = self.audio_generator.lock().await;
        generator.set_source(source);
        info!("🔄 Updated audio source");
    }
    
    /// Set custom audio samples for transmission
    pub async fn set_custom_audio(&self, samples: Vec<u8>, repeat: bool) {
        let source = AudioSource::CustomSamples { samples, repeat };
        self.set_audio_source(source).await;
    }
    
    /// Set tone generation parameters
    pub async fn set_tone(&self, frequency: f64, amplitude: f64) {
        let source = AudioSource::Tone { frequency, amplitude };
        self.set_audio_source(source).await;
    }
    
    /// Enable pass-through mode (no audio generation)
    pub async fn set_pass_through(&self) {
        let source = AudioSource::PassThrough;
        self.set_audio_source(source).await;
    }
} 