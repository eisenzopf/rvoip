//! Codec traits
//!
//! This module defines the core traits for codec implementations.

use std::any::Any;
use bytes::Bytes;
use std::fmt::Debug;
use std::sync::Arc;

use crate::error::Result;
use crate::{AudioBuffer, AudioFormat};
use super::{CodecParameters, CodecCapability};
use crate::codec::audio::common::{AudioCodecParameters, BitrateMode, QualityMode};
use crate::codec::audio::converter::SampleFormat;

/// Codec capability descriptor for negotiation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecCapability {
    /// Unique codec identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Codec-specific format parameters
    pub parameters: Bytes,
    /// MIME type (e.g., "audio/opus", "audio/PCMA")
    pub mime_type: String,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// RTP payload type (dynamic or static)
    pub payload_type: Option<u8>,
    /// Media type (audio, video, etc.)
    pub media_type: MediaType,
    /// Feature flags indicating codec capabilities
    pub features: CodecFeatures,
    /// Bandwidth requirements in kbps (min, typical, max)
    pub bandwidth: (u32, u32, u32),
}

/// Media type for a codec
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio codec
    Audio,
    /// Video codec
    Video,
    /// Text codec
    Text,
    /// Application-specific data
    Application,
}

/// Codec feature flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecFeatures {
    /// Forward error correction support
    pub has_fec: bool,
    /// Variable bitrate support
    pub variable_bitrate: bool,
    /// Discontinuous transmission (DTX) support
    pub dtx: bool,
    /// Packet loss concealment support
    pub plc: bool,
    /// Voice activity detection support
    pub vad: bool,
    /// Frame size flexibility
    pub flexible_frames: bool,
}

impl Default for CodecFeatures {
    fn default() -> Self {
        Self {
            has_fec: false,
            variable_bitrate: false,
            dtx: false,
            plc: false,
            vad: false,
            flexible_frames: false,
        }
    }
}

/// Base trait for all media codecs
pub trait Codec: Send + Sync + Debug {
    /// Get the name of the codec
    fn name(&self) -> &'static str;
    
    /// Get the codec capabilities
    fn capabilities(&self) -> CodecCapability;
    
    /// Get the RTP payload type for this codec
    fn payload_type(&self) -> u8;
    
    /// Get the clock rate in Hz
    fn clock_rate(&self) -> u32;
    
    /// Configure the codec with the given parameters
    fn configure(&mut self, params: &CodecParameters) -> Result<()>;
    
    /// Get the codec-specific parameters
    fn parameters(&self) -> CodecParameters;
    
    /// Cast to Any for downcasting
    fn as_any(&self) -> &dyn Any;
    
    /// Cast to Any for downcasting (mutable)
    fn as_any_mut(&mut self) -> &mut dyn Any;
    
    /// Get the codec capability descriptor
    fn capability(&self) -> CodecCapability;
    
    /// Clone the codec into a new boxed instance
    fn box_clone(&self) -> Box<dyn Codec>;
    
    /// Clone the codec into a new Arc instance
    fn arc_clone(&self) -> Arc<dyn Codec> {
        Arc::new(self.box_clone())
    }
    
    /// Get the media type
    fn media_type(&self) -> MediaType {
        self.capability().media_type
    }
    
    /// Check if this codec is an audio codec
    fn is_audio(&self) -> bool {
        matches!(self.media_type(), MediaType::Audio)
    }
    
    /// Check if this codec is a video codec
    fn is_video(&self) -> bool {
        matches!(self.media_type(), MediaType::Video)
    }
}

/// Audio codec trait for encoding and decoding audio data
pub trait AudioCodec: Codec {
    /// Encode PCM audio data to the codec's format
    fn encode_audio(&self, pcm: &AudioBuffer) -> Result<Bytes>;
    
    /// Decode encoded audio data to PCM
    fn decode_audio(&self, encoded: &[u8]) -> Result<AudioBuffer>;
    
    /// Check if the given audio format is supported
    fn supports_format(&self, format: AudioFormat) -> bool;
    
    /// Get the frame size in samples
    /// This is the number of samples that should be encoded/decoded in one operation
    fn frame_size(&self) -> usize;
    
    /// Get the frame duration in milliseconds
    fn frame_duration_ms(&self) -> u32 {
        (self.frame_size() * 1000) as u32 / self.clock_rate()
    }
    
