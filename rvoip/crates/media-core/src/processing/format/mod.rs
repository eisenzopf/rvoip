//! Format Conversion Components
//!
//! This module handles audio format conversion including sample rate conversion,
//! channel mixing, and bit depth conversion.

pub mod converter;
pub mod resampler;
pub mod channel_mixer;

// Re-export main types
pub use converter::{FormatConverter, ConversionParams, ConversionResult};
pub use resampler::{Resampler, ResamplerConfig};
pub use channel_mixer::{ChannelMixer, ChannelLayout}; 