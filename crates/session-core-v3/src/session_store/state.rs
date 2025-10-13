use std::net::SocketAddr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};

use crate::state_table::{Role, ConditionUpdates};
use crate::types::CallState;
use super::history::{SessionHistory, HistoryConfig, TransitionRecord};

/// Negotiated media configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
}

/// Transfer state tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    None,
    ConsultationInProgress,
    TransferInitiated,
    TransferCompleted,
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
    
    // Bridging information (for peer-to-peer conferencing)
    pub bridged_to: Option<SessionId>, // Session this is bridged to

    // Conference information
    pub conference_mixer_id: Option<MediaSessionId>, // Mixer ID if hosting conference
    pub transfer_target: Option<String>, // Target for transfers
    pub dtmf_digits: Option<String>, // DTMF digits to send

    // Attended transfer tracking
    pub consultation_session_id: Option<SessionId>, // Consultation call session for attended transfer
    pub original_session_id: Option<SessionId>, // Original session (set in consultation call)
    pub transfer_state: TransferState, // Current transfer state
    pub transfer_notify_dialog: Option<DialogId>, // Dialog to send NOTIFY messages to (for blind transfer)

    // Transfer coordination fields
    pub replaces_header: Option<String>, // Replaces header for attended transfer
    pub referred_by: Option<String>, // Referred-By header from REFER request
    pub refer_transaction_id: Option<String>, // Transaction ID for REFER request (for sending response)
    pub is_transfer_call: bool, // Flag indicating this session is a result of a transfer
    pub transferor_session_id: Option<SessionId>, // Session ID of who sent us the REFER (for NOTIFY messages)

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
            conference_mixer_id: None,
            transfer_target: None,
            dtmf_digits: None,
            consultation_session_id: None,
            original_session_id: None,
            transfer_state: TransferState::None,
            transfer_notify_dialog: None,
            replaces_header: None,
            referred_by: None,
            refer_transaction_id: None,
            is_transfer_call: false,
            transferor_session_id: None,
            created_at: now,
            history: None,
        }
    }
    
    /// Create with history tracking enabled
    pub fn with_history(session_id: SessionId, role: Role, config: HistoryConfig) -> Self {
        let mut state = Self::new(session_id, role);
        state.history = Some(SessionHistory::new(config));
        state
    }
    
    /// Record a transition in history
    pub fn record_transition(&mut self, record: TransitionRecord) {
        if let Some(ref mut history) = self.history {
            history.record_transition(record);
        }
    }
    
    /// Transition to a new state
    pub fn transition_to(&mut self, new_state: CallState) {
        if let Some(ref mut history) = self.history {
            use crate::session_store::TransitionRecord;
            use crate::state_table::EventType;
            let now = Instant::now();
            let record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0, // Will be set by history
                from_state: self.call_state,
                event: EventType::MediaEvent("transition_to".to_string()),
                to_state: Some(new_state),
                guards_evaluated: vec![],
                actions_executed: vec![],
                duration_ms: 0,
                errors: vec![],
                events_published: vec![],
            };
            history.record_transition(record);
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