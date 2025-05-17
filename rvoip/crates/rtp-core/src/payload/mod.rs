//! Payload format handlers for RTP
//!
//! This module provides implementations for various RTP payload formats
//! as defined in RFC 3551 and other RFCs.

mod traits;
mod g711;
mod g722;
mod opus;
mod vp8;
mod vp9;

pub use traits::{PayloadFormat, PayloadFormatFactory};
pub use g711::{G711UPayloadFormat, G711APayloadFormat};
pub use g722::G722PayloadFormat;
pub use opus::{OpusPayloadFormat, OpusBandwidth};
pub use vp8::Vp8PayloadFormat;
pub use vp9::Vp9PayloadFormat;

/// The different payload types as defined in RFC 3551
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PayloadType {
    /// PCMU (G.711 µ-law) - 8kHz
    PCMU = 0,
    /// GSM - 8kHz
    GSM = 3,
    /// G.723 - 8kHz
    G723 = 4,
    /// PCMA (G.711 A-law) - 8kHz
    PCMA = 8,
    /// G.722 - 8kHz
    G722 = 9,
    /// L16 stereo - 44.1kHz
    L16Stereo = 10,
    /// L16 mono - 44.1kHz
    L16Mono = 11,
    /// QCELP - 8kHz
    QCELP = 12,
    /// MPA (MPEG-1/MPEG-2 audio) - 90kHz
    MPA = 14,
    /// G.728 - 8kHz
    G728 = 15,
    /// G.729 - 8kHz
    G729 = 18,
    /// Unassigned
    Unassigned = 20,
    /// Dynamic payload type
    Dynamic(u8),
}

impl PayloadType {
    /// Convert a u8 to a PayloadType
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::PCMU,
            3 => Self::GSM,
            4 => Self::G723,
            8 => Self::PCMA,
            9 => Self::G722,
            10 => Self::L16Stereo,
            11 => Self::L16Mono,
            12 => Self::QCELP,
            14 => Self::MPA,
            15 => Self::G728,
            18 => Self::G729,
            20 => Self::Unassigned,
            96..=127 => Self::Dynamic(value),
            _ => Self::Dynamic(value),
        }
    }
    
    /// Get the default clock rate for this payload type
    pub fn default_clock_rate(&self) -> u32 {
        match self {
            Self::PCMU => 8000,
            Self::GSM => 8000,
            Self::G723 => 8000,
            Self::PCMA => 8000,
            Self::G722 => 8000,
            Self::L16Stereo => 44100,
            Self::L16Mono => 44100,
            Self::QCELP => 8000,
            Self::MPA => 90000,
            Self::G728 => 8000,
            Self::G729 => 8000,
            Self::Unassigned => 8000,
            Self::Dynamic(_) => 8000, // Default to 8kHz for dynamic types
        }
    }
    
    /// Get a human-readable name for this payload type
    pub fn name(&self) -> &'static str {
        match self {
            Self::PCMU => "PCMU (G.711 µ-law)",
            Self::GSM => "GSM",
            Self::G723 => "G.723",
            Self::PCMA => "PCMA (G.711 A-law)",
            Self::G722 => "G.722",
            Self::L16Stereo => "L16 (stereo)",
            Self::L16Mono => "L16 (mono)",
            Self::QCELP => "QCELP",
            Self::MPA => "MPA (MPEG audio)",
            Self::G728 => "G.728",
            Self::G729 => "G.729",
            Self::Unassigned => "Unassigned",
            Self::Dynamic(_) => "Dynamic",
        }
    }
    
    /// Convert to a u8
    pub fn to_u8(&self) -> u8 {
        match self {
            Self::PCMU => 0,
            Self::GSM => 3,
            Self::G723 => 4,
            Self::PCMA => 8,
            Self::G722 => 9,
            Self::L16Stereo => 10,
            Self::L16Mono => 11,
            Self::QCELP => 12,
            Self::MPA => 14,
            Self::G728 => 15,
            Self::G729 => 18,
            Self::Unassigned => 20,
            Self::Dynamic(pt) => *pt,
        }
    }
}

/// Create a payload format handler for the given payload type
pub fn create_payload_format(
    payload_type: PayloadType, 
    clock_rate: Option<u32>,
) -> Option<Box<dyn PayloadFormat>> {
    let clock_rate = clock_rate.unwrap_or_else(|| payload_type.default_clock_rate());
    
    match payload_type {
        PayloadType::PCMU => Some(Box::new(G711UPayloadFormat::new(clock_rate))),
        PayloadType::PCMA => Some(Box::new(G711APayloadFormat::new(clock_rate))),
        PayloadType::G722 => Some(Box::new(G722PayloadFormat::new(clock_rate))),
        PayloadType::Dynamic(pt) if pt >= 96 && pt <= 127 => {
            // For dynamic payload types, we need to know what codec it represents
            // This would typically come from SDP, but for now we'll use some defaults:
            match pt {
                96 => Some(Box::new(OpusPayloadFormat::new(pt, 1))),  // Assume Opus mono
                97 => Some(Box::new(OpusPayloadFormat::new(pt, 2))),  // Assume Opus stereo
                98 => Some(Box::new(Vp8PayloadFormat::new(pt))),      // Assume VP8
                99 => Some(Box::new(Vp9PayloadFormat::new(pt))),      // Assume VP9
                _ => Some(Box::new(OpusPayloadFormat::new(pt, 1)))    // Default to Opus
            }
        },
        // Add other payload formats as they are implemented
        _ => None,
    }
} 