//! iLBC (Internet Low Bitrate Codec) implementation
//!
//! iLBC is designed for narrow band speech with graceful speech quality degradation in case of
//! lost frames, making it particularly suitable for VoIP applications.

use std::sync::Arc;
use bytes::Bytes;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};
use crate::SampleRate;
use crate::codec::{Codec, CodecCapabilities, CodecParameters, CodecInfo, PayloadType};
use super::common::AudioCodec;

/// Frame modes for iLBC
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IlbcMode {
    /// 20ms frame mode (15.2 kbps)
    Mode20Ms,
    /// 30ms frame mode (13.33 kbps)
    Mode30Ms,
}

/// iLBC codec implementation
#[derive(Debug)]
pub struct IlbcCodec {
    /// Codec parameters
    params: CodecParameters,
    
    /// Frame mode (20ms or 30ms)
    mode: IlbcMode,
    
    /// Enhanced PLC (Packet Loss Concealment)
    enhanced_plc: bool,
}

impl Default for IlbcCodec {
    fn default() -> Self {
        Self {
            params: CodecParameters::new("iLBC", PayloadType::Dynamic(102)),
            mode: IlbcMode::Mode20Ms,
            enhanced_plc: true,
        }
    }
}

impl AudioCodec for IlbcCodec {
    fn encode(&self, _input: &[i16], _output: &mut [u8]) -> Result<usize> {
        // Stub implementation
        Err(Error::NotImplemented("iLBC encoding not yet implemented".to_string()))
    }
    
    fn decode(&self, _input: &[u8], _output: &mut [i16]) -> Result<usize> {
        // Stub implementation
        Err(Error::NotImplemented("iLBC decoding not yet implemented".to_string()))
    }
    
    fn sample_rate(&self) -> SampleRate {
        SampleRate::Rate8000 // iLBC is an 8kHz codec
    }
    
    fn channels(&self) -> u8 {
        1 // Mono only
    }
}

impl Codec for IlbcCodec {
    fn name(&self) -> &str {
        "iLBC"
    }
    
    fn payload_type(&self) -> PayloadType {
        self.params.payload_type
    }
    
    fn clock_rate(&self) -> u32 {
        8000 // 8kHz sampling rate
    }
    
    fn encode_bytes(&self, _input: &Bytes) -> Result<Bytes> {
        // Stub implementation
        Err(Error::NotImplemented("iLBC encoding not yet implemented".to_string()))
    }
    
    fn decode_bytes(&self, _input: &Bytes) -> Result<Bytes> {
        // Stub implementation
        Err(Error::NotImplemented("iLBC decoding not yet implemented".to_string()))
    }
    
    fn capabilities(&self) -> CodecCapabilities {
        CodecCapabilities {
            mime_type: "audio/iLBC".to_string(),
            channels: 1,
            clock_rate: 8000,
            features: vec![
                match self.mode {
                    IlbcMode::Mode20Ms => "mode=20".to_string(),
                    IlbcMode::Mode30Ms => "mode=30".to_string(),
                }
            ],
        }
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "iLBC".to_string(),
            description: "Internet Low Bitrate Codec (RFC 3951)".to_string(),
            media_type: "audio".to_string(),
            parameters: self.params.clone(),
        }
    }
    
    fn frame_duration_ms(&self) -> f32 {
        match self.mode {
            IlbcMode::Mode20Ms => 20.0,
            IlbcMode::Mode30Ms => 30.0,
        }
    }
}

/// Builder for creating iLBC codec instances
pub struct IlbcCodecBuilder {
    codec: IlbcCodec,
}

impl IlbcCodecBuilder {
    /// Create a new IlbcCodecBuilder
    pub fn new() -> Self {
        Self {
            codec: IlbcCodec::default(),
        }
    }
    
    /// Set the frame mode (20ms or 30ms)
    pub fn with_mode(mut self, mode: IlbcMode) -> Self {
        self.codec.mode = mode;
        self
    }
    
    /// Set 20ms frame mode
    pub fn with_20ms_mode(mut self) -> Self {
        self.codec.mode = IlbcMode::Mode20Ms;
        self
    }
    
    /// Set 30ms frame mode
    pub fn with_30ms_mode(mut self) -> Self {
        self.codec.mode = IlbcMode::Mode30Ms;
        self
    }
    
    /// Enable or disable enhanced PLC
    pub fn with_enhanced_plc(mut self, enabled: bool) -> Self {
        self.codec.enhanced_plc = enabled;
        self
    }
    
    /// Build the iLBC codec
    pub fn build(self) -> IlbcCodec {
        self.codec
    }
} 