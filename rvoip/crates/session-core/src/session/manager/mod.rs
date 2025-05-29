// Session Manager module - refactored into focused components

mod core;
mod lifecycle;
mod media;
mod transfer;
mod bridge_api;

// Re-export the main SessionManager type
pub use core::SessionManager; 