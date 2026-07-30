//! Retained DTLS-SRTP key-material types
//!
//! These low-level helpers do not make the unavailable DTLS connection stack
//! complete or negotiable.

pub mod extractor;

// Re-export SRTP context
pub use extractor::DtlsSrtpContext;
