use crate::{AudioBuffer, Sample, SampleRate};
use crate::error::Result;

/// Audio processing quality level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessingQuality {
    /// Low quality, lower CPU usage
    Low,
    /// Medium quality, balanced CPU usage
    Medium,
    /// High quality, higher CPU usage
    High,
}

impl Default for ProcessingQuality {
    fn default() -> Self {
        Self::Medium
    }
}

/// Voice activity detection level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadMode {
    /// Very aggressive VAD (more likely to classify as non-speech)
    VeryAggressive = 3,
    /// Aggressive VAD
    Aggressive = 2,
    /// Moderate VAD (balanced)
    Moderate = 1,
    /// Quality VAD (more likely to classify as speech)
    Quality = 0,
}

impl Default for VadMode {
    fn default() -> Self {
        Self::Moderate
    }
}

/// Noise suppression level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseSuppressionLevel {
    /// No suppression
    None = 0,
    /// Low suppression
    Low = 1,
    /// Moderate suppression
    Moderate = 2,
    /// High suppression
    High = 3,
    /// Very high suppression
    VeryHigh = 4,
}

impl Default for NoiseSuppressionLevel {
    fn default() -> Self {
        Self::Moderate
    }
}

/// Echo cancellation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EchoMode {
    /// No echo cancellation
    Off,
    /// Conference mode (balanced)
    Conference,
    /// Desktop sharing (aggressive)
    Desktop,
}

impl Default for EchoMode {
    fn default() -> Self {
        Self::Conference
    }
}

/// Gain control mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GainControlMode {
    /// Adaptive gain control (automatically adjust levels)
    Adaptive,
    /// Fixed gain control (apply a fixed gain)
    Fixed,
    /// Off (no gain control)
    Off,
}

impl Default for GainControlMode {
    fn default() -> Self {
        Self::Adaptive
    }
}

/// Voice activity status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceActivityStatus {
    /// Active speech detected
    Active,
    /// No speech detected
    Inactive,
    /// Uncertain (could be quiet speech or noise)
    Uncertain,
}

/// Calculate the RMS (Root Mean Square) level of an audio buffer
pub fn calculate_rms(buffer: &AudioBuffer) -> f32 {
    let bytes_per_sample = (buffer.format.bit_depth / 8) as usize;
    let samples_count = buffer.data.len() / bytes_per_sample / (buffer.format.channels as usize);
    
    if samples_count == 0 {
        return 0.0;
    }
    
    let max_value = if buffer.format.bit_depth == 8 {
        128.0
    } else {
        32768.0 // 16-bit
    };
    
    let mut sum_squares = 0.0;
    
    for i in 0..samples_count {
        let mut sample_sum = 0.0;
        
        // Average all channels for this sample
        for ch in 0..buffer.format.channels {
            let offset = (i * buffer.format.channels as usize + ch as usize) * bytes_per_sample;
            let sample_bytes = &buffer.data[offset..offset + bytes_per_sample];
            
            let sample = if bytes_per_sample == 2 {
                // 16-bit
                i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]) as f32
            } else {
                // 8-bit
                (sample_bytes[0] as f32 - 128.0) * 256.0
            };
            
            sample_sum += sample;
        }
        
        // Average the channels
        let sample_avg = sample_sum / buffer.format.channels as f32;
        
        // Normalize to -1.0 to 1.0 range
        let normalized = sample_avg / max_value;
        
        sum_squares += normalized * normalized;
    }
    
    let mean_square = sum_squares / samples_count as f32;
    mean_square.sqrt()
}

/// Convert a floating point sample (-1.0 to 1.0) to a PCM sample
pub fn float_to_pcm(value: f32, bit_depth: u8) -> Sample {
    match bit_depth {
        8 => (value * 128.0) as Sample,
        16 => (value * 32767.0) as Sample,
        _ => (value * 32767.0) as Sample, // Default to 16-bit
    }
}

/// Convert a PCM sample to a floating point value (-1.0 to 1.0)
pub fn pcm_to_float(value: Sample, bit_depth: u8) -> f32 {
    match bit_depth {
        8 => value as f32 / 128.0,
        16 => value as f32 / 32768.0,
        _ => value as f32 / 32768.0, // Default to 16-bit
    }
}

/// A simple envelope follower for tracking audio levels
pub struct EnvelopeFollower {
    /// Attack time in milliseconds (how fast the envelope rises)
    attack_ms: f32,
    /// Release time in milliseconds (how fast the envelope falls)
    release_ms: f32,
    /// Current envelope value
    value: f32,
    /// Sample rate
    sample_rate: SampleRate,
}

impl EnvelopeFollower {
    /// Create a new envelope follower
    pub fn new(sample_rate: SampleRate, attack_ms: f32, release_ms: f32) -> Self {
        Self {
            attack_ms,
            release_ms,
            value: 0.0,
            sample_rate,
        }
    }
    
    /// Process a buffer and update the envelope
    pub fn process(&mut self, buffer: &AudioBuffer) -> Result<()> {
        let bytes_per_sample = (buffer.format.bit_depth / 8) as usize;
        let samples_count = buffer.data.len() / bytes_per_sample / (buffer.format.channels as usize);
        
        if samples_count == 0 {
            return Ok(());
        }
        
        let attack_coef = (-1.0 / (self.attack_ms * buffer.format.sample_rate.as_hz() as f32 / 1000.0)).exp();
        let release_coef = (-1.0 / (self.release_ms * buffer.format.sample_rate.as_hz() as f32 / 1000.0)).exp();
        
        for i in 0..samples_count {
            let mut sample_sum = 0.0;
            
            // Average all channels for this sample
            for ch in 0..buffer.format.channels {
                let offset = (i * buffer.format.channels as usize + ch as usize) * bytes_per_sample;
                let sample_bytes = &buffer.data[offset..offset + bytes_per_sample];
                
                let sample = if bytes_per_sample == 2 {
                    // 16-bit
                    i16::from_le_bytes([sample_bytes[0], sample_bytes[1]]) as f32
                } else {
                    // 8-bit
                    (sample_bytes[0] as f32 - 128.0) * 256.0
                };
                
                sample_sum += sample.abs();
            }
            
            // Average the channels
            let sample_abs = sample_sum / buffer.format.channels as f32;
            
            // Normalize to 0.0 to 1.0 range
            let max_value = if buffer.format.bit_depth == 8 { 128.0 } else { 32768.0 };
            let normalized = sample_abs / max_value;
            
            // Update envelope
            if normalized > self.value {
                self.value = attack_coef * (self.value - normalized) + normalized;
            } else {
                self.value = release_coef * (self.value - normalized) + normalized;
            }
        }
        
        Ok(())
    }
    
    /// Get the current envelope value (0.0 to 1.0)
    pub fn value(&self) -> f32 {
        self.value
    }
    
    /// Reset the envelope follower
    pub fn reset(&mut self) {
        self.value = 0.0;
    }
    
    /// Update the sample rate
    pub fn set_sample_rate(&mut self, sample_rate: SampleRate) {
        self.sample_rate = sample_rate;
    }
    
    /// Update attack time (milliseconds)
    pub fn set_attack(&mut self, attack_ms: f32) {
        self.attack_ms = attack_ms;
    }
    
    /// Update release time (milliseconds)
    pub fn set_release(&mut self, release_ms: f32) {
        self.release_ms = release_ms;
    }
} 