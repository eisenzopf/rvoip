//! Public API for session-core
//!
//! This module provides the main public interface for the session-core crate.

pub mod types;
pub mod handlers;
pub mod builder;
pub mod control;
pub mod media;
pub mod create;
pub mod examples;

// Re-export main types
pub use types::{
    SessionId, CallSession, CallState, IncomingCall, CallDecision, 
    SessionStats, MediaInfo, PreparedCall, CallDirection, TerminationReason,
    SdpInfo, parse_sdp_connection,
};
pub use handlers::CallHandler;
pub use builder::{SessionManagerBuilder, SessionManagerConfig};
pub use control::SessionControl;
pub use media::MediaControl;

// Re-export error types
pub use crate::errors::{Result, SessionError};

// Re-export the SessionCoordinator as the main entry point
pub use crate::coordinator::SessionCoordinator; 