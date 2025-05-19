//! Opus audio codec implementation
//!
//! Opus is a lossy audio coding format developed by the Internet Engineering Task Force (IETF)
//! that is particularly suitable for interactive real-time applications over the Internet.

use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::SampleRate;
use crate::codec::{Codec, CodecCapabilities, CodecParameters, CodecInfo, PayloadType};
use super::common::AudioCodec;

/// Opus codec implementation
#[derive(Debug)]
pub struct OpusCodec {
    /// Codec parameters
    params: CodecParameters,
    
    /// Bitrate (in bits per second)
    bitrate: u32,
    
    /// Complexity (0-10, higher is more CPU intensive but better quality)
    complexity: u32,
    
    /// Forward error correction enabled
    fec_enabled: bool,
    
    /// DTX (discontinuous transmission) enabled
    dtx_enabled: bool,
    
    /// Voice optimization mode
    voice_mode: bool,
    
    /// Frame size in milliseconds (2.5, 5, 10, 20, 40, 60, 80, 100, 120)
    frame_ms: f32,
}

impl Default for OpusCodec {
    fn default() -> Self {
        Self {
            params: CodecParameters::new("opus", PayloadType::Dynamic(111)),
            bitrate: 32000, // 32 kbps default
            complexity: 5,
            fec_enabled: true,
            dtx_enabled: false,
            voice_mode: true,
            frame_ms: 20.0,
        }
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&self, _input: &[i16], _output: &mut [u8]) -> Result<usize> {
        // Stub implementation
        Err(Error::NotImplemented("Opus encoding not yet implemented".to_string()))
    }
    
    fn decode(&self, _input: &[u8], _output: &mut [i16]) -> Result<usize> {
        // Stub implementation
        Err(Error::NotImplemented("Opus decoding not yet implemented".to_string()))
    }
    
    fn sample_rate(&self) -> SampleRate {
        SampleRate::Rate48000 // Opus uses 48kHz internally
    }
    
    fn channels(&self) -> u8 {
        2 // Stereo support
    }
}

impl Codec for OpusCodec {
    fn name(&self) -> &str {
        "opus"
    }
    
    fn payload_type(&self) -> PayloadType {
        self.params.payload_type
    }
    
    fn clock_rate(&self) -> u32 {
        48000 // Always 48kHz for Opus
    }
    
    fn encode_bytes(&self, _input: &Bytes) -> Result<Bytes> {
        // Stub implementation
        Err(Error::NotImplemented("Opus encoding not yet implemented".to_string()))
    }
    
    fn decode_bytes(&self, _input: &Bytes) -> Result<Bytes> {
        // Stub implementation
        Err(Error::NotImplemented("Opus decoding not yet implemented".to_string()))
    }
    
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            mime_type: "audio/opus".to_string(),
            channels: 2,
            clock_rate: 48000,
            features: vec![
                "stereo".to_string(),
                "vbr".to_string(),
                "fec".to_string(),
                "dtx".to_string(),
            ],
        }
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "opus".to_string(),
            description: "Opus audio codec (RFC 6716)".to_string(),
            media_type: "audio".to_string(),
            parameters: self.params.clone(),
        }
    }
    
    fn frame_duration_ms(&self) -> f32 {
        self.frame_ms
    }
}

/// Builder for creating Opus codec instances
pub struct OpusCodecBuilder {
    codec: OpusCodec,
}

impl OpusCodecBuilder {
    /// Create a new OpusCodecBuilder
    pub fn new() -> Self {
        Self {
            codec: OpusCodec::default(),
        }
    }
    
    /// Set bitrate in bits per second
    pub fn with_bitrate(mut self, bitrate: u32) -> Self {
        self.codec.bitrate = bitrate;
        self
    }
    
    /// Set complexity (0-10)
    pub fn with_complexity(mut self, complexity: u32) -> Self {
        self.codec.complexity = complexity.min(10);
        self
    }
    
    /// Enable or disable forward error correction
    pub fn with_fec(mut self, enabled: bool) -> Self {
        self.codec.fec_enabled = enabled;
        self
    }
    
    /// Enable or disable DTX
    pub fn with_dtx(mut self, enabled: bool) -> Self {
        self.codec.dtx_enabled = enabled;
        self
    }
    
    /// Set voice optimization mode
    pub fn with_voice_mode(mut self, enabled: bool) -> Self {
        self.codec.voice_mode = enabled;
        self
    }
    
    /// Build the Opus codec
    pub fn build(self) -> OpusCodec {
        self.codec
    }
} 