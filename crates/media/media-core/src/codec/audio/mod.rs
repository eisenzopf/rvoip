//! Audio codec types and utilities

// Common types and utilities for audio codecs
pub mod common;
pub mod dtmf;
pub mod g711; // G.711 codec implementation
pub mod g729; // Add G.729 codec
pub mod opus; // Opus codec implementation // RFC 4733 DTMF telephone-event codec

pub use common::*;

/// Payload type constants for static audio codecs
pub mod payload_type {
    /// PCMU/G.711 µ-law (8kHz)
    pub const PCMU: u8 = 0;

    /// PCMA/G.711 A-law (8kHz)
    pub const PCMA: u8 = 8;

    /// G.722 (16kHz)
    pub const G722: u8 = 9;

    /// Comfort Noise — RFC 3389. Wire payload is one byte of noise
    /// level in -dBov (RFC 3389 §3.1) plus optional spectral info.
    /// The standard rate is 8 kHz to pair with PCMU/PCMA.
    pub const COMFORT_NOISE: u8 = 13;

    /// G.729 (8kHz, 8kbps)
    pub const G729: u8 = 18;

    /// Telephone-event (DTMF) RFC 4733
    pub const TELEPHONE_EVENT: u8 = 101;
}

// Re-export G.711 codec types
pub use g711::G711Codec;

// Re-export Opus codec types
pub use opus::{OpusApplication, OpusCodec, OpusConfig};

// Re-export G.729 codec types
pub use g729::{G729Annexes, G729Codec, G729Config};

// Re-export DTMF codec types
pub use dtmf::{DtmfEvent, TelephoneEvent};
