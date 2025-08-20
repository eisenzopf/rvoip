//! SIP Transport Event System
//!
//! This module provides event handling and cross-crate communication for sip-transport.

pub mod adapter;

// Re-export main event adapter
pub use adapter::TransportEventAdapter;