//! Session Manager Module
//!
//! Contains the core SessionManager implementation broken into focused modules.

pub mod core;
pub mod registry;
pub mod events;
pub mod cleanup;
pub mod coordinator;

// Re-export the main SessionManager and coordinator
pub use core::SessionManager;
pub use coordinator::SessionCoordinator; 