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

/// Event pattern for matching events in state table
///
/// This allows flexible matching where YAML can specify:
/// - Just the event type (wildcard match on all parameters)
/// - Event type + specific parameter values (constrained match)
///
/// Example YAML:
/// ```yaml
/// event:
///   type: "TransferRequested"  # Matches any TransferRequested
///
/// event:
///   type: "TransferRequested"  # Matches only blind transfers
///   transfer_type: "blind"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventPattern {
    /// The event type name (e.g., "TransferRequested", "IncomingCall")
    #[serde(rename = "type")]
    pub event_type: String,

    /// Optional parameter constraints
    /// If empty, matches any parameters (wildcard)
    /// If specified, all parameters must match
    #[serde(flatten)]
    pub parameters: HashMap<String, serde_yaml::Value>,
}

impl EventPattern {
    /// Check if this pattern matches the given event
    ///
    /// Matching rules:
    /// 1. Event type name must match
    /// 2. If no parameters specified in pattern -> wildcard match (always true)
    /// 3. If parameters specified -> all must match the event's values
    pub fn matches(&self, event: &EventType) -> bool {
        // Check event type matches
        if self.event_type != event.type_name() {
            return false;
        }

        // If no parameters specified, it's a wildcard match
        if self.parameters.is_empty() {
            return true;
        }

        // Check each specified parameter matches
        for (key, pattern_value) in &self.parameters {
            match event.get_parameter(key) {
                Some(event_value) => {
                    // Compare the parameter value
                    if !values_match(pattern_value, &event_value) {
                        return false;
                    }
                },
                None => {
                    // Pattern specifies a parameter that the event doesn't have
                    return false;
                }
            }
        }

        true
    }
}

