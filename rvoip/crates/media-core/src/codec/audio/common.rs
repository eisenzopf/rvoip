//! Common audio codec types and utilities

use crate::{AudioBuffer, AudioFormat, SampleRate};
use crate::error::Result;

/// Standard audio codec frame sizes in milliseconds
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameSize {
    /// 10ms frame (typical for G.711)
    Ms10 = 10,
    /// 20ms frame (common for many codecs)
    Ms20 = 20,
    /// 30ms frame (iLBC)
    Ms30 = 30,
    /// 40ms frame (some modes of Opus)
    Ms40 = 40,
    /// 60ms frame (maximum for Opus)
    Ms60 = 60,
}

impl FrameSize {
    /// Convert frame size to milliseconds
    pub fn as_ms(&self) -> u32 {
        *self as u32
    }
    
    /// Get the number of samples in a frame at the given sample rate
    pub fn samples(&self, sample_rate: SampleRate) -> usize {
        let samples_per_ms = sample_rate.as_hz() as usize / 1000;
        samples_per_ms * self.as_ms() as usize
    }
    
    /// Create from raw milliseconds, defaulting to 20ms if not a standard size
    pub fn from_ms(ms: u32) -> Self {
        match ms {
            10 => Self::Ms10,
            20 => Self::Ms20,
            30 => Self::Ms30,
            40 => Self::Ms40,
            60 => Self::Ms60,
            _ => Self::Ms20, // Default to 20ms
        }
    }
}

impl Default for FrameSize {
    fn default() -> Self {
        Self::Ms20 // Default to 20ms frames
    }
}

/// Codec quality mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityMode {
    /// Optimize for voice clarity (speech)
    Voice,
    /// Optimize for music reproduction
    Music,
    /// Balanced quality for mixed content
    Balanced,
}

impl Default for QualityMode {
    fn default() -> Self {
        Self::Voice // Default to voice optimization for VoIP
    }
}

/// Audio codec bitrate mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateMode {
    /// Constant bitrate
    Constant,
    /// Variable bitrate
    Variable,
}

impl Default for BitrateMode {
    fn default() -> Self {
        Self::Constant // Default to constant bitrate
    }
}

/// Audio codec parameters
#[derive(Debug, Clone)]
pub struct AudioCodecParameters {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u8,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Complexity setting (0-10)
    pub complexity: u8,
    /// Bitrate mode (constant, variable)
    pub bitrate_mode: BitrateMode,
    /// Quality mode (voice, music, etc)
    pub quality_mode: QualityMode,
    /// Forward error correction enabled
    pub fec_enabled: bool,
    /// Discontinuous transmission enabled
    pub dtx_enabled: bool,
    /// Frame duration in milliseconds
    pub frame_duration_ms: u32,
    /// Expected packet loss percentage
    pub packet_loss_percentage: f32,
}

impl Default for AudioCodecParameters {
    fn default() -> Self {
        Self {
            sample_rate: 8000,  // Default to 8kHz for telephony
            channels: 1,
            bitrate: 64000,     // 64kbps for G.711
            complexity: 5,
            bitrate_mode: BitrateMode::Constant,
            quality_mode: QualityMode::Voice,
            fec_enabled: false,
            dtx_enabled: false,
            frame_duration_ms: 20,
            packet_loss_percentage: 0.0,
        }
    }
}

/// Calculate the expected PCM frame size in bytes
pub fn pcm_frame_size(sample_rate: SampleRate, frame_size: FrameSize, channels: u8, bytes_per_sample: usize) -> usize {
    let samples = frame_size.samples(sample_rate) * channels as usize;
    samples * bytes_per_sample
}

/// Calculate the expected codec frame size in bytes
pub fn codec_frame_size(sample_rate: SampleRate, frame_size: FrameSize, bitrate: u32) -> usize {
    let duration_sec = frame_size.as_ms() as f64 / 1000.0;
    let bytes = (bitrate as f64 * duration_sec / 8.0).ceil() as usize;
    bytes
} 