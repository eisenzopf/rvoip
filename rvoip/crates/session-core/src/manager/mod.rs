//! Session Manager Module
//!
//! Contains the core SessionManager implementation broken into focused modules.

pub mod core;
pub mod registry;
pub mod events;
pub mod cleanup;

// Re-export the main SessionManager
pub use core::SessionManager; 