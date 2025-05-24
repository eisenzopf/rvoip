//! Audio codec types and utilities

// Common types and utilities for audio codecs
pub mod common;
pub mod g711;  // G.711 codec implementation
pub mod opus;  // Opus codec implementation
pub mod g729;  // Add G.729 codec

pub use common::*;

/// Payload type constants for static audio codecs
pub mod payload_type {
    /// PCMU/G.711 µ-law (8kHz)
    pub const PCMU: u8 = 0;
    
    /// PCMA/G.711 A-law (8kHz)
    pub const PCMA: u8 = 8;
    
    /// G.722 (16kHz)
    pub const G722: u8 = 9;
    
    /// G.729 (8kHz, 8kbps)
    pub const G729: u8 = 18;
    
    /// Telephone-event (DTMF) RFC 4733
    pub const TELEPHONE_EVENT: u8 = 101;
}

// Re-export G.711 codec types
pub use g711::{G711Codec, G711Config, G711Variant};

// Re-export Opus codec types
pub use opus::{OpusCodec, OpusConfig, OpusApplication};

// Re-export G.729 codec types
pub use g729::{G729Codec, G729Config, G729Annexes}; 