    /// Check if this codec supports packet loss concealment
    fn supports_plc(&self) -> bool;
    
    /// Apply packet loss concealment to generate audio for lost packets
    fn conceal_loss(&self, previous_frame: Option<&AudioBuffer>) -> Result<AudioBuffer>;
    
    /// Check if this codec supports discontinuous transmission (DTX)
    fn supports_dtx(&self) -> bool;
    
    /// Check if this codec supports forward error correction (FEC)
    fn supports_fec(&self) -> bool;
    
    /// Encode raw audio samples into compressed data
    fn encode(&self, input: &[i16], output: &mut Bytes) -> Result<usize>;
    
    /// Decode compressed data into raw audio samples
    fn decode(&self, input: &[u8], output: &mut [i16]) -> Result<usize>;
    
    /// Get the sample rate
    fn sample_rate(&self) -> u32;
    
    /// Get the number of channels
    fn channels(&self) -> u8;
    
    /// Set parameters for this codec
    fn set_parameters(&mut self, params: &AudioCodecParameters) -> Result<()>;
    
    /// Get the current parameters
    fn parameters(&self) -> AudioCodecParameters;
    
    /// Set the bitrate mode
    fn set_bitrate_mode(&mut self, mode: BitrateMode) -> Result<()>;
    
    /// Set the quality mode
    fn set_quality_mode(&mut self, mode: QualityMode) -> Result<()>;
    
    /// Enable forward error correction
    fn enable_fec(&mut self, enabled: bool) -> Result<()>;
    
    /// Enable discontinuous transmission
    fn enable_dtx(&mut self, enabled: bool) -> Result<()>;
    
    /// Set the complexity (0-10, higher is more complex but better quality)
    fn set_complexity(&mut self, complexity: u8) -> Result<()>;
    
    /// Set the packet loss percentage for FEC adaptation
    fn set_packet_loss(&mut self, packet_loss_pct: f32) -> Result<()>;
    
    /// Reset the codec state
    fn reset(&mut self) -> Result<()>;
}

/// Video codec trait for encoding and decoding video data
pub trait VideoCodec: Codec {
    /// Encode raw video data to the codec's format
    fn encode_video(&self, raw_video: &[u8], width: u32, height: u32) -> Result<Bytes>;
    
    /// Decode encoded video data to raw format
    fn decode_video(&self, encoded: &[u8]) -> Result<(Vec<u8>, u32, u32)>;
    
    /// Get the supported resolutions
    fn supported_resolutions(&self) -> Vec<(u32, u32)>;
    
    /// Get the current bitrate in bits per second
    fn bitrate(&self) -> u32;
    
    /// Set the target bitrate in bits per second
    fn set_bitrate(&mut self, bitrate: u32) -> Result<()>;
    
    /// Get the current frame rate
    fn framerate(&self) -> f32;
    
    /// Set the target frame rate
    fn set_framerate(&mut self, fps: f32) -> Result<()>;
    
    /// Check if this codec supports temporal scalability
    fn supports_temporal_layers(&self) -> bool;
    
    /// Set the number of temporal layers
    fn set_temporal_layers(&mut self, layers: u8) -> Result<()>;
    
    /// Check if this codec supports spatial scalability
    fn supports_spatial_layers(&self) -> bool;
    
    /// Set the number of spatial layers
    fn set_spatial_layers(&mut self, layers: u8) -> Result<()>;
    
    /// Encode raw video frame into compressed data
    fn encode_frame(&self, frame: &[u8], width: u32, height: u32, output: &mut Bytes) -> Result<usize>;
    
    /// Decode compressed data into raw video frame
    fn decode_frame(&self, input: &[u8], output: &mut [u8], width: &mut u32, height: &mut u32) -> Result<usize>;
}

/// Codec factory trait for creating codec instances
pub trait CodecFactory: Send + Sync + Debug {
    /// Get the codec identifier
    fn id(&self) -> &str;
    
    /// Get the codec name
    fn name(&self) -> &str;
    
    /// Get the supported capabilities
    fn capabilities(&self) -> Vec<CodecCapability>;
    
    /// Create a new codec instance with default parameters
    fn create_default(&self) -> Result<Box<dyn Codec>>;
    
    /// Create a new codec instance with specific parameters
    fn create_with_params(&self, params: &[u8]) -> Result<Box<dyn Codec>>;
} 