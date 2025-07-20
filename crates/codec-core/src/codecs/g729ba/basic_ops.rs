//! ITU-T G.729BA Basic Operations
//!
//! This module re-exports the basic mathematical operations from G.729A,
//! since G.729BA uses the same fixed-point arithmetic operations.

// Re-export all basic operations from G.729A
pub use crate::codecs::g729a::basic_ops::*; 