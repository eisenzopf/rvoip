//! Media Session Management
//!
//! This module provides MediaSession implementation for per-dialog media management,
//! including codec lifecycle, quality tracking, and media processing coordination.

pub mod events;
pub mod media_session;

// Re-export main types
pub use events::{MediaSessionEvent, MediaSessionEventType};
pub use media_session::{MediaSession, MediaSessionConfig, MediaSessionState};
