//! Traits for RTP payload format handlers
//!
//! This module defines the interfaces that payload format handlers must implement
//! to work with the RTP system.

use bytes::Bytes;
use std::any::Any;

/// Trait for RTP payload format handlers
///
/// This trait defines the interface for payload format handlers that can
/// pack and unpack media data for transmission over RTP.
pub trait PayloadFormat: Send + Sync {
    /// Get the payload type identifier
    fn payload_type(&self) -> u8;
    
    /// Get the clock rate for this payload format
    fn clock_rate(&self) -> u32;
    
    /// Get the number of channels
    fn channels(&self) -> u8;
    
    /// Get the preferred packet duration in milliseconds
    fn preferred_packet_duration(&self) -> u32;
    
    /// Calculate samples from duration
    ///
    /// Given a duration in milliseconds, calculate how many samples that represents
    fn samples_from_duration(&self, duration_ms: u32) -> u32 {
        (self.clock_rate() * duration_ms) / 1000
    }
    
    /// Calculate duration from samples
    ///
    /// Given a number of samples, calculate how many milliseconds that represents
    fn duration_from_samples(&self, samples: u32) -> u32 {
        (samples * 1000) / self.clock_rate()
    }
    
    /// Calculate packet size from duration
    ///
    /// Given a duration in milliseconds, calculate the expected packet size in bytes
    fn packet_size_from_duration(&self, duration_ms: u32) -> usize;
    
    /// Calculate duration from packet size
    ///
    /// Given a packet size in bytes, calculate the expected duration in milliseconds
    fn duration_from_packet_size(&self, packet_size: usize) -> u32;
    
    /// Pack media data into RTP payload
    ///
    /// This method takes raw media data (e.g. PCM samples) and encodes it into
    /// the format expected for the RTP payload.
    fn pack(&self, media_data: &[u8], timestamp: u32) -> Bytes;
    
    /// Unpack RTP payload into media data
    ///
    /// This method takes an RTP payload and decodes it into raw media data
    /// (e.g. PCM samples).
    fn unpack(&self, payload: &[u8], timestamp: u32) -> Bytes;
    
    /// Check if this payload format can handle the given payload type
    fn can_handle(&self, payload_type: u8) -> bool {
        self.payload_type() == payload_type
    }
    
    /// Get this handler as an Any for downcasting
    fn as_any(&self) -> &dyn Any;
}

/// Factory trait for creating payload format handlers
pub trait PayloadFormatFactory: Send + Sync {
    /// Create a payload format handler for the given payload type
    fn create_format(&self, payload_type: u8, clock_rate: u32) -> Option<Box<dyn PayloadFormat>>;
    
    /// Check if this factory can create a handler for the given payload type
    fn can_handle(&self, payload_type: u8) -> bool;
} 