//! Video Engine module
//!
//! This module provides the core video processing engine capabilities
//! including capture, rendering, and device management.

// Video device management
pub mod device;

// Video capture (camera)
pub mod capture;

// Video rendering
pub mod render;

// Re-export key components
pub use device::{VideoDevice, VideoDeviceManager, VideoDeviceInfo};
pub use capture::{VideoCapture, VideoCaptureConfig, VideoSource, VideoFrame};
pub use render::{VideoRenderer, VideoRenderConfig, VideoSink}; 