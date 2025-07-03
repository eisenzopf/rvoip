//! SSRC-related functionality
//!
//! This module handles SSRC-related functions including demultiplexing.

pub mod demux;

// Re-export the key functions for easier access
pub use demux::*; 