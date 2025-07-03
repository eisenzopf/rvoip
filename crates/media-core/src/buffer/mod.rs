//! Media Buffering
//!
//! This module provides buffering solutions for media processing, including
//! adaptive jitter buffering for smooth audio playback and packet reordering.

pub mod jitter;
pub mod adaptive;
pub mod frame_buffer;
pub mod ring_buffer;

// Re-export main types
pub use jitter::{JitterBuffer, JitterBufferConfig, JitterBufferStats};
pub use adaptive::{AdaptiveBuffer, AdaptiveConfig};
pub use frame_buffer::{FrameBuffer, FrameBufferConfig};
pub use ring_buffer::{RingBuffer, RingBufferError}; 