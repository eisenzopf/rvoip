//! DTLS-SRTP integration
//!
//! This module provides SRTP key extraction from DTLS handshakes.

pub mod extractor;

// Re-export SRTP context
pub use extractor::DtlsSrtpContext; 