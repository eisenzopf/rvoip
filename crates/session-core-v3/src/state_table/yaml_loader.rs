//! YAML-based state table loader for session coordination
//! 
//! This module loads state tables from YAML files, focusing on coordination
//! between dialog-core and media-core layers without duplicating their logic.

use std::collections::HashMap;
use std::path::Path;
use std::fs;
use serde::{Deserialize, Serialize};
use tracing::{info, debug};

use super::{
    StateTable, StateTableBuilder, StateKey, Transition, 
    Role, EventType, Guard, Action, 
    ConditionUpdates, EventTemplate, Condition, SessionId
};
use crate::errors::{Result, SessionError};
use crate::types::{CallState, FailureReason};

/// YAML representation of the complete state table
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlStateTable {
    /// Version of the state table format
    pub version: String,
    
    /// Metadata about the state table
    #[serde(default)]
    pub metadata: YamlMetadata,
    
    /// List of valid states
    #[serde(default)]
    pub states: Vec<YamlStateDefinition>,
    
    /// List of coordination conditions
    #[serde(default)]
    pub conditions: Vec<YamlConditionDefinition>,
    
    /// List of state transitions
    pub transitions: Vec<YamlTransition>,
}

/// Metadata about the state table
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct YamlMetadata {
    /// Description of the state table's purpose
    #[serde(default)]
    pub description: String,
    
    /// Author of the state table
    #[serde(default)]
    pub author: String,
    
    /// Date of last modification
    #[serde(default)]
    pub date: String,
}

/// Definition of a state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlStateDefinition {
    /// Name of the state
    pub name: String,
    
    /// Description of what this state represents
    #[serde(default)]
    pub description: String,
}

/// Definition of a coordination condition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlConditionDefinition {
    /// Name of the condition
    pub name: String,
    
    /// Description of what this condition tracks
    #[serde(default)]
    pub description: String,
    
    /// Default value
    #[serde(default)]
    pub default: bool,
}

/// YAML representation of a single transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlTransition {
    /// Role this transition applies to (UAC, UAS, or Both)
    pub role: String,
    
    /// Current state
    pub state: String,
    
    /// Event that triggers this transition
    pub event: YamlEvent,
    
    /// Guards that must be satisfied
    #[serde(default)]
    pub guards: Vec<YamlGuard>,
    
    /// Actions to execute
    #[serde(default)]
    pub actions: Vec<YamlAction>,
    
    /// Next state to transition to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_state: Option<String>,
    
    /// Condition updates to apply
    #[serde(default, skip_serializing_if = "YamlConditionUpdates::is_empty")]
    pub conditions: YamlConditionUpdates,
    
    /// Events to publish
    #[serde(default)]
    pub publish: Vec<String>,
    
    /// Description of this transition
    #[serde(default)]
    pub description: String,
}

/// YAML representation of an event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlEvent {
    /// Simple event (just a string)
    Simple(String),
    
    /// Complex event with type and parameters
    Complex {
        #[serde(rename = "type")]
        event_type: String,
        
        #[serde(flatten)]
        parameters: HashMap<String, serde_yaml::Value>,
    },
}

/// YAML representation of a guard condition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlGuard {
    /// Simple guard (just a string)
    Simple(String),
    
    /// Complex guard with parameters
    Complex {
        #[serde(rename = "type")]
        guard_type: String,
        
        #[serde(flatten)]
        parameters: HashMap<String, serde_yaml::Value>,
    },
}

/// YAML representation of an action
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum YamlAction {
    /// Simple action (just a string)
    Simple(String),
    
    /// Complex action with parameters
    Complex {
        #[serde(rename = "type")]
        action_type: String,
        
        #[serde(flatten)]
        parameters: HashMap<String, serde_yaml::Value>,
    },
}

/// YAML representation of condition updates
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct YamlConditionUpdates {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dialog_established: Option<bool>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_session_ready: Option<bool>,
    
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sdp_negotiated: Option<bool>,
}

