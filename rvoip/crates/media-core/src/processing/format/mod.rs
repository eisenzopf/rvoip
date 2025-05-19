//! Format conversion module
//!
//! This module provides utilities for converting between different audio formats,
//! including sample rate conversion and channel conversion.

// Re-export components
pub mod resampler;
pub mod channels;

// Re-export key types
pub use resampler::Resampler;
pub use channels::ChannelConverter; 