/// Helper function to compare YAML values with event parameter strings
fn values_match(pattern_value: &serde_yaml::Value, event_value: &str) -> bool {
    match pattern_value {
        serde_yaml::Value::String(s) => s == event_value,
        serde_yaml::Value::Number(n) => n.to_string() == event_value,
        serde_yaml::Value::Bool(b) => b.to_string() == event_value,
        _ => false,
    }
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
    /// Get the type name of this event (without parameter values)
    ///
    /// This is used for pattern matching in the state table.
    /// Returns the variant name as a string.
    pub fn type_name(&self) -> &'static str {
        match self {
            // User-initiated events
            EventType::MakeCall { .. } => "MakeCall",
            EventType::IncomingCall { .. } => "IncomingCall",
            EventType::AcceptCall => "AcceptCall",
            EventType::RejectCall { .. } => "RejectCall",
            EventType::HangupCall => "HangupCall",
            EventType::HoldCall => "HoldCall",
            EventType::ResumeCall => "ResumeCall",
            EventType::MuteCall => "MuteCall",
            EventType::UnmuteCall => "UnmuteCall",
            EventType::AttendedTransfer { .. } => "AttendedTransfer",

            // Media control events
            EventType::PlayAudio { .. } => "PlayAudio",
            EventType::StartRecording => "StartRecording",
            EventType::StopRecording => "StopRecording",
            EventType::SendDTMF { .. } => "SendDTMF",

            // Dialog events
            EventType::DialogCreated { .. } => "DialogCreated",
            EventType::CallEstablished { .. } => "CallEstablished",
            EventType::DialogInvite => "DialogInvite",
            EventType::Dialog180Ringing => "Dialog180Ringing",
            EventType::Dialog183SessionProgress => "Dialog183SessionProgress",
            EventType::Dialog200OK => "Dialog200OK",
            EventType::DialogACK => "DialogACK",
            EventType::DialogBYE => "DialogBYE",
            EventType::DialogCANCEL => "DialogCANCEL",
            EventType::DialogREFER => "DialogREFER",
            EventType::DialogReINVITE => "DialogReINVITE",
            EventType::Dialog4xxFailure(_) => "Dialog4xxFailure",
            EventType::Dialog5xxFailure(_) => "Dialog5xxFailure",
            EventType::Dialog6xxFailure(_) => "Dialog6xxFailure",
            EventType::Dialog487RequestTerminated => "Dialog487RequestTerminated",
            EventType::DialogTimeout => "DialogTimeout",
            EventType::DialogTerminated => "DialogTerminated",
            EventType::DialogError(_) => "DialogError",
            EventType::DialogStateChanged { .. } => "DialogStateChanged",
            EventType::ReinviteReceived { .. } => "ReinviteReceived",
            EventType::TransferRequested { .. } => "TransferRequested",

            // Media events
            EventType::MediaSessionCreated => "MediaSessionCreated",
            EventType::MediaSessionReady => "MediaSessionReady",
            EventType::MediaNegotiated => "MediaNegotiated",
            EventType::MediaFlowEstablished => "MediaFlowEstablished",
            EventType::MediaError(_) => "MediaError",
            EventType::MediaEvent(_) => "MediaEvent",
            EventType::MediaQualityDegraded { .. } => "MediaQualityDegraded",
            EventType::DtmfDetected { .. } => "DtmfDetected",
            EventType::RtpTimeout { .. } => "RtpTimeout",
            EventType::PacketLossThresholdExceeded { .. } => "PacketLossThresholdExceeded",

            // Internal coordination events
            EventType::InternalCheckReady => "InternalCheckReady",
            EventType::InternalACKSent => "InternalACKSent",
            EventType::InternalUASMedia => "InternalUASMedia",
            EventType::InternalCleanupComplete => "InternalCleanupComplete",
            EventType::CheckConditions => "CheckConditions",
            EventType::PublishCallEstablished => "PublishCallEstablished",

            // Conference events
            EventType::CreateConference { .. } => "CreateConference",
            EventType::AddParticipant { .. } => "AddParticipant",
            EventType::JoinConference { .. } => "JoinConference",
            EventType::LeaveConference => "LeaveConference",
            EventType::MuteInConference => "MuteInConference",
            EventType::UnmuteInConference => "UnmuteInConference",

            // Bridge/Transfer events
            EventType::BridgeSessions { .. } => "BridgeSessions",
            EventType::UnbridgeSessions => "UnbridgeSessions",
            EventType::BlindTransfer { .. } => "BlindTransfer",
            EventType::StartAttendedTransfer { .. } => "StartAttendedTransfer",
            EventType::CompleteAttendedTransfer => "CompleteAttendedTransfer",
            EventType::TransferAccepted => "TransferAccepted",
            EventType::TransferProgress => "TransferProgress",
            EventType::TransferComplete => "TransferComplete",
            EventType::TransferSuccess => "TransferSuccess",
            EventType::TransferFailed => "TransferFailed",

            // Session modification
            EventType::ModifySession => "ModifySession",

            // Registration events
            EventType::StartRegistration => "StartRegistration",
            EventType::Registration200OK => "Registration200OK",
            EventType::RegistrationFailed(_) => "RegistrationFailed",
            EventType::UnregisterRequest => "UnregisterRequest",
            EventType::RegistrationExpired => "RegistrationExpired",

            // Subscription/Notify events
            EventType::StartSubscription => "StartSubscription",
            EventType::ReceiveNOTIFY => "ReceiveNOTIFY",
            EventType::SendNOTIFY => "SendNOTIFY",
            EventType::SubscriptionAccepted => "SubscriptionAccepted",
            EventType::SubscriptionFailed(_) => "SubscriptionFailed",
            EventType::SubscriptionExpired => "SubscriptionExpired",
            EventType::UnsubscribeRequest => "UnsubscribeRequest",

            // Message events
            EventType::SendMessage => "SendMessage",
            EventType::ReceiveMESSAGE => "ReceiveMESSAGE",
            EventType::MessageDelivered => "MessageDelivered",
            EventType::MessageFailed(_) => "MessageFailed",

            // Cleanup events
            EventType::CleanupComplete => "CleanupComplete",
            EventType::Reset => "Reset",

            // Internal transfer coordination events
            EventType::InternalProceedWithTransfer => "InternalProceedWithTransfer",
            EventType::InternalMakeTransferCall => "InternalMakeTransferCall",
            EventType::InternalTransferCallEstablished => "InternalTransferCallEstablished",
        }
    }

    /// Get a parameter value from this event by key
    ///
    /// Returns Some(value) if the event has this parameter, None otherwise.
    /// This is used for pattern matching in the state table.
    pub fn get_parameter(&self, key: &str) -> Option<String> {
        match (self, key) {
            // User-initiated events
            (EventType::MakeCall { target }, "target") => Some(target.clone()),
            (EventType::IncomingCall { from, .. }, "from") => Some(from.clone()),
            (EventType::RejectCall { reason }, "reason") => Some(reason.clone()),
            (EventType::AttendedTransfer { target }, "target") => Some(target.clone()),

            // Media control events
            (EventType::PlayAudio { file }, "file") => Some(file.clone()),
            (EventType::SendDTMF { digits }, "digits") => Some(digits.clone()),

            // Dialog events
            (EventType::DialogCreated { dialog_id, .. }, "dialog_id") => Some(dialog_id.clone()),
            (EventType::DialogCreated { call_id, .. }, "call_id") => Some(call_id.clone()),
            (EventType::CallEstablished { session_id, .. }, "session_id") => Some(session_id.clone()),
            (EventType::DialogStateChanged { old_state, .. }, "old_state") => Some(old_state.clone()),
            (EventType::DialogStateChanged { new_state, .. }, "new_state") => Some(new_state.clone()),
            (EventType::TransferRequested { refer_to, .. }, "refer_to") => Some(refer_to.clone()),
            (EventType::TransferRequested { transfer_type, .. }, "transfer_type") => Some(transfer_type.clone()),

            // Media events
            (EventType::MediaError(msg), "error") => Some(msg.clone()),
            (EventType::MediaEvent(event), "event") => Some(event.clone()),
            (EventType::MediaQualityDegraded { packet_loss_percent, .. }, "packet_loss_percent") =>
                Some(packet_loss_percent.to_string()),
            (EventType::MediaQualityDegraded { jitter_ms, .. }, "jitter_ms") =>
                Some(jitter_ms.to_string()),
            (EventType::MediaQualityDegraded { severity, .. }, "severity") =>
                Some(severity.clone()),
            (EventType::DtmfDetected { digit, .. }, "digit") => Some(digit.to_string()),
            (EventType::DtmfDetected { duration_ms, .. }, "duration_ms") =>
                Some(duration_ms.to_string()),

            // Conference events
            (EventType::CreateConference { name }, "name") => Some(name.clone()),
            (EventType::AddParticipant { session_id }, "session_id") => Some(session_id.clone()),
            (EventType::JoinConference { conference_id }, "conference_id") => Some(conference_id.clone()),

            // Bridge/Transfer events
            (EventType::BridgeSessions { other_session }, "other_session") => Some(other_session.0.clone()),
            (EventType::BlindTransfer { target }, "target") => Some(target.clone()),
            (EventType::StartAttendedTransfer { target }, "target") => Some(target.clone()),

            // Status code events
            (EventType::Dialog4xxFailure(code), "code") => Some(code.to_string()),
            (EventType::Dialog5xxFailure(code), "code") => Some(code.to_string()),
            (EventType::Dialog6xxFailure(code), "code") => Some(code.to_string()),
            (EventType::RegistrationFailed(code), "code") => Some(code.to_string()),
            (EventType::SubscriptionFailed(code), "code") => Some(code.to_string()),
            (EventType::MessageFailed(code), "code") => Some(code.to_string()),

            // Default: no such parameter
            _ => None,
        }
    }

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
    SendTransferNOTIFYSuccess,
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
        
        // First check for exact state match
        if let Some(transition) = self.transitions.get(&normalized_key) {
            return Some(transition);
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
        
        // Check exact match first
        if self.transitions.contains_key(&normalized_key) {
            return true;
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