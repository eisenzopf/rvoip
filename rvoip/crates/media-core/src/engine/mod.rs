//! Media processing engines for audio and video capture/playback
//!
//! This module provides components for managing media device access,
//! capture pipelines, playback systems, and mixing for both audio and video.

pub mod audio;
pub mod video;

// Re-export key components
pub use audio::{AudioDevice, AudioEngine};
pub use video::{VideoDevice, VideoRenderer}; 