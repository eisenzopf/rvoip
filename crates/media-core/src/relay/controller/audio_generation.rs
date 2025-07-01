//! Audio generation and transmission functionality
//!
//! This module provides audio generation capabilities for testing and
//! audio transmission management for RTP sessions.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info};
use bytes::Bytes;

use rvoip_rtp_core::RtpSession;

/// Audio generator for creating test tones and audio streams
pub struct AudioGenerator {
    /// Sample rate (Hz)
    sample_rate: u32,
    /// Current phase for sine wave generation
    phase: f64,
    /// Frequency of the generated tone (Hz)
    frequency: f64,
    /// Amplitude (0.0 to 1.0)
    amplitude: f64,
}

impl AudioGenerator {
    /// Create a new audio generator
    pub fn new(sample_rate: u32, frequency: f64, amplitude: f64) -> Self {
        Self {
            sample_rate,
            phase: 0.0,
            frequency,
            amplitude,
        }
    }
    
    /// Generate audio samples for PCMU (G.711 μ-law) encoding
    pub fn generate_pcmu_samples(&mut self, num_samples: usize) -> Vec<u8> {
        let mut samples = Vec::with_capacity(num_samples);
        let phase_increment = 2.0 * std::f64::consts::PI * self.frequency / self.sample_rate as f64;
        
        for _ in 0..num_samples {
            // Generate sine wave sample
            let sample = (self.phase.sin() * self.amplitude * 32767.0) as i16;
            
            // Convert to μ-law (simplified implementation)
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
    
    /// Convert linear PCM to μ-law (G.711)
    fn linear_to_ulaw(pcm: i16) -> u8 {
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
}

/// Audio transmission task for RTP sessions
pub struct AudioTransmitter {
    /// RTP session for transmission
    rtp_session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Audio generator
    audio_generator: AudioGenerator,
    /// Transmission interval (20ms for standard audio)
    interval: Duration,
    /// Current RTP timestamp
    timestamp: u32,
    /// Samples per packet (160 samples for 20ms at 8kHz)
    samples_per_packet: usize,
    /// Whether transmission is active
    is_active: Arc<RwLock<bool>>,
}

impl AudioTransmitter {
    /// Create a new audio transmitter
    pub fn new(rtp_session: Arc<tokio::sync::Mutex<RtpSession>>) -> Self {
        Self {
            rtp_session,
            audio_generator: AudioGenerator::new(8000, 440.0, 0.5), // 440Hz tone at 8kHz
            interval: Duration::from_millis(20), // 20ms packets
            timestamp: 0,
            samples_per_packet: 160, // 20ms * 8000 samples/sec = 160 samples
            is_active: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Start audio transmission
    pub async fn start(&mut self) {
        *self.is_active.write().await = true;
        info!("🎵 Started audio transmission (440Hz tone, 20ms packets)");
        
        let rtp_session = self.rtp_session.clone();
        let is_active = self.is_active.clone();
        let mut interval_timer = interval(self.interval);
        let mut timestamp = self.timestamp;
        let mut audio_gen = AudioGenerator::new(8000, 440.0, 0.5);
        
        tokio::spawn(async move {
            while *is_active.read().await {
                interval_timer.tick().await;
                
                // Generate audio samples
                let audio_samples = audio_gen.generate_pcmu_samples(160); // 160 samples for 20ms
                
                // Send RTP packet
                {
                    let mut session = rtp_session.lock().await;
                    if let Err(e) = session.send_packet(timestamp, Bytes::from(audio_samples), false).await {
                        error!("Failed to send RTP audio packet: {}", e);
                    } else {
                        debug!("📡 Sent RTP audio packet (timestamp: {}, 160 samples)", timestamp);
                    }
                }
                
                // Update timestamp (160 samples at 8kHz = 20ms)
                timestamp = timestamp.wrapping_add(160);
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
} 