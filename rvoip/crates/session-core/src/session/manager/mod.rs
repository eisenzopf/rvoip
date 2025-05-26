// Session Manager module - refactored into focused components

mod core;
mod lifecycle;
mod media;
mod transfer;

// Re-export the main SessionManager type
pub use core::SessionManager; 