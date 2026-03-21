//! Session Core v2 API (merged into session-core v1)

pub mod types;
pub mod builder;

// Re-export the main types
pub use types::{
    SessionId, CallSession, IncomingCall, CallDecision,
    SessionStats, MediaInfo, AudioStreamConfig,
    parse_sdp_connection, SdpInfo,
};
pub use crate::state_table::CallState;

// Re-export builder
pub use builder::SessionBuilder;

// Re-export from state table for consistency
pub use crate::state_table::types::{Role, EventType};

// Error types
pub use crate::errors_v2::{Result, SessionError};