impl YamlConditionUpdates {
    fn is_empty(&self) -> bool {
        self.dialog_established.is_none() 
            && self.media_session_ready.is_none()
            && self.sdp_negotiated.is_none()
    }
}

/// Default state table embedded in the binary
const DEFAULT_STATE_TABLE_YAML: &str = include_str!("../../state_tables/default.yaml");

/// YAML table loader
pub struct YamlTableLoader {
    /// Builder for constructing the state table
    builder: StateTableBuilder,
    
    /// Loaded YAML data
    yaml_data: Option<YamlStateTable>,
}

impl YamlTableLoader {
    /// Create a new YAML table loader
    pub fn new() -> Self {
        Self {
            builder: StateTableBuilder::new(),
            yaml_data: None,
        }
    }
    
    /// Load the default embedded state table
    pub fn load_default() -> Result<StateTable> {
        Self::load_embedded_default()
    }
    
    /// Load the embedded default state table (always succeeds)
    pub fn load_embedded_default() -> Result<StateTable> {
        let mut loader = Self::new();
        loader.load_from_string(DEFAULT_STATE_TABLE_YAML)
            .expect("Embedded default state table must be valid");
        loader.build()
    }
    
    /// Load state table from a file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<StateTable> {
        let mut loader = Self::new();
        
        let yaml_content = fs::read_to_string(path.as_ref())
            .map_err(|e| SessionError::InternalError(
                format!("Failed to read YAML file: {}", e)
            ))?;
        
