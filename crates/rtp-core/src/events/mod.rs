//! RTP Core Event System
//!
//! This module provides event handling and cross-crate communication for rtp-core.

pub mod adapter;

// Re-export main event adapter
pub use adapter::RtpEventAdapter;