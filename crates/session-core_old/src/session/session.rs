//! Session Implementation
//!
//! Core session handling logic with consolidated ID mappings.

use crate::api::types::{SessionId, CallState, CallSession, SessionRole};
use crate::errors::Result;

/// Internal session implementation that consolidates all related IDs and state
/// This is the single source of truth for session data, containing all mappings.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique session identifier (primary key)
    pub session_id: SessionId,
    
    /// Role of this session (UAC or UAS)
    pub role: SessionRole,
    
    /// Associated SIP dialog ID (if any)
    pub dialog_id: Option<rvoip_dialog_core::DialogId>,
    
    /// Associated media session ID (if any)  
    pub media_session_id: Option<crate::media::types::MediaSessionId>,
    
    /// Core call session data (public API compatible)
    pub call_session: CallSession,
    
    /// Local SDP for hold/resume operations
    pub local_sdp: Option<String>,
    
    /// Remote SDP for reference
    pub remote_sdp: Option<String>,
    
    /// When this session was created
    pub created_at: std::time::Instant,
    
    /// When this session was last updated
    pub updated_at: std::time::Instant,
}

impl Session {
    /// Create a new session from a CallSession with a specific role
    pub fn from_call_session_with_role(call_session: CallSession, role: SessionRole) -> Self {
        let now = std::time::Instant::now();
        Self {
            session_id: call_session.id.clone(),
            role,
            dialog_id: None,
            media_session_id: None,
            call_session,
            local_sdp: None,
            remote_sdp: None,
            created_at: now,
            updated_at: now,
        }
    }
    
    /// Create a new session from a CallSession (defaults to UAC for compatibility)
    pub fn from_call_session(call_session: CallSession) -> Self {
        Self::from_call_session_with_role(call_session, SessionRole::UAC)
    }
    
    /// Create a new session with just a session ID and role
    pub fn new_with_role(session_id: SessionId, role: SessionRole) -> Self {
        let call_session = CallSession {
            id: session_id.clone(),
            from: String::new(),
            to: String::new(),
            state: CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
            sip_call_id: None,
        };
        Self::from_call_session_with_role(call_session, role)
    }
    
    /// Create a new session with just a session ID (defaults to UAC for compatibility)
    pub fn new(session_id: SessionId) -> Self {
        Self::new_with_role(session_id, SessionRole::UAC)
    }
    
    /// Associate a dialog ID with this session
    pub fn set_dialog_id(&mut self, dialog_id: rvoip_dialog_core::DialogId) {
        tracing::debug!("Associated dialog {} with session {}", dialog_id, self.session_id);
        self.dialog_id = Some(dialog_id);
        self.updated_at = std::time::Instant::now();
    }
    
    /// Associate a media session ID with this session
    pub fn set_media_session_id(&mut self, media_session_id: crate::media::types::MediaSessionId) {
        tracing::debug!("Associated media session {} with session {}", media_session_id, self.session_id);
        self.media_session_id = Some(media_session_id);
        self.updated_at = std::time::Instant::now();
    }
    
    /// Update the call state
    pub fn update_call_state(&mut self, new_state: CallState) -> Result<()> {
        let old_state = self.call_session.state.clone();
        self.call_session.state = new_state.clone();
        self.updated_at = std::time::Instant::now();
        tracing::debug!("Session {} state: {:?} -> {:?}", self.session_id, old_state, new_state);
        Ok(())
    }
    
    /// Update call details (from and to)
    pub fn update_call_details(&mut self, from: String, to: String) {
        self.call_session.from = from;
        self.call_session.to = to;
        self.updated_at = std::time::Instant::now();
    }
    
    /// Check if session has all required IDs for a complete call
    pub fn is_fully_established(&self) -> bool {
        self.dialog_id.is_some() && 
        self.media_session_id.is_some() && 
        matches!(self.call_session.state, CallState::Active)
    }
    
    /// Check if session has dialog mapping
    pub fn has_dialog(&self) -> bool {
        self.dialog_id.is_some()
    }
    
    /// Check if session has media mapping
    pub fn has_media_session(&self) -> bool {
        self.media_session_id.is_some()
    }
    
    /// Get session duration
    pub fn duration(&self) -> std::time::Duration {
        self.updated_at.duration_since(self.created_at)
    }
    
    /// Get the call session for public API consumption
    pub fn as_call_session(&self) -> &CallSession {
        &self.call_session
    }
    
    /// Convert to call session (for API compatibility)
    pub fn into_call_session(self) -> CallSession {
        self.call_session
    }
    
    /// Get current state
    pub fn state(&self) -> &CallState {
        &self.call_session.state
    }
    
    /// Get dialog ID if present
    pub fn dialog_id(&self) -> Option<&rvoip_dialog_core::DialogId> {
        self.dialog_id.as_ref()
    }
    
    /// Get media session ID if present
    pub fn media_session_id(&self) -> Option<&crate::media::types::MediaSessionId> {
        self.media_session_id.as_ref()
    }
}

// Backward compatibility alias
pub type SessionImpl = Session; 