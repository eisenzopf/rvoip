//! Core transport components
//!
//! This module contains core functionality for the media transport client:
//! - Connection management
//! - Frame handling
//! - Event subscription and notification

// Re-export modules
pub mod connection;
pub mod frame;
pub mod events;

// Re-export important types and functions
pub use connection::{connect, disconnect, get_local_address, is_connected};
pub use frame::{send_frame, receive_frame, process_packet};
pub use events::{register_event_callback, register_connect_callback, register_disconnect_callback}; 