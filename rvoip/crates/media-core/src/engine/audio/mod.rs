//! Audio engine components for capture, playback, and mixing
//!
//! This module provides the components for interfacing with audio hardware,
//! capturing audio from microphones, playing audio through speakers, and
//! mixing multiple audio streams.

// Device abstraction for audio hardware access
pub mod device;

// Audio capture functionality
pub mod capture;

// Audio playback functionality
pub mod playback;

// Audio mixing for combining streams
pub mod mixer;

// Re-export key types
pub use device::{AudioDevice, AudioDeviceInfo, AudioDeviceManager};
pub use capture::{AudioCapture, AudioCaptureConfig, AudioCaptureEvent};
pub use playback::{AudioPlayback, AudioPlaybackConfig, AudioPlaybackEvent};
pub use mixer::{AudioMixer, MixerConfig, MixerStream}; 