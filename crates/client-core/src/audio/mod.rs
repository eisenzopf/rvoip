//! Audio Device Abstraction Layer
//!
//! This module provides cross-platform audio device access for VoIP applications.
//! It integrates with the session-core audio streaming API to provide complete
//! audio input/output functionality.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────┐    ┌─────────────────────┐    ┌─────────────────────┐
//! │   ClientManager     │    │  AudioDeviceManager │    │   Platform Audio    │
//! │                     │    │                     │    │                     │
//! │ start_audio_xxx()   │───▶│ AudioDevice trait   │───▶│ cpal / mock / etc.  │
//! │ stop_audio_xxx()    │    │ Device enumeration  │    │ Format conversion   │
//! │                     │    │ Format conversion   │    │                     │
//! └─────────────────────┘    └─────────────────────┘    └─────────────────────┘
//!           │                          │                          │
//!           │                          │                          │
//!           ▼                          ▼                          ▼
//! ┌─────────────────────┐    ┌─────────────────────┐    ┌─────────────────────┐
//! │   Session-Core      │    │   Audio Callbacks   │    │   Audio Hardware    │
//! │                     │    │                     │    │                     │
//! │ AudioFrame API      │◄───│ Frame conversion    │◄───│ Sample rate conv.   │
//! │ MediaControl trait  │    │ Channel bridging    │    │ Format adaptation   │
//! │                     │    │                     │    │                     │
//! └─────────────────────┘    └─────────────────────┘    └─────────────────────┘
//! ```
//!
//! # Quick Start
//!
//! ## Audio Playback (Incoming Call Audio)
//!
//! ```rust,no_run
//! use rvoip_client_core::audio::{AudioDeviceManager, AudioDirection};
//! use rvoip_client_core::CallId;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let device_manager = AudioDeviceManager::new().await?;
//!
//! // List available playback devices
//! let devices = device_manager.list_devices(AudioDirection::Output).await?;
//! println!("Available speakers: {:#?}", devices);
//!
//! // Use default speaker
//! let speaker = device_manager.get_default_device(AudioDirection::Output).await?;
//!
//! // Start playback for a call (frames come from session-core)
//! let call_id = CallId::new_v4();
//! device_manager.start_playback(&call_id, speaker).await?;
//!
//! // Audio frames will automatically flow: RTP → session-core → speaker
//! // ...
//!
//! // Stop playback
//! device_manager.stop_playback(&call_id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Audio Capture (Outgoing Call Audio)
//!
//! ```rust,no_run
//! use rvoip_client_core::audio::{AudioDeviceManager, AudioDirection};
//! use rvoip_client_core::CallId;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let device_manager = AudioDeviceManager::new().await?;
//!
//! // Use default microphone
//! let microphone = device_manager.get_default_device(AudioDirection::Input).await?;
//!
//! // Start capture for a call (frames go to session-core)
//! let call_id = CallId::new_v4();
//! device_manager.start_capture(&call_id, microphone).await?;
//!
//! // Audio frames will automatically flow: microphone → session-core → RTP
//! // ...
//!
//! // Stop capture
//! device_manager.stop_capture(&call_id).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Platform Support
//!
//! This module supports multiple audio backends:
//!
//! - **`cpal`** - Cross-platform audio library (Windows, macOS, Linux)
//! - **`mock`** - Testing and simulation
//! - **Future**: Direct platform APIs for optimal performance
//!
//! The backend is automatically selected based on the target platform and available features.

pub mod device;
pub mod manager;
pub mod platform;

// Re-exports for convenience
pub use device::{AudioDevice, AudioDeviceInfo, AudioDirection, AudioFormat, AudioError, AudioResult};
pub use manager::{AudioDeviceManager, PlaybackSession, CaptureSession};
pub use platform::{create_platform_device, list_platform_devices, get_default_platform_device};

/// Audio module version
pub const VERSION: &str = "0.1.0";

/// Default audio configuration for VoIP
pub const DEFAULT_SAMPLE_RATE: u32 = 8000;  // 8kHz for narrowband voice
pub const DEFAULT_CHANNELS: u16 = 1;        // Mono
pub const DEFAULT_FRAME_SIZE_MS: u32 = 20;  // 20ms frames (160 samples at 8kHz)

/// Calculate samples per frame for given sample rate and frame duration
pub const fn samples_per_frame(sample_rate: u32, frame_size_ms: u32) -> usize {
    ((sample_rate * frame_size_ms) / 1000) as usize
} 