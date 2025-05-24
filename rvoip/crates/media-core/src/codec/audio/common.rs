use crate::{AudioBuffer, AudioFormat, SampleRate};
use crate::codec::{AudioCodec, CodecParameters};
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

/// Common audio codec parameters and types

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
            sample_rate: 48000,
            channels: 1,
            bitrate: 32000,
            complexity: 5,
            bitrate_mode: BitrateMode::Variable,
            quality_mode: QualityMode::Voice,
            fec_enabled: true,
            dtx_enabled: true,
            frame_duration_ms: 20,
            packet_loss_percentage: 0.0,
        }
    }
}

/// Channel layout
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLayout {
    /// Mono (1 channel)
    Mono,
    /// Stereo (2 channels)
    Stereo,
    /// 2.1 channels (left, right, LFE)
    TwoPointOne,
    /// 5.1 channels (front L/C/R, surround L/R, LFE)
    FivePointOne,
    /// 7.1 channels
    SevenPointOne,
    /// Custom channel layout
    Custom(u8),
}

impl ChannelLayout {
    /// Get the number of channels in this layout
    pub fn channel_count(&self) -> u8 {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::TwoPointOne => 3,
            Self::FivePointOne => 6,
            Self::SevenPointOne => 8,
            Self::Custom(count) => *count,
        }
    }
    
    /// Create a channel layout from a channel count
    pub fn from_count(count: u8) -> Self {
        match count {
            1 => Self::Mono,
            2 => Self::Stereo,
            3 => Self::TwoPointOne,
            6 => Self::FivePointOne,
            8 => Self::SevenPointOne,
            _ => Self::Custom(count),
        }
    }
}

/// Audio sample rate in Hz
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleRate {
    /// 8 kHz (narrowband)
    NarrowBand,
    /// 16 kHz (wideband)
    WideBand,
    /// 32 kHz (super-wideband)
    SuperWideBand,
    /// 48 kHz (fullband)
    FullBand,
    /// Custom sample rate
    Custom(u32),
}

impl SampleRate {
    /// Get the sample rate in Hz
    pub fn as_hz(&self) -> u32 {
        match self {
            Self::NarrowBand => 8000,
            Self::WideBand => 16000,
            Self::SuperWideBand => 32000,
            Self::FullBand => 48000,
            Self::Custom(rate) => *rate,
        }
    }
    
    /// Create a sample rate from Hz value
    pub fn from_hz(hz: u32) -> Self {
        match hz {
            8000 => Self::NarrowBand,
            16000 => Self::WideBand,
            32000 => Self::SuperWideBand,
            48000 => Self::FullBand,
            _ => Self::Custom(hz),
        }
    }
}

/// Audio format descriptor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Sample rate
    pub sample_rate: SampleRate,
    /// Channel layout
    pub channels: ChannelLayout,
    /// Sample format
    pub format: SampleFormat,
}

impl AudioFormat {
    /// Create a new audio format descriptor
    pub fn new(sample_rate: SampleRate, channels: ChannelLayout, format: SampleFormat) -> Self {
        Self {
            sample_rate,
            channels,
            format,
        }
    }
    
    /// Create a common PCM format (16-bit, 48kHz, stereo)
    pub fn pcm_stereo() -> Self {
        Self {
            sample_rate: SampleRate::FullBand,
            channels: ChannelLayout::Stereo,
            format: SampleFormat::S16,
        }
    }
    
    /// Create a common PCM format for telephony (16-bit, 8kHz, mono)
    pub fn pcm_telephony() -> Self {
        Self {
            sample_rate: SampleRate::NarrowBand,
            channels: ChannelLayout::Mono,
            format: SampleFormat::S16,
        }
    }
    
    /// Get the byte size of one sample
    pub fn bytes_per_sample(&self) -> usize {
        self.format.bytes_per_sample()
    }
    
    /// Get the number of channels
    pub fn channel_count(&self) -> u8 {
        self.channels.channel_count()
    }
    
    /// Calculate bytes per frame (for a given duration)
    pub fn bytes_per_frame(&self, duration_ms: u32) -> usize {
        let samples = (self.sample_rate.as_hz() as u64 * duration_ms as u64) / 1000;
        samples as usize * self.channel_count() as usize * self.bytes_per_sample()
    }
}

/// Audio sample format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    /// Unsigned 8-bit
    U8,
    /// Signed 16-bit
    S16,
    /// Signed 24-bit
    S24,
    /// Signed 32-bit
    S32,
    /// 32-bit float
    F32,
    /// 64-bit float
    F64,
}

impl SampleFormat {
    /// Get the number of bytes per sample
    pub fn bytes_per_sample(&self) -> usize {
        match self {
            Self::U8 => 1,
            Self::S16 => 2,
            Self::S24 => 3,
            Self::S32 => 4,
            Self::F32 => 4,
            Self::F64 => 8,
        }
    }
    
    /// Get the bit depth
    pub fn bit_depth(&self) -> u8 {
        match self {
            Self::U8 => 8,
            Self::S16 => 16,
            Self::S24 => 24,
            Self::S32 => 32,
            Self::F32 => 32,
            Self::F64 => 64,
        }
    }
    
    /// Check if the format is floating point
    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
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