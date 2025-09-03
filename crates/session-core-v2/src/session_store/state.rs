use std::net::SocketAddr;
use std::time::Instant;
use serde::{Deserialize, Serialize};
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};

use crate::state_table::{Role, CallState, ConditionUpdates};
use super::history::SessionHistory;

/// Negotiated media configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
}

/// Complete state of a session
#[derive(Debug, Clone)]
pub struct SessionState {
    // Identity
    pub session_id: SessionId,
    pub role: Role,
    
    // Current state
    pub call_state: CallState,
    pub entered_state_at: Instant,
    
    // Readiness conditions (the 3 flags)
    pub dialog_established: bool,
    pub media_session_ready: bool,
    pub sdp_negotiated: bool,
    
    // Track if call established was triggered
    pub call_established_triggered: bool,
    
    // SDP data
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub negotiated_config: Option<NegotiatedConfig>,
    
    // Related IDs
    pub dialog_id: Option<DialogId>,
    pub media_session_id: Option<MediaSessionId>,
    pub call_id: Option<CallId>,
    
    // SIP URIs
    pub local_uri: Option<String>,  // From URI for UAC, To URI for UAS
    pub remote_uri: Option<String>, // To URI for UAC, From URI for UAS
    
    // Store last 200 OK response for ACK
    pub last_200_ok: Option<Vec<u8>>, // Serialized response
    
    // Bridging information
    pub bridged_to: Option<SessionId>, // Session this is bridged to
    
    // Timestamps
    pub created_at: Instant,
    
    // Optional history tracking
    pub history: Option<SessionHistory>,
}

impl SessionState {
    /// Create a new session state
    pub fn new(session_id: SessionId, role: Role) -> Self {
        let now = Instant::now();
        Self {
            session_id,
            role,
            call_state: CallState::Idle,
            entered_state_at: now,
            dialog_established: false,
            media_session_ready: false,
            sdp_negotiated: false,
            call_established_triggered: false,
            local_sdp: None,
            remote_sdp: None,
            negotiated_config: None,
            dialog_id: None,
            media_session_id: None,
            call_id: None,
            local_uri: None,
            remote_uri: None,
            last_200_ok: None,
            bridged_to: None,
            created_at: now,
            history: None,
        }
    }
    
    /// Create with history tracking enabled
    pub fn with_history(session_id: SessionId, role: Role) -> Self {
        let mut state = Self::new(session_id, role);
        state.history = Some(SessionHistory::new(100));
        state
    }
    
    /// Transition to a new state
    pub fn transition_to(&mut self, new_state: CallState) {
        if let Some(ref mut history) = self.history {
            history.record_transition(
                self.call_state,
                new_state,
                Instant::now(),
            );
        }
        self.call_state = new_state;
        self.entered_state_at = Instant::now();
    }
    
    /// Apply condition updates from a transition
    pub fn apply_condition_updates(&mut self, updates: &ConditionUpdates) {
        if let Some(value) = updates.dialog_established {
            self.dialog_established = value;
        }
        if let Some(value) = updates.media_session_ready {
            self.media_session_ready = value;
        }
        if let Some(value) = updates.sdp_negotiated {
            self.sdp_negotiated = value;
        }
    }
    
    /// Check if all readiness conditions are met
    pub fn all_conditions_met(&self) -> bool {
        self.dialog_established && self.media_session_ready && self.sdp_negotiated
    }
    
    /// Get time spent in current state
    pub fn time_in_state(&self) -> std::time::Duration {
        Instant::now() - self.entered_state_at
    }
    
    /// Get total session duration
    pub fn session_duration(&self) -> std::time::Duration {
        Instant::now() - self.created_at
    }
}