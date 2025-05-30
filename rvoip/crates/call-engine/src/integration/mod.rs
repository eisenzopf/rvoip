//! Session-core integration module
//!
//! This module provides adapters and integration components for
//! interfacing with session-core functionality.

pub mod session;
pub mod bridge;
pub mod events;

pub use session::SessionAdapter;
pub use bridge::BridgeAdapter;
pub use events::EventAdapter; 