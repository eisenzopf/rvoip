//! Audio codec types and utilities

// Common types and utilities for audio codecs
pub mod common;
pub use common::*;

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