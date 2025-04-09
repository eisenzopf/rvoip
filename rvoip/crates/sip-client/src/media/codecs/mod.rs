mod pcmu;
mod pcma;
mod opus_codec;
#[cfg(feature = "g729")]
mod g729_codec;

pub use pcmu::PcmuCodec;
pub use pcma::PcmaCodec;
pub use opus_codec::OpusCodec;
#[cfg(feature = "g729")]
pub use g729_codec::G729Codec;

use crate::error::{Error, Result};

/// Audio codec interface
pub trait AudioCodec {
    /// Encode PCM audio data to codec-specific format
    fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>>;
    
    /// Decode codec-specific format to PCM audio data
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>>;
    
    /// Get the sample rate used by this codec
    fn get_sample_rate(&self) -> u32;
    
    /// Get the number of channels used by this codec
    fn get_channels(&self) -> u8;
    
    /// Get the frame size in samples
    fn get_frame_size(&self) -> usize;
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

impl Default for CodecParams {
    fn default() -> Self {
        Self {
            codec_type: CodecType::Pcmu,
            sample_rate: None,
            channels: None,
            bitrate: None,
            frame_size: None,
            payload_type: None,
        }
    }
}

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodecType {
    /// G.711 μ-law (PCMU)
    Pcmu,
    
    /// G.711 A-law (PCMA)
    Pcma,
    
    /// G.722
    G722,
    
    /// G.729
    G729,
    
    /// Opus
    Opus,
}

impl CodecType {
    /// Get the default payload type for this codec
    pub fn default_payload_type(&self) -> u8 {
        match self {
            CodecType::Pcmu => 0,
            CodecType::Pcma => 8,
            CodecType::G722 => 9,
            CodecType::G729 => 18,
            CodecType::Opus => 111, // Dynamic payload type
        }
    }
    
    /// Get the default sample rate for this codec
    pub fn default_sample_rate(&self) -> u32 {
        match self {
            CodecType::Pcmu => 8000,
            CodecType::Pcma => 8000,
            CodecType::G722 => 16000,
            CodecType::G729 => 8000,
            CodecType::Opus => 48000,
        }
    }
    
    /// Get the SDP encoding name for this codec
    pub fn sdp_encoding_name(&self) -> &'static str {
        match self {
            CodecType::Pcmu => "PCMU",
            CodecType::Pcma => "PCMA",
            CodecType::G722 => "G722",
            CodecType::G729 => "G729",
            CodecType::Opus => "opus",
        }
    }
}

/// Create a new audio codec instance
pub fn create_codec(params: CodecParams) -> Result<Box<dyn AudioCodec>> {
    match params.codec_type {
        CodecType::Pcmu => {
            Ok(Box::new(PcmuCodec::new(params)?))
        },
        CodecType::Pcma => {
            Ok(Box::new(PcmaCodec::new(params)?))
        },
        CodecType::G722 => {
            Err(Error::Codec("G.722 codec not yet implemented".into()))
        },
        CodecType::G729 => {
            #[cfg(feature = "g729")]
            {
                Ok(Box::new(G729Codec::new(params)?))
            }
            #[cfg(not(feature = "g729"))]
            {
                Err(Error::Codec("G.729 codec support not enabled".into()))
            }
        },
        CodecType::Opus => {
            #[cfg(feature = "opus")]
            {
                Ok(Box::new(OpusCodec::new(params)?))
            }
            #[cfg(not(feature = "opus"))]
            {
                Err(Error::Codec("Opus codec support not enabled".into()))
            }
        },
    }
} 