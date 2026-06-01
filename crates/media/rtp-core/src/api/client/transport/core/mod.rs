//! Core transport components
//!
//! This module contains core functionality for the media transport client:
//! - Connection management
//! - Frame handling
//! - Event subscription and notification

// Re-export modules
pub mod connection;
pub mod events;
pub mod frame;

// Re-export important types and functions
pub use connection::{connect, disconnect, get_local_address, is_connected};
pub use events::{
    register_connect_callback, register_disconnect_callback, register_event_callback,
};
pub use frame::{process_packet, receive_frame, send_frame};
