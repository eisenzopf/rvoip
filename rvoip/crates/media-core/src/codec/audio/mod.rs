//! Audio codec implementations for the media-core library
//!
//! This module contains implementations of various audio codecs used in VoIP applications,
//! including G.711 (PCMU/PCMA), G.722, Opus, and others.

// Common types and utilities for audio codecs
mod common;
pub use common::*;

// G.711 µ-law codec (PCMU)
#[cfg(feature = "pcmu")]
pub mod pcmu;

// G.711 A-law codec (PCMA)
#[cfg(feature = "pcma")]
pub mod pcma;

// G.722 wideband codec
#[cfg(feature = "g722")]
pub mod g722;

// Opus codec (wide-band and full-band)
#[cfg(feature = "opus")]
pub mod opus;

// Re-export common audio codec types
pub use crate::codec::AudioCodec;

// Utility to convert PCM samples between formats
pub mod converter;

/// Payload type constants for static audio codecs
pub mod payload_type {
    /// PCMU/G.711 µ-law (8kHz)
    pub const PCMU: u8 = 0;
    
    /// PCMA/G.711 A-law (8kHz)
    pub const PCMA: u8 = 8;
    
    /// G.722 (16kHz)
    pub const G722: u8 = 9;
    
    /// Telephone-event (DTMF) RFC 4733
    pub const TELEPHONE_EVENT: u8 = 101;
} 