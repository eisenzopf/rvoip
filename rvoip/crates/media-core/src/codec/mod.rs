//! Audio codec implementations
//!
//! This module contains various audio codec implementations for encoding and decoding
//! audio data in different formats.

use bytes::Bytes;
use crate::{Result, AudioBuffer, AudioFormat};

/// G.711 codec implementation (μ-law and A-law)
pub mod g711;
pub use g711::{G711Codec, G711Variant};

/// Opus codec implementation
pub mod opus;
pub use opus::OpusCodec;

/// Audio codec trait for encoding and decoding audio data
pub trait Codec: Send + Sync {
    /// Get the name of the codec
    fn name(&self) -> &'static str;
    
    /// Get the payload type for this codec (as defined in RTP standards)
    fn payload_type(&self) -> u8;
    
    /// Get the native sample rate for this codec
    fn sample_rate(&self) -> u32;
    
    /// Check if the given format is supported by this codec
    fn supports_format(&self, format: AudioFormat) -> bool;
    
    /// Encode PCM audio to the codec's format
    fn encode(&self, pcm: &AudioBuffer) -> Result<Bytes>;
    
    /// Decode encoded audio data to PCM
    fn decode(&self, encoded: &[u8]) -> Result<AudioBuffer>;
    
    /// Get the frame size in samples
    /// This is the number of samples that should be encoded/decoded in one operation
    fn frame_size(&self) -> usize;
    
    /// Get the frame duration in milliseconds
    fn frame_duration_ms(&self) -> u32 {
        (self.frame_size() * 1000) as u32 / self.sample_rate()
    }
}

/// Factory function to create a codec by payload type
pub fn codec_for_payload_type(pt: u8) -> Option<Box<dyn Codec>> {
    match pt {
        0 => Some(Box::new(G711Codec::new(G711Variant::PCMU))),
        8 => Some(Box::new(G711Codec::new(G711Variant::PCMA))),
        _ => None,
    }
}

/// Codec parameters for configuration
#[derive(Debug, Clone)]
pub struct CodecParams {
    /// Codec type
    pub codec_type: CodecType,
    
    /// Sample rate
    pub sample_rate: Option<u32>,
    
    /// Number of channels
    pub channels: Option<u8>,
    
    /// Bitrate (for variable bitrate codecs)
    pub bitrate: Option<u32>,
    
    /// Frame size in samples
    pub frame_size: Option<u32>,
    
    /// Payload type
    pub payload_type: Option<u8>,
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 μ-law (PCMU)
    Pcmu,
    
    /// G.711 A-law (PCMA)
    Pcma,
    
    /// Opus
    Opus,
    
    /// G.729
    G729,
}

// Codec Framework
//
// This module provides the codec framework for media encoding and decoding.
// It defines traits for audio and video codecs, and includes implementations
// for common codecs used in VoIP applications.

use std::any::Any;
use std::fmt::Debug;
use bytes::Bytes;

use crate::{AudioBuffer, AudioFormat, SampleRate, Error, Result};

pub mod audio;
pub mod video;
pub mod registry;
pub mod traits;

// Re-export specific codecs
pub use audio::g711::G711Codec;
pub use audio::opus::OpusCodec;
pub use audio::g722::G722Codec;
// pub use audio::ilbc::IlbcCodec; // TODO: Implement iLBC codec

// Re-export traits
pub use traits::{Codec, AudioCodec, VideoCodec};

/// Payload type for RTP
pub type PayloadType = u8;

/// Codec capability
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecCapability {
    /// Codec name
    pub name: String,
    
    /// Media type (audio, video, etc.)
    pub media_type: MediaType,
    
    /// RTP payload type (dynamic types are 96-127)
    pub payload_type: PayloadType,
    
    /// Clock rate in Hz
    pub clock_rate: u32,
    
    /// Number of channels (for audio)
    pub channels: Option<u8>,
    
    /// Format parameters
    pub parameters: Vec<CodecParameter>,
}

/// Media type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio media
    Audio,
    /// Video media
    Video,
    /// Application data
    Application,
}

/// Codec parameter
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecParameter {
    /// Parameter name
    pub name: String,
    
    /// Parameter value
    pub value: String,
}

impl CodecParameter {
    /// Create a new codec parameter
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Codec parameters
#[derive(Debug, Clone, Default)]
pub struct CodecParameters {
    /// Media type
    pub media_type: MediaType,
    
    /// Clock rate in Hz
    pub clock_rate: u32,
    
    /// Number of channels (for audio)
    pub channels: u8,
    
    /// Sample rate for audio
    pub sample_rate: Option<SampleRate>,
    
    /// Bitrate target for encoding
    pub bitrate: Option<u32>,
    
    /// Maximum frame size in milliseconds
    pub max_frame_size_ms: Option<u32>,
    
    /// Frame duration in milliseconds
    pub frame_duration_ms: Option<u32>,
    
    /// Packet loss concealment enabled
    pub plc_enabled: bool,
    
    /// Forward error correction enabled
    pub fec_enabled: bool,
    
    /// DTX (Discontinuous Transmission) enabled
    pub dtx_enabled: bool,
    
    /// Additional codec-specific parameters
    pub additional_params: Vec<CodecParameter>,
}

impl CodecParameters {
    /// Create new audio codec parameters
    pub fn audio(clock_rate: u32, channels: u8) -> Self {
        Self {
            media_type: MediaType::Audio,
            clock_rate,
            channels,
            sample_rate: Some(SampleRate::from_hz(clock_rate)),
            ..Default::default()
        }
    }
    
    /// Create new video codec parameters
    pub fn video(clock_rate: u32) -> Self {
        Self {
            media_type: MediaType::Video,
            clock_rate,
            channels: 1,
            ..Default::default()
        }
    }
    
    /// Add a parameter
    pub fn with_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.additional_params.push(CodecParameter::new(name, value));
        self
    }
    
    /// Set bitrate
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.bitrate = Some(bitrate);
        self
    }
    
    /// Set frame duration
    pub fn with_frame_duration(mut self, duration_ms: u32) -> Self {
        self.frame_duration_ms = Some(duration_ms);
        self
    }
    
    /// Enable packet loss concealment
    pub fn with_plc(mut self, enabled: bool) -> Self {
        self.plc_enabled = enabled;
        self
    }
    
    /// Enable forward error correction
    pub fn with_fec(mut self, enabled: bool) -> Self {
        self.fec_enabled = enabled;
        self
    }
    
    /// Enable DTX
    pub fn with_dtx(mut self, enabled: bool) -> Self {
        self.dtx_enabled = enabled;
        self
    }
    
    /// Get a parameter by name
    pub fn get_param(&self, name: &str) -> Option<&str> {
        self.additional_params.iter()
            .find(|p| p.name == name)
            .map(|p| p.value.as_str())
    }
} 