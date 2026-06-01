//! Audio Processing Components
//!
//! This module contains audio processing components including voice activity detection,
//! echo cancellation, automatic gain control, and noise suppression.

pub mod aec; // New AEC implementation
pub mod agc; // New AGC implementation
pub mod mixer;
pub mod processor;
pub mod stream; // New conference audio stream management
pub mod vad; // New audio mixer for conference calls

// Advanced v2 implementations with cutting-edge features
pub mod aec_v2;
pub mod agc_v2; // Multi-band AGC with look-ahead
pub mod vad_v2; // Advanced VAD with spectral features // Frequency-domain AEC with NLMS

// Future components (to be implemented in Phase 3)
// pub mod ns;
// pub mod plc;
// pub mod dtmf_detector;

// Re-export main types
pub use aec::{AcousticEchoCanceller, AecConfig, AecResult};
pub use agc::{AgcConfig, AgcResult, AutomaticGainControl};
pub use mixer::AudioMixer;
pub use processor::{AudioProcessingConfig, AudioProcessingResult, AudioProcessor};
pub use stream::{AudioStreamConfig, AudioStreamManager};
pub use vad::{VadConfig, VadResult, VoiceActivityDetector};

// Re-export advanced v2 types
pub use aec_v2::{AdvancedAcousticEchoCanceller, AdvancedAecConfig, AdvancedAecResult};
pub use agc_v2::{AdvancedAgcConfig, AdvancedAgcResult, AdvancedAutomaticGainControl};
pub use vad_v2::{
    AdvancedVadConfig, AdvancedVadResult, AdvancedVoiceActivityDetector, DetectorScores,
};
