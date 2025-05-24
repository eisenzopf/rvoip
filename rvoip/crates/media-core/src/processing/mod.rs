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
    aec::AcousticEchoCanceller,
    agc::AutomaticGainControl,
    vad::VoiceActivityDetector,
    ns::NoiseSuppressor,
    plc::PacketLossConcealer,
    dtmf::DtmfDetector,
};

// Alias the long names for convenience
pub use audio::aec::AcousticEchoCanceller as EchoCanceller;
pub use audio::agc::AutomaticGainControl as GainControl;
pub use audio::ns::NoiseSuppressor as NoiseSupressor; // Note: keeping the typo for compatibility
pub use audio::plc::PacketLossConcealer as PacketLossConcealor; // Note: keeping the typo for compatibility

/// A pipeline of audio processing components
pub mod pipeline;
pub use pipeline::*; 