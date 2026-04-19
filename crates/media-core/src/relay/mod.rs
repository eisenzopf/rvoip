//! Media Relay Module
//!
//! Hosts the [`controller::MediaSessionController`] — the owner of per-dialog
//! RTP sessions and the [`controller::bridge`] primitive used by
//! session-core-v3 and b2bua-style consumers.
//!
//! Also defines the G.711 passthrough codec wrappers used by the codec
//! registry in `crate::codec`.

use crate::error::Result;

// Controller module for session-core integration
pub mod controller;

// Re-export controller types for convenience
pub use controller::{
    MediaSessionController,
    MediaConfig,
    MediaSessionStatus,
    MediaSessionInfo,
    MediaSessionEvent,
};
pub use crate::types::DialogId;

/// Simple G.711 PCMU codec implementation
#[derive(Debug, Clone)]
pub struct G711PcmuCodec;

impl G711PcmuCodec {
    /// Create a new PCMU codec
    pub fn new() -> Self {
        Self
    }

    /// Get the payload type (0 for PCMU)
    pub fn payload_type(&self) -> u8 {
        0
    }

    /// Get the codec name
    pub fn name(&self) -> &'static str {
        "PCMU"
    }

    /// Get the clock rate (8000 Hz for G.711)
    pub fn clock_rate(&self) -> u32 {
        8000
    }

    /// Get the number of channels (1 for mono)
    pub fn channels(&self) -> u8 {
        1
    }

    /// Process a packet (passthrough for basic relay)
    pub fn process_packet(&self, payload: &[u8]) -> Result<bytes::Bytes> {
        // For basic relay, just pass through the payload
        Ok(bytes::Bytes::copy_from_slice(payload))
    }
}

impl crate::codec::Codec for G711PcmuCodec {
    fn payload_type(&self) -> u8 {
        0
    }

    fn name(&self) -> &'static str {
        "PCMU"
    }

    fn process_payload(&self, payload: &[u8]) -> crate::Result<Vec<u8>> {
        Ok(payload.to_vec())
    }
}

/// Simple G.711 PCMA codec implementation
#[derive(Debug, Clone)]
pub struct G711PcmaCodec;

impl G711PcmaCodec {
    /// Create a new PCMA codec
    pub fn new() -> Self {
        Self
    }

    /// Get the payload type (8 for PCMA)
    pub fn payload_type(&self) -> u8 {
        8
    }

    /// Get the codec name
    pub fn name(&self) -> &'static str {
        "PCMA"
    }

    /// Get the clock rate (8000 Hz for G.711)
    pub fn clock_rate(&self) -> u32 {
        8000
    }

    /// Get the number of channels (1 for mono)
    pub fn channels(&self) -> u8 {
        1
    }

    /// Process a packet (passthrough for basic relay)
    pub fn process_packet(&self, payload: &[u8]) -> Result<bytes::Bytes> {
        // For basic relay, just pass through the payload
        Ok(bytes::Bytes::copy_from_slice(payload))
    }
}

impl crate::codec::Codec for G711PcmaCodec {
    fn payload_type(&self) -> u8 {
        8
    }

    fn name(&self) -> &'static str {
        "PCMA"
    }

    fn process_payload(&self, payload: &[u8]) -> crate::Result<Vec<u8>> {
        Ok(payload.to_vec())
    }
}
