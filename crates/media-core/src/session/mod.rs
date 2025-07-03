//! Media Session Management
//!
//! This module provides MediaSession implementation for per-dialog media management,
//! including codec lifecycle, quality tracking, and media processing coordination.

pub mod media_session;
pub mod events;

// Re-export main types
pub use media_session::{MediaSession, MediaSessionConfig, MediaSessionState};
pub use events::{MediaSessionEvent, MediaSessionEventType}; 