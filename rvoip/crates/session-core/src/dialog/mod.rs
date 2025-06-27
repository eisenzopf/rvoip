//! Session-Core Dialog Integration
//!
//! This module provides comprehensive dialog integration for session-core,
//! coordinating between session management and dialog-core SIP operations.
//!
//! Architecture (parallel to media/ module):
//! - `DialogManager`: Main interface for dialog operations (parallel to MediaManager)
//! - `SessionDialogCoordinator`: Automatic dialog lifecycle management (parallel to SessionMediaCoordinator)
//! - `DialogConfigConverter`: SIP ↔ session configuration conversion (parallel to MediaConfigConverter)
//! - `DialogBridge`: Event integration between dialog and session systems (parallel to MediaBridge)
//! - `types`: Dialog type definitions (parallel to MediaEngine/types)
//! - `builder`: Dialog setup and creation (unique to dialog - media doesn't need this)

pub mod manager;
pub mod coordinator;
pub mod config;
pub mod bridge;
pub mod types;
pub mod builder;

// Re-exports for convenience
pub use manager::DialogManager;
pub use coordinator::SessionDialogCoordinator;
pub use config::DialogConfigConverter;
pub use bridge::DialogBridge;
pub use types::*;
pub use builder::DialogBuilder;

/// Dialog integration result type
pub type DialogResult<T> = Result<T, DialogError>;

/// Dialog integration errors (parallel to MediaError)
#[derive(Debug, thiserror::Error)]
pub enum DialogError {
    #[error("Dialog session not found: {session_id}")]
    SessionNotFound { session_id: String },
    
    #[error("Dialog configuration error: {message}")]
    Configuration { message: String },
    
    #[error("SIP processing error: {message}")]
    SipProcessing { message: String },
    
    #[error("Dialog creation failed: {reason}")]
    DialogCreation { reason: String },
    
    #[error("Dialog-core error: {source}")]
    DialogCore { 
        #[from]
        source: Box<dyn std::error::Error + Send + Sync> 
    },
    
    #[error("Session coordination error: {message}")]
    Coordination { message: String },
}

impl From<DialogError> for crate::errors::SessionError {
    fn from(err: DialogError) -> Self {
        crate::errors::SessionError::DialogIntegration { 
            message: err.to_string() 
        }
    }
} 