//! Audio Processing Components
//!
//! This module contains audio processing components including voice activity detection,
//! echo cancellation, automatic gain control, and noise suppression.

pub mod processor;
pub mod vad;
pub mod agc;  // New AGC implementation
pub mod aec;  // New AEC implementation
pub mod stream;  // New conference audio stream management
pub mod mixer;  // New audio mixer for conference calls

// Advanced v2 implementations with cutting-edge features
pub mod vad_v2;  // Advanced VAD with spectral features
pub mod agc_v2;  // Multi-band AGC with look-ahead
pub mod aec_v2;  // Frequency-domain AEC with NLMS

// Future components (to be implemented in Phase 3)
// pub mod ns;
// pub mod plc;
// pub mod dtmf_detector;

// Re-export main types
pub use processor::{AudioProcessor, AudioProcessingConfig, AudioProcessingResult};
pub use vad::{VoiceActivityDetector, VadConfig, VadResult};
pub use agc::{AutomaticGainControl, AgcConfig, AgcResult};
pub use aec::{AcousticEchoCanceller, AecConfig, AecResult};
pub use stream::{AudioStreamManager, AudioStreamConfig};
pub use mixer::{AudioMixer};

// Re-export advanced v2 types
pub use vad_v2::{AdvancedVoiceActivityDetector, AdvancedVadConfig, AdvancedVadResult, DetectorScores};
pub use agc_v2::{AdvancedAutomaticGainControl, AdvancedAgcConfig, AdvancedAgcResult};
pub use aec_v2::{AdvancedAcousticEchoCanceller, AdvancedAecConfig, AdvancedAecResult};
