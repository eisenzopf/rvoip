//! Media buffer management module for the media-core library
//!
//! This module provides buffer management for handling media packets,
//! including jitter buffers and adaptive buffer sizing for network optimization.

// Jitter buffer for handling network timing variances
pub mod jitter;
pub use jitter::{JitterBuffer, JitterBufferConfig, JitterBufferStats, JitterBufferMode};

// Adaptive buffer sizing for network conditions
pub mod adaptive;
pub use adaptive::{AdaptiveBuffer, AdaptiveBufferConfig};

// Re-export common buffer types
pub mod common;
pub use common::*; 