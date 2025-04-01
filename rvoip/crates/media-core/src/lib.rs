//! Media Core library for the RVOIP project
//! 
//! This crate provides codec implementations, media processing utilities,
//! and other media handling functionality.

pub mod error;
pub mod codec;

pub use error::Error;
pub use codec::Codec;

/// Result type for media operations
pub type Result<T> = std::result::Result<T, Error>;

/// Media sample type (raw audio data)
pub type Sample = i16;

/// PCM sample rate in Hz
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// 8kHz (narrowband)
    Rate8000 = 8000,
    /// 16kHz (wideband)
    Rate16000 = 16000,
    /// 32kHz
    Rate32000 = 32000,
    /// 44.1kHz (CD quality)
    Rate44100 = 44100,
    /// 48kHz
    Rate48000 = 48000,
}

impl SampleRate {
    /// Get the sample rate in Hz
    pub fn as_hz(&self) -> u32 {
        *self as u32
    }
    
    /// Create from a raw Hz value, defaulting to 8kHz if not recognized
    pub fn from_hz(hz: u32) -> Self {
        match hz {
            8000 => Self::Rate8000,
            16000 => Self::Rate16000,
            32000 => Self::Rate32000,
            44100 => Self::Rate44100,
            48000 => Self::Rate48000,
            _ => Self::Rate8000, // Default to 8kHz
        }
    }
}

impl Default for SampleRate {
    fn default() -> Self {
        Self::Rate8000 // Default to 8kHz (common for telephony)
    }
}

/// Audio format (channels, bit depth, sample rate)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Number of channels (1 for mono, 2 for stereo)
    pub channels: u8,
    /// Bits per sample (typically 8, 16, or 32)
    pub bit_depth: u8,
    /// Sample rate in Hz
    pub sample_rate: SampleRate,
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            channels: 1,         // Default to mono
            bit_depth: 16,       // Default to 16-bit
            sample_rate: SampleRate::default(),
        }
    }
}

impl AudioFormat {
    /// Create a new audio format
    pub fn new(channels: u8, bit_depth: u8, sample_rate: SampleRate) -> Self {
        Self {
            channels,
            bit_depth,
            sample_rate,
        }
    }
    
    /// Create a new mono 16-bit format with the given sample rate
    pub fn mono_16bit(sample_rate: SampleRate) -> Self {
        Self::new(1, 16, sample_rate)
    }
    
    /// Create a new stereo 16-bit format with the given sample rate
    pub fn stereo_16bit(sample_rate: SampleRate) -> Self {
        Self::new(2, 16, sample_rate)
    }
    
    /// Standard narrowband telephony format (mono, 16-bit, 8kHz)
    pub fn telephony() -> Self {
        Self::mono_16bit(SampleRate::Rate8000)
    }
}

/// A chunk of audio samples (PCM or encoded)
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    /// Raw audio data
    pub data: bytes::Bytes,
    /// Audio format information
    pub format: AudioFormat,
}

impl AudioBuffer {
    /// Create a new audio buffer with the given data and format
    pub fn new(data: bytes::Bytes, format: AudioFormat) -> Self {
        Self { data, format }
    }
    
    /// Get the duration of the audio in milliseconds
    pub fn duration_ms(&self) -> u32 {
        let bytes_per_sample = (self.format.bit_depth / 8) as u32;
        let samples = (self.data.len() as u32) / bytes_per_sample / (self.format.channels as u32);
        (samples * 1000) / self.format.sample_rate.as_hz()
    }
    
    /// Get the number of samples in the buffer
    pub fn samples(&self) -> usize {
        let bytes_per_sample = (self.format.bit_depth / 8) as usize;
        self.data.len() / bytes_per_sample / (self.format.channels as usize)
    }
} 