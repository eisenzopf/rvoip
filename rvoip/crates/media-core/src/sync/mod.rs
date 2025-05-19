//! Media synchronization module for the media-core library
//!
//! This module provides utilities for managing media timing,
//! synchronizing audio and video streams, and managing media clocks.

// Media clock implementation
pub mod clock;

// A/V synchronization
pub mod lipsync;

// Re-export key types
pub use clock::{MediaClock, ClockSource, MediaTimestamp};
pub use lipsync::{LipSync, LipSyncConfig, MediaSyncPoint}; 