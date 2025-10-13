use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use crate::types::CallState;

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

/// Media direction for hold/resume
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum MediaDirection {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

/// Dialog ID type - wraps UUID for compatibility with rvoip_dialog_core
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct DialogId(pub uuid::Uuid);

impl DialogId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    
    /// Create from a UUID
    pub fn from_uuid(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }
    
    /// Get the inner UUID
    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl std::fmt::Display for DialogId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Conversion from rvoip_dialog_core::DialogId to our DialogId
impl From<rvoip_dialog_core::DialogId> for DialogId {
    fn from(dialog_id: rvoip_dialog_core::DialogId) -> Self {
        Self(dialog_id.0)
    }
}

// Conversion from our DialogId to rvoip_dialog_core::DialogId  
impl From<DialogId> for rvoip_dialog_core::DialogId {
    fn from(dialog_id: DialogId) -> Self {
        rvoip_dialog_core::DialogId(dialog_id.0)
    }
}

// Allow conversion from &DialogId to rvoip_dialog_core::DialogId
impl From<&DialogId> for rvoip_dialog_core::DialogId {
    fn from(dialog_id: &DialogId) -> Self {
        rvoip_dialog_core::DialogId(dialog_id.0)
    }
}

/// Media session ID type  
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct MediaSessionId(pub String);

impl MediaSessionId {
    pub fn new() -> Self {
        Self(format!("media-{}", uuid::Uuid::new_v4()))
    }
}

impl std::fmt::Display for MediaSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

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
    MuteCall,
    UnmuteCall,
    AttendedTransfer { target: String },

    // Media control events
    PlayAudio { file: String },
    StartRecording,
    StopRecording,
    SendDTMF { digits: String },
    
    // Dialog events (from dialog-core)
    DialogCreated { dialog_id: String, call_id: String },
    CallEstablished { session_id: String, sdp_answer: Option<String> },
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
    DialogStateChanged { old_state: String, new_state: String },
    ReinviteReceived { sdp: Option<String> },
    TransferRequested { refer_to: String, transfer_type: String },
    
    // Media events (from media-core)
    MediaSessionCreated,
    MediaSessionReady,
    MediaNegotiated,
    MediaFlowEstablished,
    MediaError(String),
    MediaEvent(String), // Generic media events like "rfc_compliant_media_creation_uac"
    MediaQualityDegraded { packet_loss_percent: u32, jitter_ms: u32, severity: String },
    DtmfDetected { digit: char, duration_ms: u32 },
    RtpTimeout { last_packet_time: String },
    PacketLossThresholdExceeded { loss_percentage: u32 },
    
    // Internal coordination events
    InternalCheckReady,
    InternalACKSent,
    InternalUASMedia,
    InternalCleanupComplete,
    CheckConditions,
    PublishCallEstablished,
    
    // Conference events
    CreateConference { name: String },
    AddParticipant { session_id: String },
    JoinConference { conference_id: String },
    LeaveConference,
    MuteInConference,
    UnmuteInConference,
    
    // Bridge/Transfer events
    BridgeSessions { other_session: SessionId },
    UnbridgeSessions,
    BlindTransfer { target: String },
    StartAttendedTransfer { target: String },
    CompleteAttendedTransfer,
    TransferAccepted,
    TransferProgress,
    TransferComplete,
    TransferSuccess,
    TransferFailed,
    
    // Session modification
    ModifySession,

    // Registration events
    StartRegistration,
    Registration200OK,
    RegistrationFailed(u16),
    UnregisterRequest,
    RegistrationExpired,

    // Subscription/Notify events
    StartSubscription,
    ReceiveNOTIFY,
    SendNOTIFY,
    SubscriptionAccepted,
    SubscriptionFailed(u16),
    SubscriptionExpired,
    UnsubscribeRequest,

    // Message events
    SendMessage,
    ReceiveMESSAGE,
    MessageDelivered,
    MessageFailed(u16),

    // Cleanup events
    CleanupComplete,
    Reset,

    // Internal transfer coordination events
    InternalProceedWithTransfer,
    InternalMakeTransferCall,
    InternalTransferCallEstablished,
}