        loader.load_from_string(&yaml_content)?;
        loader.build()
    }
    
    /// Load state table from a string
    pub fn load_from_string(&mut self, yaml_content: &str) -> Result<()> {
        let yaml_data: YamlStateTable = serde_yaml::from_str(yaml_content)
            .map_err(|e| SessionError::InternalError(
                format!("Failed to parse YAML: {}", e)
            ))?;
        
        // Validate version - accept both 1.x and 2.x versions
        if !yaml_data.version.starts_with("1.") && !yaml_data.version.starts_with("2.") {
            return Err(SessionError::InternalError(
                format!("Unsupported state table version: {} (expected 1.x or 2.x)", yaml_data.version)
            ));
        }
        
        info!("Loaded state table version {} with {} transitions", 
              yaml_data.version, yaml_data.transitions.len());
        
        self.yaml_data = Some(yaml_data);
        Ok(())
    }
    
    /// Merge another YAML file into the current table
    pub fn merge_file<P: AsRef<Path>>(&mut self, path: P) -> Result<&mut Self> {
        let yaml_content = fs::read_to_string(path.as_ref())
            .map_err(|e| SessionError::InternalError(
                format!("Failed to read YAML file for merge: {}", e)
            ))?;
        
        self.merge_string(&yaml_content)?;
        Ok(self)
    }
    
    /// Merge YAML content into the current table
    pub fn merge_string(&mut self, yaml_content: &str) -> Result<()> {
        let merge_data: YamlStateTable = serde_yaml::from_str(yaml_content)
            .map_err(|e| SessionError::InternalError(
                format!("Failed to parse YAML for merge: {}", e)
            ))?;
        
        if let Some(ref mut yaml_data) = self.yaml_data {
            let num_transitions = merge_data.transitions.len();
            // Merge transitions
            yaml_data.transitions.extend(merge_data.transitions);
            
            // Merge states (avoiding duplicates)
            for state in merge_data.states {
                if !yaml_data.states.iter().any(|s| s.name == state.name) {
                    yaml_data.states.push(state);
                }
            }
            
            // Merge conditions (avoiding duplicates)
            for condition in merge_data.conditions {
                if !yaml_data.conditions.iter().any(|c| c.name == condition.name) {
                    yaml_data.conditions.push(condition);
                }
            }
            
            info!("Merged {} transitions into state table", num_transitions);
        } else {
            self.yaml_data = Some(merge_data);
        }
        
        Ok(())
    }
    
    /// Build the final state table from loaded YAML
    pub fn build(mut self) -> Result<StateTable> {
        let yaml_data = self.yaml_data.take()
            .ok_or_else(|| SessionError::InternalError(
                "No YAML data loaded".to_string()
            ))?;
        
        // Process each transition
        for yaml_transition in yaml_data.transitions {
            match self.convert_transition(yaml_transition) {
                Ok((key, transition)) => {
                    // Normal transition
                    self.builder.add_raw_transition(key, transition);
                }
                Err(SessionError::InternalError(msg)) if msg.starts_with("WILDCARD_TRANSITION:") => {
                    // Parse wildcard transition data
                    let parts: Vec<&str> = msg.strip_prefix("WILDCARD_TRANSITION:").unwrap().split(':').collect();
                    if parts.len() == 3 {
                        // Deserialize the components
                        if let (Ok(role), Ok(event), Ok(transition)) = (
                            serde_json::from_str::<Role>(parts[0]),
                            serde_json::from_str::<EventType>(parts[1]),
                            serde_json::from_str::<Transition>(parts[2])
                        ) {
                            // Add wildcard transition
                            self.builder.add_wildcard_transition(role, event, transition);
                        } else {
                            tracing::warn!("Failed to parse wildcard transition data");
                        }
                    }
                }
                Err(e) => return Err(e),
            }
        }
        
        Ok(self.builder.build())
    }
    
    /// Convert a YAML transition to internal format
    /// Returns a special error for wildcard transitions
    fn convert_transition(&self, yaml: YamlTransition) -> Result<(StateKey, Transition)> {
        // Convert role
        let role = match yaml.role.to_lowercase().as_str() {
            "uac" => Role::UAC,
            "uas" => Role::UAS,
            "both" => Role::Both,
            "server" => Role::UAS,  // Accept Server as alias for UAS
            _ => return Err(SessionError::InternalError(
                format!("Invalid role: {}", yaml.role)
            )),
        };
        
        // Check if this is a wildcard state
        let is_wildcard = yaml.state == "Any" || yaml.state == "*";
        
        // Convert state (use Idle as placeholder for wildcards)
        let state = if is_wildcard {
            CallState::Idle // Placeholder, won't be used
        } else {
            self.parse_call_state(&yaml.state)?
        };
        
        // Convert event
        let event = self.parse_event(yaml.event)?;
        
        // Create state key
        let key = StateKey { role, state, event: event.clone() };
        
        // Convert guards
        let guards = yaml.guards.into_iter()
            .map(|g| self.parse_guard(g))
            .collect::<Result<Vec<_>>>()?;
        
        // Convert actions
        let actions = yaml.actions.into_iter()
            .map(|a| self.parse_action(a))
            .collect::<Result<Vec<_>>>()?;
        
        // Convert next state
        let next_state = yaml.next_state
            .map(|s| self.parse_call_state(&s))
            .transpose()?;
        
        // Convert condition updates
        let condition_updates = ConditionUpdates {
            dialog_established: yaml.conditions.dialog_established,
            media_session_ready: yaml.conditions.media_session_ready,
            sdp_negotiated: yaml.conditions.sdp_negotiated,
        };
        
        // Convert publish events
        let publish_events = yaml.publish.into_iter()
            .map(|e| self.parse_event_template(&e))
            .collect::<Result<Vec<_>>>()?;
        
        // Create transition
        let transition = Transition {
            guards,
            actions,
            next_state,
            condition_updates,
            publish_events,
        };
        
        // If this is a wildcard, return a special error that includes the transition data
        if is_wildcard {
            // We'll use a special error to signal wildcard transitions
            return Err(SessionError::InternalError(
                format!("WILDCARD_TRANSITION:{}:{}:{}", 
                    serde_json::to_string(&role).unwrap_or_default(),
                    serde_json::to_string(&event).unwrap_or_default(),
                    serde_json::to_string(&transition).unwrap_or_default()
                )
            ));
        }
        
        Ok((key, transition))
    }
    
    /// Parse a call state from string
    fn parse_call_state(&self, state: &str) -> Result<CallState> {
        match state {
            "Idle" => Ok(CallState::Idle),
            "Initiating" => Ok(CallState::Initiating),
            "Ringing" => Ok(CallState::Ringing),
            "Answering" => Ok(CallState::Answering),
            "EarlyMedia" => Ok(CallState::EarlyMedia),
            "Active" => Ok(CallState::Active),
            "OnHold" => Ok(CallState::OnHold),
            "Resuming" => Ok(CallState::Resuming),
            "Bridged" => Ok(CallState::Bridged),
            "Transferring" => Ok(CallState::Transferring),
            "TransferringCall" => Ok(CallState::TransferringCall),
            "Terminating" => Ok(CallState::Terminating),
            "Terminated" => Ok(CallState::Terminated),
            "Muted" => Ok(CallState::Muted),
            "ConsultationCall" => Ok(CallState::ConsultationCall),
            "Cancelled" => Ok(CallState::Cancelled),
            
            // Registration states
            "Registering" => Ok(CallState::Registering),
            "Registered" => Ok(CallState::Registered),
            "Unregistering" => Ok(CallState::Unregistering),
            
            // Subscription/Presence states
            "Subscribing" => Ok(CallState::Subscribing),
            "Subscribed" => Ok(CallState::Subscribed),
            "Publishing" => Ok(CallState::Publishing),
            
            // Authentication and routing states
            "Authenticating" => Ok(CallState::Authenticating),
            "Messaging" => Ok(CallState::Messaging),
            
            _ if state.starts_with("Failed") => {
                // Parse Failed(reason) states
                Ok(CallState::Failed(FailureReason::Other))
            }
            _ => Err(SessionError::InternalError(
                format!("Invalid call state: {}", state)
            )),
        }
    }
    
    /// Parse an event from YAML representation
    fn parse_event(&self, event: YamlEvent) -> Result<EventType> {
        match event {
            YamlEvent::Simple(name) => self.parse_event_by_name(&name),
            YamlEvent::Complex { event_type, parameters } => {
                // Handle complex events with parameters
                match event_type.as_str() {
                    "MakeCall" => {
                        let target = parameters.get("target")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(EventType::MakeCall { target })
                    }
                    "IncomingCall" => {
                        let from = parameters.get("from")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let sdp = parameters.get("sdp")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        Ok(EventType::IncomingCall { from, sdp })
                    }
                    _ => self.parse_event_by_name(&event_type),
                }
            }
        }
    }
    
    /// Parse an event by name
    fn parse_event_by_name(&self, name: &str) -> Result<EventType> {
        match name {
            // Application events
            "MakeCall" => Ok(EventType::MakeCall { target: String::new() }),
            "AcceptCall" => Ok(EventType::AcceptCall),
            "RejectCall" => Ok(EventType::RejectCall { reason: String::new() }),
            "HangupCall" => Ok(EventType::HangupCall),
            "HoldCall" => Ok(EventType::HoldCall),
            "ResumeCall" => Ok(EventType::ResumeCall),
            
            // Dialog events (abstracted)
            "DialogProgress" | "Dialog180Ringing" => Ok(EventType::Dialog180Ringing),
            "Dialog183SessionProgress" => Ok(EventType::Dialog183SessionProgress),
            "DialogEstablished" | "Dialog200OK" => Ok(EventType::Dialog200OK),
            "DialogFailed" => Ok(EventType::Dialog4xxFailure(400)),
            "DialogTerminated" => Ok(EventType::DialogBYE),
            
            // Gateway-specific BYE events
            "InboundBYE" | "OutboundBYE" => Ok(EventType::DialogBYE),
            "IncomingCall" => Ok(EventType::IncomingCall { 
                from: String::new(), 
                sdp: None 
            }),
            
            // Media events
            "MediaReady" => Ok(EventType::MediaEvent("media_session_created".to_string())),
            "MediaFlowing" => Ok(EventType::MediaEvent("media_flow_established".to_string())),
            "MediaFailed" => Ok(EventType::MediaEvent("media_failed".to_string())),
            "SDPNegotiated" => Ok(EventType::MediaEvent("sdp_negotiated".to_string())),
            
            // Internal coordination
            "CheckReadiness" => Ok(EventType::CheckConditions),
            "PublishEstablished" => Ok(EventType::PublishCallEstablished),
            
            // Bridge events
            "BridgeToSession" | "BridgeSessions" => Ok(EventType::BridgeSessions { 
                other_session: SessionId::new() 
            }),
            
            // Transfer events
            // "BlindTransfer" event removed
            "TransferRequested" => Ok(EventType::TransferRequested {
                refer_to: String::new(),
                transfer_type: String::new(),
                transaction_id: String::new(),
            }),
            // "TransferComplete" event removed

            // Internal transfer coordination events
            "InternalProceedWithTransfer" => Ok(EventType::InternalProceedWithTransfer),
            "InternalMakeTransferCall" => Ok(EventType::InternalMakeTransferCall),
            "InternalTransferCallEstablished" => Ok(EventType::InternalTransferCallEstablished),

            // Registration events
            "StartRegistration" => Ok(EventType::StartRegistration),
            "Registration200OK" => Ok(EventType::Registration200OK),
            "RegistrationFailed" => Ok(EventType::RegistrationFailed(0)),
            "UnregisterRequest" => Ok(EventType::UnregisterRequest),
            "RegistrationExpired" => Ok(EventType::RegistrationExpired),

            // Subscription events
            "StartSubscription" => Ok(EventType::StartSubscription),
            "ReceiveNOTIFY" => Ok(EventType::ReceiveNOTIFY),
            "SendNOTIFY" => Ok(EventType::SendNOTIFY),
            "SubscriptionAccepted" => Ok(EventType::SubscriptionAccepted),
            "SubscriptionFailed" => Ok(EventType::SubscriptionFailed(0)),
            "SubscriptionExpired" => Ok(EventType::SubscriptionExpired),
            "UnsubscribeRequest" => Ok(EventType::UnsubscribeRequest),

            // Message events
            "SendMessage" => Ok(EventType::SendMessage),
            "ReceiveMESSAGE" => Ok(EventType::ReceiveMESSAGE),
            "MessageDelivered" => Ok(EventType::MessageDelivered),
            "MessageFailed" => Ok(EventType::MessageFailed(0)),

            // Default: treat as media event
            _ => Ok(EventType::MediaEvent(name.to_string())),
        }
    }
    
    /// Parse a guard from YAML representation
    fn parse_guard(&self, guard: YamlGuard) -> Result<Guard> {
        match guard {
            YamlGuard::Simple(name) => self.parse_guard_by_name(&name),
            YamlGuard::Complex { guard_type, .. } => {
                self.parse_guard_by_name(&guard_type)
            }
        }
    }
    
    /// Parse a guard by name
    fn parse_guard_by_name(&self, name: &str) -> Result<Guard> {
        match name {
            "HasLocalSDP" => Ok(Guard::HasLocalSDP),
            "HasRemoteSDP" => Ok(Guard::HasRemoteSDP),
            "DialogEstablished" => Ok(Guard::DialogEstablished),
            "MediaReady" => Ok(Guard::MediaReady),
            "SDPNegotiated" => Ok(Guard::SDPNegotiated),
            "AllConditionsMet" | "all_conditions_met" => Ok(Guard::AllConditionsMet),
            "IsIdle" => Ok(Guard::IsIdle),
            "InActiveCall" => Ok(Guard::InActiveCall),
            "IsRegistered" => Ok(Guard::IsRegistered),
            "IsSubscribed" => Ok(Guard::IsSubscribed),
            "HasActiveSubscription" => Ok(Guard::HasActiveSubscription),
            "OtherSessionActive" => Ok(Guard::Custom(name.to_string())),
            _ => {
                debug!("Unknown guard '{}', treating as custom", name);
                Ok(Guard::Custom(name.to_string()))
            }
        }
    }
    
    /// Parse an action from YAML representation
    fn parse_action(&self, action: YamlAction) -> Result<Action> {
        match action {
            YamlAction::Simple(name) => self.parse_action_by_name(&name),
            YamlAction::Complex { action_type, parameters } => {
                // Handle parameterized actions
                match action_type.as_str() {
                    "SendSIPResponse" => {
                        let code = parameters.get("code")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(200) as u16;
                        let reason = parameters.get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("OK")
                            .to_string();
                        Ok(Action::SendSIPResponse(code, reason))
                    }
                    "SetCondition" => {
                        let condition = parameters.get("condition")
                            .and_then(|v| v.as_str())
                            .unwrap_or("dialog_established");
                        let value = parameters.get("value")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);
                        
                        let cond = match condition {
                            "dialog_established" => Condition::DialogEstablished,
                            "media_session_ready" => Condition::MediaSessionReady,
                            "sdp_negotiated" => Condition::SDPNegotiated,
                            _ => return Err(SessionError::InternalError(
                                format!("Invalid condition: {}", condition)
                            )),
                        };
                        
                        Ok(Action::SetCondition(cond, value))
                    }
                    _ => self.parse_action_by_name(&action_type),
                }
            }
        }
    }
    
    /// Parse an action by name
    fn parse_action_by_name(&self, name: &str) -> Result<Action> {
        match name {
            // Dialog actions
            "CreateDialog" => Ok(Action::CreateDialog),
            "GenerateLocalSDP" => Ok(Action::GenerateLocalSDP),
            "SendINVITE" | "TriggerDialogINVITE" => Ok(Action::SendINVITE),
            "SendACK" => Ok(Action::SendACK),
            "SendBYE" => Ok(Action::SendBYE),
            "SendCANCEL" => Ok(Action::SendCANCEL),
            "SendReINVITE" => Ok(Action::SendReINVITE),
            
            // Media actions
            "CreateMediaSession" => Ok(Action::CreateMediaSession),
            "StartMediaSession" => Ok(Action::StartMediaSession),
            "StopMediaSession" | "StopMedia" => Ok(Action::StopMediaSession),
            "NegotiateSDPAsUAC" => Ok(Action::NegotiateSDPAsUAC),
            "NegotiateSDPAsUAS" => Ok(Action::NegotiateSDPAsUAS),
            "SuspendMedia" => Ok(Action::Custom("SuspendMedia".to_string())),
            "ResumeMedia" => Ok(Action::Custom("ResumeMedia".to_string())),
            
            // State updates
            "StoreLocalSDP" => Ok(Action::StoreLocalSDP),
            "StoreRemoteSDP" => Ok(Action::StoreRemoteSDP),
            "StoreNegotiatedConfig" => Ok(Action::StoreNegotiatedConfig),
            
            // Callbacks
            "TriggerCallEstablished" | "PublishEstablished" => Ok(Action::TriggerCallEstablished),
            "TriggerCallTerminated" => Ok(Action::TriggerCallTerminated),
            
            // Cleanup
            "StartDialogCleanup" => Ok(Action::StartDialogCleanup),
            "StartMediaCleanup" => Ok(Action::StartMediaCleanup),
            "CleanupDialog" => Ok(Action::CleanupDialog),
            "CleanupMedia" => Ok(Action::CleanupMedia),

            // Registration actions
            "SendREGISTER" => Ok(Action::SendREGISTER),
            "ProcessRegistrationResponse" => Ok(Action::ProcessRegistrationResponse),

            // Subscription actions
            "SendSUBSCRIBE" => Ok(Action::SendSUBSCRIBE),
            "ProcessNOTIFY" => Ok(Action::ProcessNOTIFY),
            "SendNOTIFY" => Ok(Action::SendNOTIFY),

            // Message actions
            "SendMESSAGE" => Ok(Action::SendMESSAGE),
            "ProcessMESSAGE" => Ok(Action::ProcessMESSAGE),

            // Bridge/Conference
            "CreateMediaBridge" => Ok(Action::Custom("CreateMediaBridge".to_string())),
            "LinkSessions" => Ok(Action::Custom("LinkSessions".to_string())),
            "HoldOriginalCall" | "HoldCurrentCall" => Ok(Action::HoldCurrentCall),
            "ResumeOriginalCall" => Ok(Action::RestoreMediaFlow),

            // Event publishing actions (replace callbacks)
            "SendReferAccepted" => Ok(Action::SendReferAccepted),
            "PublishReferEvent" => Ok(Action::PublishReferEvent),
            "PublishIncomingCallEvent" => Ok(Action::PublishIncomingCallEvent),
            "PublishCallEndedEvent" => Ok(Action::PublishCallEndedEvent),
            "PublishCallAnsweredEvent" => Ok(Action::PublishCallAnsweredEvent),
            "PublishCallOnHoldEvent" => Ok(Action::PublishCallOnHoldEvent),
            "PublishCallResumedEvent" => Ok(Action::PublishCallResumedEvent),
            "PublishDtmfReceivedEvent" => Ok(Action::PublishDtmfReceivedEvent),

            // Internal
            "CheckReadiness" => Ok(Action::Custom("CheckReadiness".to_string())),

            // Default: treat as custom
            _ => {
                debug!("Unknown action '{}', treating as custom", name);
                Ok(Action::Custom(name.to_string()))
            }
        }
    }
    
    /// Parse an event template for publishing
    fn parse_event_template(&self, name: &str) -> Result<EventTemplate> {
        match name {
            "SessionCreated" => Ok(EventTemplate::SessionCreated),
            "StateChanged" => Ok(EventTemplate::StateChanged),
            "CallEstablished" => Ok(EventTemplate::CallEstablished),
            "CallTerminated" => Ok(EventTemplate::CallTerminated),
            "CallFailed" => Ok(EventTemplate::CallFailed),
            "MediaFlowEstablished" => Ok(EventTemplate::MediaFlowEstablished),
            "CallRinging" => Ok(EventTemplate::Custom("CallRinging".to_string())),
            "CallOnHold" => Ok(EventTemplate::Custom("CallOnHold".to_string())),
            "CallResumed" => Ok(EventTemplate::Custom("CallResumed".to_string())),
            "SessionsBridged" => Ok(EventTemplate::Custom("SessionsBridged".to_string())),
            "TransferSucceeded" => Ok(EventTemplate::Custom("TransferSucceeded".to_string())),
            _ => Ok(EventTemplate::Custom(name.to_string())),
        }
    }
}

