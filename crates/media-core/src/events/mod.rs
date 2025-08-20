//! Media Core Event System
//!
//! This module provides event handling and cross-crate communication for media-core.

pub mod adapter;

// Re-export main event adapter
pub use adapter::MediaEventAdapter;