//! Format Conversion Components
//!
//! This module handles audio format conversion including sample rate conversion,
//! channel mixing, and bit depth conversion.

pub mod channel_mixer;
pub mod converter;
pub mod resampler;

// Re-export main types
pub use channel_mixer::{ChannelLayout, ChannelMixer};
pub use converter::{ConversionParams, ConversionResult, FormatConverter};
pub use resampler::{Resampler, ResamplerConfig};
