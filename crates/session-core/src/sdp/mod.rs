//! SDP Negotiation Module for session-core
//! 
//! Handles SDP offer/answer negotiation between two transaction users (TUs)
//! based on their media preferences and capabilities.

mod negotiator;
mod types;

pub use negotiator::SdpNegotiator;
pub use types::{NegotiatedMediaConfig, SdpRole};

// Re-export from media module for convenience
pub use crate::media::config::{MediaConfigConverter, NegotiatedConfig}; 