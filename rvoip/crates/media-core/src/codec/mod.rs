//! Audio codec implementations
//!
//! This module contains various audio codec implementations for encoding and decoding
//! audio data in different formats.

use bytes::Bytes;
use crate::{Result, AudioBuffer, AudioFormat};

/// G.711 codec implementation (μ-law and A-law)
pub mod g711;
pub use g711::{G711Codec, G711Variant};

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