//! Media Core Event System
//!
//! This module provides event handling and cross-crate communication for media-core.

pub mod adapter;
pub mod event_hub;

// Re-export main event types
pub use adapter::MediaEventAdapter;
pub use event_hub::MediaEventHub;