impl Default for YamlTableLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_simple_yaml() {
        let yaml = r#"
version: "1.0"
transitions:
  - role: UAC
    state: Idle
    event: MakeCall
    next_state: Initiating
    actions:
      - SendINVITE
    publish:
      - SessionCreated
"#;
        
        let mut loader = YamlTableLoader::new();
        loader.load_from_string(yaml).expect("Failed to load YAML");
        let table = loader.build().expect("Failed to build table");
        
        // Verify the transition was added
        let key = StateKey {
            role: Role::UAC,
            state: CallState::Idle,
            event: EventType::MakeCall { target: String::new() },
        };
        
        assert!(table.has_transition(&key));
    }
    
    #[test]
    fn test_complex_event_parsing() {
        let yaml = r#"
version: "1.0"
transitions:
  - role: UAC
    state: Idle
    event:
      type: MakeCall
      target: "sip:bob@example.com"
    next_state: Initiating
"#;
        
        let mut loader = YamlTableLoader::new();
        loader.load_from_string(yaml).expect("Failed to load YAML");
        loader.build().expect("Failed to build table");
    }
    
    #[test]
    fn test_condition_updates() {
        let yaml = r#"
version: "1.0"
transitions:
  - role: Both
    state: Active
    event: CheckReadiness
    conditions:
      dialog_established: true
      media_session_ready: true
      sdp_negotiated: true
"#;
        
        let mut loader = YamlTableLoader::new();
        loader.load_from_string(yaml).expect("Failed to load YAML");
        let table = loader.build().expect("Failed to build table");
        
        let key = StateKey {
            role: Role::Both,
            state: CallState::Active,
            event: EventType::CheckConditions,
        };
        
        let transition = table.get_transition(&key).expect("Transition not found");
        assert!(transition.condition_updates.dialog_established.unwrap_or(false));
    }
}