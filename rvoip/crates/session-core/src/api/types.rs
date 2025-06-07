//! Core API Types
//!
//! Defines the main types that developers interact with when using the session API.

use std::sync::Arc;
use std::time::Instant;
use serde::{Serialize, Deserialize};
use crate::errors::Result;

/// Unique identifier for a session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        // Generate a unique session ID
        let id = format!("sess_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents an active call session
#[derive(Debug, Clone)]
pub struct CallSession {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub started_at: Option<Instant>,
}

impl CallSession {
    /// Get the session ID
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Get the current call state
    pub fn state(&self) -> &CallState {
        &self.state
    }

    /// Check if the call is active (connected)
    pub fn is_active(&self) -> bool {
        matches!(self.state, CallState::Active)
    }

    /// Wait for the call to be answered
    pub async fn wait_for_answer(&self) -> Result<()> {
        // TODO: Implement waiting for answer
        todo!("wait_for_answer implementation")
    }

    /// Hold the call
    /// Note: Use SessionManager::hold_session() method instead
    pub async fn hold(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::hold_session() method instead".to_string()
        ))
    }

    /// Resume the call from hold
    /// Note: Use SessionManager::resume_session() method instead
    pub async fn resume(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::resume_session() method instead".to_string()
        ))
    }

    /// Transfer the call to another destination
    /// Note: Use SessionManager::transfer_session() method instead
    pub async fn transfer(&self, target: &str) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::transfer_session() method instead".to_string()
        ))
    }

    /// Terminate the call
    /// Note: Use SessionManager::terminate_session() method instead
    pub async fn terminate(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::terminate_session() method instead".to_string()
        ))
    }
}

/// Represents an incoming call that needs to be handled
#[derive(Debug, Clone)]
pub struct IncomingCall {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub sdp: Option<String>,
    pub headers: std::collections::HashMap<String, String>,
    pub received_at: Instant,
}

impl IncomingCall {
    /// Accept the incoming call
    /// Note: Use accept_call() function with SessionManager parameter instead
    pub async fn accept(&self) -> Result<CallSession> {
        Err(crate::errors::SessionError::Other(
            "Use accept_call(session_manager, &session_id) function instead".to_string()
        ))
    }

    /// Reject the incoming call with a reason
    /// Note: Use reject_call() function with SessionManager parameter instead
    pub async fn reject(&self, reason: &str) -> Result<()> {
        Err(crate::errors::SessionError::Other(
            "Use reject_call(session_manager, &session_id, reason) function instead".to_string()
        ))
    }

    /// Get caller information
    pub fn caller(&self) -> &str {
        &self.from
    }

    /// Get called party information
    pub fn called(&self) -> &str {
        &self.to
    }
}

/// Current state of a call session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated
    Initiating,
    /// Call is ringing (180 Ringing received)
    Ringing,
    /// Call is active and media is flowing
    Active,
    /// Call is on hold
    OnHold,
    /// Call is being transferred
    Transferring,
    /// Call is being terminated
    Terminating,
    /// Call has ended
    Terminated,
    /// Call was cancelled (487 Request Terminated)
    Cancelled,
    /// Call failed or was rejected
    Failed(String),
}

impl CallState {
    /// Check if this is a final state (call is over)
    pub fn is_final(&self) -> bool {
        matches!(self, CallState::Terminated | CallState::Cancelled | CallState::Failed(_))
    }

    /// Check if the call is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(self, CallState::Initiating | CallState::Ringing | CallState::Active | CallState::OnHold)
    }
}

/// Decision on how to handle an incoming call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDecision {
    /// Accept the call immediately
    Accept,
    /// Reject the call with a reason
    Reject(String),
    /// Defer the decision (e.g., add to queue)
    Defer,
    /// Forward the call to another destination
    Forward(String),
}

/// Statistics about active sessions
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub failed_sessions: usize,
    pub average_duration: Option<std::time::Duration>,
}

/// Media information for a session
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub codec: Option<String>,
} 