// Session implementation split into focused modules

mod core;
mod state;
mod media;
mod transfer;

// Re-export the main types and implementations
pub use core::{Session, SessionMediaState}; 