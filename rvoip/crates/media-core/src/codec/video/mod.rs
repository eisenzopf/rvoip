//! Video codec implementations
//!
//! This module provides implementations of video codecs such as H.264 and VP8.

// Video codec interfaces
mod common;

// Specific codec implementations
pub mod h264;
pub mod vp8;

// Re-export common types
pub use common::*; 