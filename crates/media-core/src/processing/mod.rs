//! Media Processing Pipeline
//!
//! This module contains all media processing capabilities including audio processing,
//! format conversion, and processing pipeline orchestration.

pub mod pipeline;
pub mod audio;
pub mod format;

// Re-export main processing types
pub use pipeline::{ProcessingPipeline, ProcessingConfig};
pub use audio::{
    AudioProcessor, AudioProcessingConfig, AudioProcessingResult,
    VoiceActivityDetector, VadConfig, VadResult,
    AutomaticGainControl, AgcConfig, AgcResult,
    AcousticEchoCanceller, AecConfig, AecResult,
    AudioStreamManager, AudioStreamConfig,
    AudioMixer,
};
pub use format::{FormatConverter, ConversionParams}; 