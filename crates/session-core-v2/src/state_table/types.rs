use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Session ID type
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        Self(format!("session-{}", uuid::Uuid::new_v4()))
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Direction of media flow
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum MediaFlowDirection {
    Send,
    Receive,
    Both,
    None,
}

/// Dialog ID type
pub type DialogId = String;

/// Media session ID type  
pub type MediaSessionId = String;

/// Call ID type
pub type CallId = String;

/// Key for looking up transitions in the state table
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateKey {
    pub role: Role,
    pub state: CallState,
    pub event: EventType,
}

/// Role in the call (caller or receiver)
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum Role {
    UAC,  // User Agent Client (caller)
    UAS,  // User Agent Server (receiver)
    Both, // Applies to both roles
}

/// Call states
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum CallState {
    Idle,
    Initiating,
    Ringing,
    EarlyMedia,
    Active,
    OnHold,
    Resuming,
    Bridged,
    Transferring,
    Terminating,
    Terminated,
    Failed(FailureReason),
}

/// Reasons for call failure
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum FailureReason {
    Timeout,
    Rejected,
    NetworkError,
    MediaError,
    ProtocolError,
    Other,
}

/// Event types that trigger transitions
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventType {
    // User-initiated events
    MakeCall { target: String },
    IncomingCall { from: String, sdp: Option<String> },
    AcceptCall,
    RejectCall { reason: String },
    HangupCall,
    HoldCall,
    ResumeCall,
    BlindTransfer { target: String },
    AttendedTransfer { target: String },
    
    // Media control events
    PlayAudio { file: String },
    StartRecording,
    StopRecording,
    SendDTMF { digits: String },
    
    // Dialog events (from dialog-core)
    DialogInvite,
    Dialog180Ringing,
    Dialog183SessionProgress,
    Dialog200OK,
    DialogACK,
    DialogBYE,
    DialogCANCEL,
    DialogREFER,
    DialogReINVITE,
    Dialog4xxFailure(u16),
    Dialog5xxFailure(u16),
    Dialog6xxFailure(u16),
    Dialog487RequestTerminated,
    DialogTimeout,
    DialogTerminated,
    DialogError(String),
    
    // Media events (from media-core)
    MediaSessionCreated,
    MediaSessionReady,
    MediaNegotiated,
    MediaFlowEstablished,
    MediaError(String),
    MediaEvent(String), // Generic media events like "rfc_compliant_media_creation_uac"
    
    // Internal coordination events
    InternalCheckReady,
    InternalACKSent,
    InternalUASMedia,
    InternalCleanupComplete,
    CheckConditions,
    PublishCallEstablished,
    
    // Bridge/Transfer events
    BridgeSessions { other_session: SessionId },
    UnbridgeSessions,
    InitiateTransfer { target: String },
    TransferAccepted,
    TransferProgress,
    TransferComplete,
    TransferFailed,
    
    // Session modification
    ModifySession,
    
    // Cleanup events
    CleanupComplete,
    Reset,
}

/// Transition definition - what happens when an event occurs in a state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    /// Conditions that must be true for this transition
    pub guards: Vec<Guard>,
    
    /// Actions to execute
    pub actions: Vec<Action>,
    
    /// Next state (if changing)
    pub next_state: Option<CallState>,
    
    /// Condition flags to update
    pub condition_updates: ConditionUpdates,
    
    /// Events to publish after transition
    pub publish_events: Vec<EventTemplate>,
}

/// Guards that must be satisfied for a transition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Guard {
    HasLocalSDP,
    HasRemoteSDP,
    HasNegotiatedConfig,
    AllConditionsMet,
    DialogEstablished,
    MediaReady,
    SDPNegotiated,
    IsIdle,
    InActiveCall,
    Custom(String),
}

/// Actions to execute during a transition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    // Dialog actions
    SendSIPResponse(u16, String),
    SendINVITE,
    SendACK,
    SendBYE,
    SendCANCEL,
    SendReINVITE,
    
    // Media actions
    StartMediaSession,
    StopMediaSession,
    NegotiateSDPAsUAC,
    NegotiateSDPAsUAS,
    PlayAudioFile(String),
    StartRecordingMedia,
    StopRecordingMedia,
    
    // State updates
    SetCondition(Condition, bool),
    StoreLocalSDP,
    StoreRemoteSDP,
    StoreNegotiatedConfig,
    
    // Bridge/Transfer actions
    CreateBridge(SessionId),
    DestroyBridge,
    InitiateBlindTransfer(String),
    InitiateAttendedTransfer(String),
    
    // Callbacks
    TriggerCallEstablished,
    TriggerCallTerminated,
    
    // Cleanup
    StartDialogCleanup,
    StartMediaCleanup,
    
    // Custom action for extension
    Custom(String),
}

/// Conditions that track readiness
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    DialogEstablished,
    MediaSessionReady,
    SDPNegotiated,
}

/// Updates to condition flags
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConditionUpdates {
    pub dialog_established: Option<bool>,
    pub media_session_ready: Option<bool>,
    pub sdp_negotiated: Option<bool>,
}

impl ConditionUpdates {
    pub fn none() -> Self {
        Self::default()
    }
    
    pub fn set_dialog_established(established: bool) -> Self {
        Self {
            dialog_established: Some(established),
            ..Default::default()
        }
    }
    
    pub fn set_media_ready(ready: bool) -> Self {
        Self {
            media_session_ready: Some(ready),
            ..Default::default()
        }
    }
    
    pub fn set_sdp_negotiated(negotiated: bool) -> Self {
        Self {
            sdp_negotiated: Some(negotiated),
            ..Default::default()
        }
    }
}

/// Event templates for publishing
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventTemplate {
    StateChanged,
    SessionCreated,
    IncomingCall,
    CallEstablished,
    CallTerminated,
    CallFailed,
    MediaFlowEstablished,
    MediaNegotiated,
    MediaSessionReady,
    Custom(String),
}

/// Master state table containing all transitions
pub struct MasterStateTable {
    transitions: HashMap<StateKey, Transition>,
}

/// Type alias for external use
pub type StateTable = MasterStateTable;

impl MasterStateTable {
    pub fn new() -> Self {
        Self {
            transitions: HashMap::new(),
        }
    }
    
    pub fn insert(&mut self, key: StateKey, transition: Transition) {
        self.transitions.insert(key, transition);
    }
    
    pub fn get(&self, key: &StateKey) -> Option<&Transition> {
        self.transitions.get(key)
    }
    
    pub fn get_transition(&self, key: &StateKey) -> Option<&Transition> {
        self.transitions.get(key)
    }
    
    pub fn has_transition(&self, key: &StateKey) -> bool {
        self.transitions.contains_key(key)
    }
    
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        
        // Check for orphan states (updated with new states)
        for state in [
            CallState::Idle,
            CallState::Initiating,
            CallState::Ringing,
            CallState::EarlyMedia,
            CallState::Active,
            CallState::OnHold,
            CallState::Resuming,
            CallState::Bridged,
            CallState::Transferring,
            CallState::Terminating,
        ] {
            let has_exit = self.transitions.iter().any(|(k, _)| k.state == state);
            if !has_exit {
                errors.push(format!("State {:?} has no exit transitions", state));
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}