//! Audio Processing Components
//!
//! This module contains audio processing components including voice activity detection,
//! echo cancellation, automatic gain control, and noise suppression.

pub mod processor;
pub mod vad;

// Future components (to be implemented in Phase 3)
// pub mod aec;
// pub mod agc; 
// pub mod ns;
// pub mod plc;
// pub mod dtmf_detector;

// Re-export main types
pub use processor::{AudioProcessor, AudioProcessingConfig, AudioProcessingResult};
pub use vad::{VoiceActivityDetector, VadConfig, VadResult}; 