impl EventType {
    /// Normalize the event for state table lookups by removing runtime-specific field values.
    /// This allows the state table to match on event type rather than exact field values.
    pub fn normalize(&self) -> Self {
        match self {
            // User events - normalize to empty/default values
            EventType::MakeCall { .. } => EventType::MakeCall { target: String::new() },
            EventType::IncomingCall { .. } => EventType::IncomingCall { from: String::new(), sdp: None },
            EventType::RejectCall { .. } => EventType::RejectCall { reason: String::new() },
            EventType::BlindTransfer { .. } => EventType::BlindTransfer { target: String::new() },
            EventType::AttendedTransfer { .. } => EventType::AttendedTransfer { target: String::new() },
            
            // Media events - normalize
            EventType::PlayAudio { .. } => EventType::PlayAudio { file: String::new() },
            EventType::SendDTMF { .. } => EventType::SendDTMF { digits: String::new() },

            // Conference events - normalize
            EventType::CreateConference { .. } => EventType::CreateConference { name: String::new() },
            EventType::AddParticipant { .. } => EventType::AddParticipant { session_id: String::new() },
            EventType::JoinConference { .. } => EventType::JoinConference { conference_id: String::new() },

            // Bridge events - normalize session ID
            EventType::BridgeSessions { .. } => EventType::BridgeSessions { other_session: SessionId::new() },
            EventType::StartAttendedTransfer { .. } => EventType::StartAttendedTransfer { target: String::new() },

            // Transfer events - normalize
            EventType::TransferRequested { .. } => EventType::TransferRequested {
                refer_to: String::new(),
                transfer_type: String::new()
            },

            // Registration events - normalize status codes
            EventType::RegistrationFailed(_) => EventType::RegistrationFailed(0),

            // Subscription events - normalize status codes
            EventType::SubscriptionFailed(_) => EventType::SubscriptionFailed(0),

            // Message events - normalize status codes
            EventType::MessageFailed(_) => EventType::MessageFailed(0),

            // Events without fields pass through unchanged
            _ => self.clone(),
        }
    }
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
    IsRegistered,
    IsSubscribed,
    HasActiveSubscription,
    Custom(String),
}

