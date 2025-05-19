//! Media signal processing module for the media-core library
//!
//! This module contains implementations of various audio and video processing
//! algorithms used for enhancing call quality, such as echo cancellation,
//! noise suppression, and voice activity detection.

// Audio processing components
pub mod audio;

// Format conversion (resampling, etc.)
pub mod format;

// Re-export commonly used types
pub use audio::{
    aec::EchoCanceller,
    agc::GainControl,
    vad::VoiceActivityDetector,
    ns::NoiseSupressor,
    plc::PacketLossConcealor,
    dtmf::DtmfDetector,
};

/// A pipeline of audio processing components
pub mod pipeline;
pub use pipeline::*; 