//! Media session management
//!
//! This module provides the core abstractions for managing media sessions,
//! which coordinate the flow of media between endpoints.

// Media session implementation
pub mod media_session;

// Session configuration
pub mod config;

// Session events
pub mod events;

// Media flow control
pub mod flow;

// Re-export key types
pub use media_session::{MediaSession, MediaSessionId};
pub use config::{MediaSessionConfig, MediaType};
pub use events::{MediaSessionEvent, MediaSessionEventKind};
pub use flow::{MediaFlow, MediaState};

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use uuid::Uuid;

use crate::codec::Codec;
use crate::error::{Error, Result};

/// Media direction for a media stream
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDirection {
    /// Send and receive media
    SendRecv,
    /// Send media only
    SendOnly,
    /// Receive media only
    RecvOnly,
    /// No media (inactive)
    Inactive,
}

impl Default for MediaDirection {
    fn default() -> Self {
        Self::SendRecv
    }
}

/// Media type for a session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio media
    Audio,
    /// Video media
    Video,
    /// Application data
    Application,
}

/// Media state for a session
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaState {
    /// Session is being created
    Creating,
    /// Session is ready but not active
    Ready,
    /// Session is active and media is flowing
    Active,
    /// Session is on hold
    Held,
    /// Session is being terminated
    Terminating,
    /// Session has been terminated
    Terminated,
}

/// Unique identifier for a media session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MediaSessionId(Uuid);

impl MediaSessionId {
    /// Create a new random session ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    
    /// Get the underlying UUID
    pub fn uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for MediaSessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MediaSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Session statistics
#[derive(Debug, Clone, Default)]
pub struct MediaSessionStats {
    /// Total packets sent
    pub packets_sent: u64,
    /// Total packets received
    pub packets_received: u64,
    /// Total bytes sent
    pub bytes_sent: u64,
    /// Total bytes received
    pub bytes_received: u64,
    /// Packets lost
    pub packets_lost: u64,
    /// Current jitter in milliseconds
    pub jitter_ms: f64,
    /// Round-trip time in milliseconds
    pub rtt_ms: f64,
    /// Mean Opinion Score (estimated audio quality, 1.0-5.0)
    pub mos: f32,
} 