/// Actions to execute during a transition
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Action {
    // Dialog actions
    CreateDialog,
    CreateMediaSession,
    GenerateLocalSDP,
    SendSIPResponse(u16, String),
    SendINVITE,
    SendACK,
    SendBYE,
    SendCANCEL,
    SendReINVITE,
    
    // Call control actions
    HoldCall,
    ResumeCall,
    TransferCall(String),
    SendDTMF(char),
    StartRecording,
    StopRecording,
    
    // Media actions
    StartMediaSession,
    StopMediaSession,
    NegotiateSDPAsUAC,
    NegotiateSDPAsUAS,
    PlayAudioFile(String),
    StartRecordingMedia,
    StopRecordingMedia,
    
    // Conference actions
    CreateAudioMixer,
    RedirectToMixer,
    ConnectToMixer,
    DisconnectFromMixer,
    MuteToMixer,
    UnmuteToMixer,
    DestroyMixer,
    BridgeToMixer,
    RestoreDirectMedia,
    StartRecordingMixer,
    StopRecordingMixer,
    
    // Media direction actions
    UpdateMediaDirection { direction: MediaDirection },
    
    // Transfer actions
    SendREFER,
    SendREFERWithReplaces,
    HoldCurrentCall,
    CreateConsultationCall,
    TerminateConsultationCall,

    // Blind transfer recipient actions
    AcceptTransferREFER,
    SendTransferNOTIFY,
    SendTransferNOTIFYRinging,
    SendTransferNOTIFYSuccess,
    SendTransferNOTIFYFailure,
    StoreTransferTarget,
    TerminateCurrentCall,

    // Audio control
    MuteLocalAudio,
    UnmuteLocalAudio,
    SendDTMFTone,
    
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
    
    // Resource management
    RestoreMediaFlow,
    ReleaseAllResources,
    StartEmergencyCleanup,
    AttemptMediaRecovery,
    CleanupResources,
    
    // Callbacks
    TriggerCallEstablished,
    TriggerCallTerminated,
    
    // Cleanup
    StartDialogCleanup,
    StartMediaCleanup,

    // Registration actions
    SendREGISTER,
    ProcessRegistrationResponse,

    // Subscription actions
    SendSUBSCRIBE,
    ProcessNOTIFY,
    SendNOTIFY,

    // Message actions
    SendMESSAGE,
    ProcessMESSAGE,

    // Generic cleanup actions
    CleanupDialog,
    CleanupMedia,

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

/// States that must always have exit transitions if used
const CORE_STATES_REQUIRING_EXITS: &[CallState] = &[
    CallState::Idle,
    CallState::Initiating,
    CallState::Ringing,
    CallState::Active,
];

/// Master state table containing all transitions
pub struct MasterStateTable {
    transitions: HashMap<StateKey, Transition>,
    /// Wildcard transitions that apply to any state
    wildcard_transitions: HashMap<(Role, EventType), Transition>,
}

/// Type alias for external use
pub type StateTable = MasterStateTable;

impl MasterStateTable {
    pub fn new() -> Self {
        Self {
            transitions: HashMap::new(),
            wildcard_transitions: HashMap::new(),
        }
    }
    
    pub fn insert(&mut self, key: StateKey, transition: Transition) {
        // Always normalize the event when inserting
        let normalized_key = StateKey {
            role: key.role,
            state: key.state,
            event: key.event.normalize(),
        };
        self.transitions.insert(normalized_key, transition);
    }
    
    /// Insert a wildcard transition that applies to any state
    pub fn insert_wildcard(&mut self, role: Role, event: EventType, transition: Transition) {
        let normalized_event = event.normalize();
        self.wildcard_transitions.insert((role, normalized_event), transition);
    }
    
    pub fn get(&self, key: &StateKey) -> Option<&Transition> {
        // Normalize the event for lookup
        let normalized_key = StateKey {
            role: key.role,
            state: key.state,
            event: key.event.normalize(),
        };

        // First check for exact role match
        if let Some(transition) = self.transitions.get(&normalized_key) {
            return Some(transition);
        }

        // If UAC or UAS, also check for Role::Both transitions
        if key.role == Role::UAC || key.role == Role::UAS {
            let both_key = StateKey {
                role: Role::Both,
                state: key.state,
                event: key.event.normalize(),
            };
            if let Some(transition) = self.transitions.get(&both_key) {
                return Some(transition);
            }
        }

        // If no exact match, check for wildcard transition
        let normalized_event = key.event.normalize();
        self.wildcard_transitions.get(&(key.role, normalized_event))
    }
    
    pub fn get_transition(&self, key: &StateKey) -> Option<&Transition> {
        self.get(key)
    }
    
    pub fn has_transition(&self, key: &StateKey) -> bool {
        // Normalize the event for lookup
        let normalized_key = StateKey {
            role: key.role,
            state: key.state,
            event: key.event.normalize(),
        };

        // Check exact role match first
        if self.transitions.contains_key(&normalized_key) {
            return true;
        }

        // If UAC or UAS, also check for Role::Both transitions
        if key.role == Role::UAC || key.role == Role::UAS {
            let both_key = StateKey {
                role: Role::Both,
                state: key.state,
                event: key.event.normalize(),
            };
            if self.transitions.contains_key(&both_key) {
                return true;
            }
        }

        // Check wildcard match
        let normalized_event = key.event.normalize();
        self.wildcard_transitions.contains_key(&(key.role, normalized_event))
    }
    
    pub fn transition_count(&self) -> usize {
        self.transitions.len() + self.wildcard_transitions.len()
    }
    
    /// Collect all states referenced in this state table
    pub fn collect_used_states(&self) -> HashSet<CallState> {
        let mut states = HashSet::new();
        
        // Collect from regular transitions
        for (key, transition) in &self.transitions {
            states.insert(key.state);
            if let Some(next_state) = &transition.next_state {
                states.insert(*next_state);
            }
        }
        
        // Collect from wildcard transitions
        for (_, transition) in &self.wildcard_transitions {
            if let Some(next_state) = &transition.next_state {
                states.insert(*next_state);
            }
        }
        
        states
    }
    
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        
        // Collect states actually used in this table
        let used_states = self.collect_used_states();
        
        // Check for orphan states only among used states
        for state in used_states.iter() {
            // Skip terminal states
            if matches!(state, CallState::Terminated | CallState::Cancelled | CallState::Failed(_)) {
                continue;
            }
            
            // Check if state has exit transitions
            let has_exact_exit = self.transitions.iter().any(|(k, _)| k.state == *state);
            let has_wildcard_exit = !self.wildcard_transitions.is_empty();
            
            if !has_exact_exit && !has_wildcard_exit {
                // Only error for core states, warn for others
                if CORE_STATES_REQUIRING_EXITS.contains(state) {
                    errors.push(format!("Core state {:?} has no exit transitions", state));
                }
                // Note: We could collect warnings here for non-core states if desired
                // For now, we just skip them to avoid false positives
            }
        }
        
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}