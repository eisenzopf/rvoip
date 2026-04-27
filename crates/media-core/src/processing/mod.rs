//! Media Processing Pipeline
//!
//! This module contains all media processing capabilities including audio processing,
//! format conversion, and processing pipeline orchestration.

pub mod audio;
pub mod format;
pub mod pipeline;

// Re-export main processing types
pub use audio::{
    AcousticEchoCanceller, AecConfig, AecResult, AgcConfig, AgcResult, AudioMixer,
    AudioProcessingConfig, AudioProcessingResult, AudioProcessor, AudioStreamConfig,
    AudioStreamManager, AutomaticGainControl, VadConfig, VadResult, VoiceActivityDetector,
};
pub use format::{ConversionParams, FormatConverter};
pub use pipeline::{ProcessingConfig, ProcessingPipeline};
