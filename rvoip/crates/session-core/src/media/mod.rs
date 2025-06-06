//! Session-Core Media Integration
//!
//! This module provides comprehensive media integration for session-core,
//! coordinating between SIP signaling and media-core processing.
//!
//! Architecture:
//! - `MediaManager`: Main interface for media operations (adapted from src-old)
//! - `SessionMediaCoordinator`: Automatic media lifecycle management (adapted from src-old)
//! - `MediaConfigConverter`: SDP ↔ media-core configuration conversion (adapted from src-old)
//! - `MediaBridge`: Event integration between media and session systems (new)
//! - `types`: Modern type definitions adapted to new architecture (new)

pub mod manager;
pub mod coordinator;
pub mod config;
pub mod bridge;
pub mod types;

// Re-exports for convenience
pub use manager::MediaManager;
pub use coordinator::SessionMediaCoordinator;
pub use config::MediaConfigConverter;
pub use bridge::MediaBridge;
pub use types::*;

/// Media integration result type
pub type MediaResult<T> = Result<T, MediaError>;

/// Media integration errors
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("Media session not found: {session_id}")]
    SessionNotFound { session_id: String },
    
    #[error("Media configuration error: {message}")]
    Configuration { message: String },
    
    #[error("SDP processing error: {message}")]
    SdpProcessing { message: String },
    
    #[error("Codec negotiation failed: {reason}")]
    CodecNegotiation { reason: String },
    
    #[error("Media engine error: {source}")]
    MediaEngine { 
        #[from]
        source: Box<dyn std::error::Error + Send + Sync> 
    },
    
    #[error("Session coordination error: {message}")]
    Coordination { message: String },
}

impl From<MediaError> for crate::errors::SessionError {
    fn from(err: MediaError) -> Self {
        crate::errors::SessionError::MediaIntegration { 
            message: err.to_string() 
        }
    }
} 