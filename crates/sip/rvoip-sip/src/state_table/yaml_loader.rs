//! YAML-based state table loader for session coordination
//!
//! This module loads state tables from YAML files, focusing on coordination
//! between dialog-core and media-core layers without duplicating their logic.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tracing::{debug, info};

use super::{
    Action, Condition, ConditionUpdates, EventTemplate, EventType, Guard, Role, SessionId,
    StateKey, StateTable, StateTableBuilder, Transition,
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
        loader
            .load_from_string(DEFAULT_STATE_TABLE_YAML)
            .expect("Embedded default state table must be valid");
        loader.build()
    }

    /// Exact bytes compiled by [`Self::load_embedded_default`].
    ///
    /// This is crate-private so runtime source selection and beta evidence can
    /// hash the authority that was actually selected without adding a public
    /// YAML API or duplicating the embedded file path.
    pub(crate) fn embedded_default_yaml_bytes() -> &'static [u8] {
        DEFAULT_STATE_TABLE_YAML.as_bytes()
    }

    /// Load state table from a file
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<StateTable> {
        let mut loader = Self::new();

        let yaml_content = fs::read_to_string(path.as_ref())
            .map_err(|e| SessionError::InternalError(format!("Failed to read YAML file: {}", e)))?;

        loader.load_from_string(&yaml_content)?;
        loader.build()
    }

    /// Load state table from a string
    pub fn load_from_string(&mut self, yaml_content: &str) -> Result<()> {
        Self::validate_raw_yaml_content(yaml_content)?;

        let yaml_data: YamlStateTable = serde_yaml::from_str(yaml_content)
            .map_err(|e| SessionError::InternalError(format!("Failed to parse YAML: {}", e)))?;

        // Validate version - accept both 1.x and 2.x versions
        if !yaml_data.version.starts_with("1.") && !yaml_data.version.starts_with("2.") {
            return Err(SessionError::InternalError(format!(
                "Unsupported state table version: {} (expected 1.x or 2.x)",
                yaml_data.version
            )));
        }

        info!(
            "Loaded state table version {} with {} transitions",
            yaml_data.version,
            yaml_data.transitions.len()
        );

        self.yaml_data = Some(yaml_data);
        Ok(())
    }

    /// Merge another YAML file into the current table
    pub fn merge_file<P: AsRef<Path>>(&mut self, path: P) -> Result<&mut Self> {
        let yaml_content = fs::read_to_string(path.as_ref()).map_err(|e| {
            SessionError::InternalError(format!("Failed to read YAML file for merge: {}", e))
        })?;

        self.merge_string(&yaml_content)?;
        Ok(self)
    }

    /// Merge YAML content into the current table
    pub fn merge_string(&mut self, yaml_content: &str) -> Result<()> {
        Self::validate_raw_yaml_content(yaml_content)?;

        let merge_data: YamlStateTable = serde_yaml::from_str(yaml_content).map_err(|e| {
            SessionError::InternalError(format!("Failed to parse YAML for merge: {}", e))
        })?;

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
                if !yaml_data
                    .conditions
                    .iter()
                    .any(|c| c.name == condition.name)
                {
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
        let yaml_data = self
            .yaml_data
            .take()
            .ok_or_else(|| SessionError::InternalError("No YAML data loaded".to_string()))?;

        self.validate_yaml_data(&yaml_data)?;

        // Process each transition
        for yaml_transition in yaml_data.transitions {
            match self.convert_transition(yaml_transition) {
                Ok((key, transition)) => {
                    // Normal transition
                    self.builder.add_raw_transition(key, transition);
                }
                Err(SessionError::InternalError(msg))
                    if msg.starts_with("WILDCARD_TRANSITION:") =>
                {
                    // Parse wildcard transition data
                    let parts: Vec<&str> = msg
                        .strip_prefix("WILDCARD_TRANSITION:")
                        .unwrap()
                        .splitn(3, ':')
                        .collect();
                    if parts.len() == 3 {
                        // Deserialize the components
                        if let (Ok(role), Ok(event), Ok(transition)) = (
                            serde_json::from_str::<Role>(parts[0]),
                            serde_json::from_str::<EventType>(parts[1]),
                            serde_json::from_str::<Transition>(parts[2]),
                        ) {
                            // Add wildcard transition
                            self.builder
                                .add_wildcard_transition(role, event, transition);
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

    fn validation_error(errors: Vec<String>) -> SessionError {
        SessionError::InternalError(format!(
            "State table YAML validation failed:\n- {}",
            errors.join("\n- ")
        ))
    }

    fn validate_raw_yaml_content(yaml_content: &str) -> Result<()> {
        let raw: serde_yaml::Value = serde_yaml::from_str(yaml_content)
            .map_err(|e| SessionError::InternalError(format!("Failed to parse YAML: {}", e)))?;

        let mut errors = Vec::new();
        let allowed_condition_updates: HashSet<&str> = [
            "dialog_established",
            "media_session_ready",
            "sdp_negotiated",
        ]
        .into_iter()
        .collect();

        let transitions = raw.get("transitions").and_then(|value| value.as_sequence());

        if let Some(transitions) = transitions {
            for (index, transition) in transitions.iter().enumerate() {
                let Some(mapping) = transition.as_mapping() else {
                    errors.push(format!("transition #{} is not a mapping", index + 1));
                    continue;
                };

                let Some(conditions) = mapping
                    .get(serde_yaml::Value::String("conditions".to_string()))
                    .and_then(|value| value.as_mapping())
                else {
                    continue;
                };

                for key in conditions.keys() {
                    let Some(key) = key.as_str() else {
                        errors.push(format!(
                            "transition #{} has a non-string condition update key",
                            index + 1
                        ));
                        continue;
                    };

                    if !allowed_condition_updates.contains(key) {
                        errors.push(format!(
                            "transition #{} uses unsupported condition update '{}'",
                            index + 1,
                            key
                        ));
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(Self::validation_error(errors))
        }
    }

    fn validate_yaml_data(&self, yaml_data: &YamlStateTable) -> Result<()> {
        let mut errors = Vec::new();
        // Duplicate identity must retain event payloads. `EventType::Debug`
        // deliberately exposes only the variant name, so using its text as a
        // key collapses distinct typed events such as the RFC 4028 internal
        // `MediaEvent` capabilities into one false duplicate.
        let mut seen_transitions: HashMap<(Role, String, EventType), usize> = HashMap::new();

        let declared_states: HashSet<String> = yaml_data
            .states
            .iter()
            .map(|state| state.name.clone())
            .collect();
        let should_validate_declared_states = !declared_states.is_empty();

        for (index, transition) in yaml_data.transitions.iter().enumerate() {
            let line_hint = format!("transition #{}", index + 1);
            let role = match transition.role.to_lowercase().as_str() {
                "uac" => Role::UAC,
                "uas" | "server" => Role::UAS,
                "both" => Role::Both,
                _ => {
                    errors.push(format!(
                        "{} has invalid role '{}'",
                        line_hint, transition.role
                    ));
                    continue;
                }
            };

            if should_validate_declared_states {
                for (field, state) in [
                    ("state", Some(transition.state.as_str())),
                    ("next_state", transition.next_state.as_deref()),
                ] {
                    let Some(state) = state else { continue };
                    if state == "Any" || state == "*" {
                        continue;
                    }
                    if !declared_states.contains(state) {
                        errors.push(format!(
                            "{} references undeclared {} '{}'",
                            line_hint, field, state
                        ));
                    }
                }
            }

            let event = match self.parse_event(transition.event.clone()) {
                Ok(event) => event.normalize(),
                Err(err) => {
                    errors.push(format!("{} has invalid event: {}", line_hint, err));
                    continue;
                }
            };
            let event_label = format!("{:?}", event);
            let key = (role, transition.state.clone(), event.clone());
            if let Some(previous) = seen_transitions.insert(key, index + 1) {
                errors.push(format!(
                    "{} duplicates transition #{} for role={:?}, state={}, event={}",
                    line_hint, previous, role, transition.state, event_label
                ));
            }

            for action in &transition.actions {
                if let Err(err) = self.parse_action(action.clone()) {
                    errors.push(format!("{} has invalid action: {}", line_hint, err));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(Self::validation_error(errors))
        }
    }

    /// Convert a YAML transition to internal format
    /// Returns a special error for wildcard transitions
    fn convert_transition(&self, yaml: YamlTransition) -> Result<(StateKey, Transition)> {
        // Convert role
        let role = match yaml.role.to_lowercase().as_str() {
            "uac" => Role::UAC,
            "uas" => Role::UAS,
            "both" => Role::Both,
            "server" => Role::UAS, // Accept Server as alias for UAS
            _ => {
                return Err(SessionError::InternalError(format!(
                    "Invalid role: {}",
                    yaml.role
                )))
            }
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
        let key = StateKey {
            role,
            state,
            event: event.clone(),
        };

        // Convert guards
        let guards = yaml
            .guards
            .into_iter()
            .map(|g| self.parse_guard(g))
            .collect::<Result<Vec<_>>>()?;

        // Convert actions
        let actions = yaml
            .actions
            .into_iter()
            .map(|a| self.parse_action(a))
            .collect::<Result<Vec<_>>>()?;

        // Convert next state
        let next_state = yaml
            .next_state
            .map(|s| self.parse_call_state(&s))
            .transpose()?;

        // Convert condition updates
        let condition_updates = ConditionUpdates {
            dialog_established: yaml.conditions.dialog_established,
            media_session_ready: yaml.conditions.media_session_ready,
            sdp_negotiated: yaml.conditions.sdp_negotiated,
        };

        // Convert publish events
        let publish_events = yaml
            .publish
            .into_iter()
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
            return Err(SessionError::InternalError(format!(
                "WILDCARD_TRANSITION:{}:{}:{}",
                serde_json::to_string(&role).unwrap_or_default(),
                serde_json::to_string(&event).unwrap_or_default(),
                serde_json::to_string(&transition).unwrap_or_default()
            )));
        }

        Ok((key, transition))
    }

    /// Parse a call state from string
    fn parse_call_state(&self, state: &str) -> Result<CallState> {
        match state {
            "Idle" => Ok(CallState::Idle),
            "Initiating" => Ok(CallState::Initiating),
            "CancelPending" => Ok(CallState::CancelPending),
            "Cancelling" => Ok(CallState::Cancelling),
            "Ringing" => Ok(CallState::Ringing),
            "Answering" => Ok(CallState::Answering),
            "AnsweringHangupPending" => Ok(CallState::AnsweringHangupPending),
            "EarlyMedia" => Ok(CallState::EarlyMedia),
            "Active" => Ok(CallState::Active),
            "HoldPending" => Ok(CallState::HoldPending),
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
            _ => Err(SessionError::InternalError(format!(
                "Invalid call state: {}",
                state
            ))),
        }
    }

    /// Parse an event from YAML representation
    fn parse_event(&self, event: YamlEvent) -> Result<EventType> {
        match event {
            YamlEvent::Simple(name) => self.parse_event_by_name(&name),
            YamlEvent::Complex {
                event_type,
                parameters,
            } => {
                // Handle complex events with parameters
                match event_type.as_str() {
                    "MakeCall" => {
                        let target = parameters
                            .get("target")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(EventType::MakeCall { target })
                    }
                    "IncomingCall" | "IncomingCallAutoAccept" => {
                        let from = parameters
                            .get("from")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let sdp = parameters
                            .get("sdp")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        if event_type == "IncomingCallAutoAccept" {
                            Ok(EventType::IncomingCallAutoAccept { from, sdp })
                        } else {
                            Ok(EventType::IncomingCall { from, sdp })
                        }
                    }
                    "SendEarlyMedia" => {
                        let sdp = parameters
                            .get("sdp")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        Ok(EventType::SendEarlyMedia { sdp })
                    }
                    "AuthRequired" => {
                        let status_code = parameters
                            .get("status_code")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0) as u16;
                        let challenge = parameters
                            .get("challenge")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let method = parameters
                            .get("method")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(EventType::AuthRequired {
                            status_code,
                            challenge,
                            method,
                        })
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
            "MakeCall" => Ok(EventType::MakeCall {
                target: String::new(),
            }),
            "AcceptCall" => Ok(EventType::AcceptCall),
            "RejectCall" => Ok(EventType::RejectCall {
                status: 0,
                reason: String::new(),
            }),
            "RedirectCall" => Ok(EventType::RedirectCall {
                status: 0,
                contacts: Vec::new(),
            }),
            "SendEarlyMedia" => Ok(EventType::SendEarlyMedia { sdp: None }),
            "AuthRequired" => Ok(EventType::AuthRequired {
                status_code: 0,
                challenge: String::new(),
                method: String::new(),
            }),
            // RFC 4028 §6 — 422 Session Interval Too Small. Field-less YAML
            // name maps to a default `min_se_secs: 0`; the runtime event
            // carries the actual floor from dialog-core's parser.
            "SessionIntervalTooSmall" => Ok(EventType::SessionIntervalTooSmall { min_se_secs: 0 }),
            // RFC 3261 §22.2 — backward-compat alias. The dedicated
            // Registration401 path has been retired in favor of the shared
            // AuthRequired event, but externally-authored state tables may
            // still reference the old name.
            "Registration401" => Ok(EventType::AuthRequired {
                status_code: 401,
                challenge: String::new(),
                method: "REGISTER".to_string(),
            }),
            "HangupCall" => Ok(EventType::HangupCall),
            "CancelCall" => Ok(EventType::CancelCall),
            "HoldCall" => Ok(EventType::HoldCall),
            "ResumeCall" => Ok(EventType::ResumeCall),

            // Dialog events (abstracted)
            "DialogProgress" | "Dialog180Ringing" => Ok(EventType::Dialog180Ringing),
            "Dialog183SessionProgress" => Ok(EventType::Dialog183SessionProgress),
            "DialogEstablished" | "Dialog200OK" => Ok(EventType::Dialog200OK),
            "DialogFailed" => Ok(EventType::Dialog4xxFailure(400)),
            "Dialog4xxFailure" => Ok(EventType::Dialog4xxFailure(400)),
            "Dialog5xxFailure" => Ok(EventType::Dialog5xxFailure(500)),
            "Dialog6xxFailure" => Ok(EventType::Dialog6xxFailure(600)),
            "Dialog487RequestTerminated" => Ok(EventType::Dialog487RequestTerminated),
            "Dialog3xxRedirect" => Ok(EventType::Dialog3xxRedirect {
                status: 0,
                targets: Vec::new(),
            }),
            "ReinviteGlare" => Ok(EventType::ReinviteGlare),
            "ReinviteReceived" => Ok(EventType::ReinviteReceived { sdp: None }),
            "UpdateReceived" => Ok(EventType::UpdateReceived { sdp: None }),
            // ACK delivered to UAS — drives the Answering → Active transition
            // that promotes the dialog from early to confirmed. Without this
            // entry the YAML "DialogACK" event falls through to
            // `EventType::MediaEvent("DialogACK")` and the transition never
            // fires.
            "DialogACK" => Ok(EventType::DialogACK),
            "DialogBYE" => Ok(EventType::DialogBYE),
            "DialogCANCEL" => Ok(EventType::DialogCANCEL),
            "DialogTimeout" => Ok(EventType::DialogTimeout),
            "DialogTerminated" => Ok(EventType::DialogTerminated),
            "ConfirmedNegotiationFailure" => Ok(EventType::MediaEvent(
                crate::state_table::types::CONFIRMED_NEGOTIATION_FAILURE_EVENT.to_string(),
            )),

            // Gateway-specific BYE events
            "InboundBYE" | "OutboundBYE" => Ok(EventType::DialogBYE),
            "IncomingCall" => Ok(EventType::IncomingCall {
                from: String::new(),
                sdp: None,
            }),
            "IncomingCallAutoAccept" => Ok(EventType::IncomingCallAutoAccept {
                from: String::new(),
                sdp: None,
            }),

            // Media events
            "MediaReady" => Ok(EventType::MediaEvent("media_session_created".to_string())),
            "MediaFlowing" => Ok(EventType::MediaEvent("media_flow_established".to_string())),
            "MediaFailed" => Ok(EventType::MediaEvent("media_failed".to_string())),
            "SDPNegotiated" => Ok(EventType::MediaEvent("sdp_negotiated".to_string())),
            // Reserved RFC 4028 driver events. Their string form is public-
            // compatible, but the executor rejects them unless accompanied by
            // the matching crate-private exact-session sidecar.
            "InternalSessionRefreshUpdateDue" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_DUE_EVENT.to_string(),
            )),
            "InternalSessionRefreshReinviteDue" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_REINVITE_DUE_EVENT.to_string(),
            )),
            "InternalSessionRefreshUpdateSucceeded" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_UPDATE_OK_EVENT.to_string(),
            )),
            "InternalSessionRefreshUpdateFailed" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_UPDATE_FAILED_EVENT.to_string(),
            )),
            "InternalSessionRefreshReinviteSucceeded" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_REINVITE_OK_EVENT.to_string(),
            )),
            "InternalSessionRefreshReinviteFailed" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_REINVITE_FAILED_EVENT.to_string(),
            )),
            "InternalSessionRefreshPeerExpired" => Ok(EventType::MediaEvent(
                crate::state_machine::executor::SESSION_REFRESH_PEER_EXPIRED_EVENT.to_string(),
            )),

            // Internal coordination
            "CheckReadiness" => Ok(EventType::CheckConditions),
            "PublishEstablished" => Ok(EventType::PublishCallEstablished),

            // Bridge events
            "BridgeToSession" | "BridgeSessions" => Ok(EventType::BridgeSessions {
                other_session: SessionId::new(),
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
            // "Registration401" is aliased above to EventType::AuthRequired
            // (shared event with INVITE auth). Do not re-add the legacy
            // binding here; the alias intentionally takes priority.
            "RetryRegistration" => Ok(EventType::RetryRegistration),
            "RefreshRegistration" => Ok(EventType::RefreshRegistration),
            "RegistrationFailed" => Ok(EventType::RegistrationFailed(0)),
            "StartUnregistration" => Ok(EventType::StartUnregistration),
            "Unregistration200OK" => Ok(EventType::Unregistration200OK),
            "UnregistrationFailed" => Ok(EventType::UnregistrationFailed),
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

            // SIP_API_DESIGN_2 §7.1 — builder-staged outbound events.
            // Each `coord.<verb>(..).send()` dispatches one of these so
            // the YAML row drives `Action::Send<METHOD>WithOptions`.
            "SendOutboundInvite" => Ok(EventType::SendOutboundInvite),
            "SendOutboundReInvite" => Ok(EventType::SendOutboundReInvite),
            "SendOutboundBye" => Ok(EventType::SendOutboundBye),
            "SendOutboundCancel" => Ok(EventType::SendOutboundCancel),
            "SendOutboundRefer" => Ok(EventType::SendOutboundRefer),
            "SendOutboundNotify" => Ok(EventType::SendOutboundNotify),
            "SendOutboundInfo" => Ok(EventType::SendOutboundInfo),
            "SendOutboundUpdate" => Ok(EventType::SendOutboundUpdate),
            "SendOutboundMessage" => Ok(EventType::SendOutboundMessage),
            "SendOutboundOptions" => Ok(EventType::SendOutboundOptions),
            "SendOutboundSubscribe" => Ok(EventType::SendOutboundSubscribe),
            "SendOutboundRegister" => Ok(EventType::SendOutboundRegister),

            _ => Err(SessionError::InternalError(format!(
                "Unknown YAML event '{}': add a matching arm in \
                 state_table/yaml_loader.rs::parse_event_by_name or remove \
                 the YAML reference.",
                name
            ))),
        }
    }

    /// Parse a guard from YAML representation
    fn parse_guard(&self, guard: YamlGuard) -> Result<Guard> {
        match guard {
            YamlGuard::Simple(name) => self.parse_guard_by_name(&name),
            YamlGuard::Complex { guard_type, .. } => self.parse_guard_by_name(&guard_type),
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
            "HasPendingReinvite" => Ok(Guard::HasPendingReinvite),
            "HasPendingOfferAnswer" => Ok(Guard::Custom(
                crate::state_table::types::HAS_PENDING_OFFER_ANSWER_GUARD.to_string(),
            )),
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
            YamlAction::Complex {
                action_type,
                parameters,
            } => {
                // Handle parameterized actions
                match action_type.as_str() {
                    "SendSIPResponse" => {
                        let code = parameters
                            .get("code")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(200) as u16;
                        let reason = parameters
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("OK")
                            .to_string();
                        Ok(Action::SendSIPResponse(code, reason))
                    }
                    "SetCondition" => {
                        let condition = parameters
                            .get("condition")
                            .and_then(|v| v.as_str())
                            .unwrap_or("dialog_established");
                        let value = parameters
                            .get("value")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        let cond = match condition {
                            "dialog_established" => Condition::DialogEstablished,
                            "media_session_ready" => Condition::MediaSessionReady,
                            "sdp_negotiated" => Condition::SDPNegotiated,
                            _ => {
                                return Err(SessionError::InternalError(format!(
                                    "Invalid condition: {}",
                                    condition
                                )))
                            }
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
            "SendRejectResponse" => Ok(Action::SendRejectResponse),
            "SendRedirectResponse" => Ok(Action::SendRedirectResponse),
            "RetryWithContact" => Ok(Action::RetryWithContact),
            "ScheduleReinviteRetry" => Ok(Action::ScheduleReinviteRetry),
            "ClearPendingReinvite" => Ok(Action::ClearPendingReinvite),
            // SendCANCEL legacy variant deleted per Phase 5 — YAML now
            // emits SendCANCELWithOptions exclusively. Keep an alias so
            // historical YAML still parses for the duration of the
            // deprecation cycle.
            "SendCANCEL" | "SendCANCELWithOptions" => Ok(Action::SendCANCELWithOptions),
            "SendReINVITE" => Ok(Action::SendReINVITE),

            // Media actions
            "CreateMediaSession" => Ok(Action::CreateMediaSession),
            "StartMediaSession" => Ok(Action::StartMediaSession),
            // StopMediaSession/StopMedia aliases map to CleanupMedia — the
            // two used to be distinct but StopMediaSession was broken (see
            // MediaAdapter history), so they're unified now.
            "StopMediaSession" | "StopMedia" => Ok(Action::CleanupMedia),
            "NegotiateSDPAsUAC" => Ok(Action::NegotiateSDPAsUAC),
            "NegotiateSDPAsUAS" => Ok(Action::NegotiateSDPAsUAS),
            "PrepareEarlyMediaSDP" => Ok(Action::PrepareEarlyMediaSDP),
            "SwitchToPassThroughOnActive" => Ok(Action::SwitchToPassThroughOnActive),
            "StoreAuthChallenge" => Ok(Action::StoreAuthChallenge),
            "SendINVITEWithAuth" => Ok(Action::SendINVITEWithAuth),
            "SendINVITEWithBumpedSessionExpires" => Ok(Action::SendINVITEWithBumpedSessionExpires),
            "SendREGISTERWithAuth" => Ok(Action::SendREGISTERWithAuth),
            "SendRequestWithAuth" => Ok(Action::SendRequestWithAuth),
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
            "SendUnREGISTER" | "SendREGISTERWithExpires0" => Ok(Action::SendUnREGISTER),
            "ProcessRegistrationResponse" => Ok(Action::ProcessRegistrationResponse),

            // Subscription actions
            "SendSUBSCRIBE" => Ok(Action::SendSUBSCRIBE),
            "ProcessNOTIFY" => Ok(Action::ProcessNOTIFY),
            // SendNOTIFY legacy variant deleted per Phase 5; alias kept
            // for the deprecation cycle so historical YAML parses.
            "SendNOTIFY" | "SendNOTIFYWithOptions" => Ok(Action::SendNOTIFYWithOptions),

            // Message actions
            "SendMESSAGE" => Ok(Action::SendMESSAGE),
            "ProcessMESSAGE" => Ok(Action::ProcessMESSAGE),

            // Bridge/Conference helpers that are still real state-machine
            // actions. Media bridging itself is direct-wired through the
            // coordinator/media adapter and must not appear in YAML as a
            // Custom no-op.
            "HoldOriginalCall" | "HoldCurrentCall" => Ok(Action::HoldCurrentCall),
            "ResumeOriginalCall" => Ok(Action::RestoreMediaFlow),

            // REFER response action (keep for proper REFER handling)
            "SendReferAccepted" => Ok(Action::SendReferAccepted),

            // RFC 3515 §2.4.5 progress NOTIFYs.
            "SendRefer100Trying" => Ok(Action::SendRefer100Trying),
            "SendTransferNotifyRinging" => Ok(Action::SendTransferNotifyRinging),
            "SendTransferNotifySuccess" => Ok(Action::SendTransferNotifySuccess),
            "SendTransferNotifyFailure" => Ok(Action::SendTransferNotifyFailure),

            // Internal
            "CheckReadiness" => Ok(Action::Custom("CheckReadiness".to_string())),
            "ArmSessionRefreshTimer" => Ok(Action::Custom("ArmSessionRefreshTimer".to_string())),
            "PrepareSessionRefreshUpdate" => {
                Ok(Action::Custom("PrepareSessionRefreshUpdate".to_string()))
            }
            "PrepareSessionRefreshReinvite" => {
                Ok(Action::Custom("PrepareSessionRefreshReinvite".to_string()))
            }
            "PrepareSessionRefreshExpiry" => {
                Ok(Action::Custom("PrepareSessionRefreshExpiry".to_string()))
            }

            // SIP_API_DESIGN_2 §7.1 — unified outbound dispatch through
            // the option stash. Builder `.send()` stages
            // `pending_<method>_options` and queues
            // `EventType::SendOutbound<METHOD>`; the YAML transition row
            // emits `Send<METHOD>WithOptions` which reads the stash.
            "SendINVITEWithOptions" => Ok(Action::SendINVITEWithOptions),
            "SendReINVITEWithOptions" => Ok(Action::SendReINVITEWithOptions),
            "SendREGISTERWithOptions" => Ok(Action::SendREGISTERWithOptions),
            "SendSUBSCRIBEWithOptions" => Ok(Action::SendSUBSCRIBEWithOptions),
            "SendMESSAGEWithOptions" => Ok(Action::SendMESSAGEWithOptions),
            // SendNOTIFYWithOptions/SendCANCELWithOptions handled by
            // their legacy-alias arms above (Phase 5 consolidation).
            "SendBYEWithOptions" => Ok(Action::SendBYEWithOptions),
            "SendREFERWithOptions" => Ok(Action::SendREFERWithOptions),
            "SendINFOWithOptions" => Ok(Action::SendINFOWithOptions),
            "SendUPDATEWithOptions" => Ok(Action::SendUPDATEWithOptions),
            "SendOPTIONSWithOptions" => Ok(Action::SendOPTIONSWithOptions),

            // §7.3 invariant #2 — clear the stash on final-response
            // transitions (200 / 4xx / 5xx / 6xx / timeout).
            "ClearPendingINVITEOptions" => Ok(Action::ClearPendingINVITEOptions),
            "ClearPendingReINVITEOptions" => Ok(Action::ClearPendingReINVITEOptions),
            "ClearPendingREGISTEROptions" => Ok(Action::ClearPendingREGISTEROptions),
            "ClearPendingSUBSCRIBEOptions" => Ok(Action::ClearPendingSUBSCRIBEOptions),
            "ClearPendingMESSAGEOptions" => Ok(Action::ClearPendingMESSAGEOptions),
            "ClearPendingNOTIFYOptions" => Ok(Action::ClearPendingNOTIFYOptions),
            "ClearPendingBYEOptions" => Ok(Action::ClearPendingBYEOptions),
            "ClearPendingCANCELOptions" => Ok(Action::ClearPendingCANCELOptions),
            "ClearPendingREFEROptions" => Ok(Action::ClearPendingREFEROptions),
            "ClearPendingINFOOptions" => Ok(Action::ClearPendingINFOOptions),
            "ClearPendingUPDATEOptions" => Ok(Action::ClearPendingUPDATEOptions),
            "ClearPendingOPTIONSOptions" => Ok(Action::ClearPendingOPTIONSOptions),

            // Unknown action — drift detection. Previously silently fell through
            // to `Action::Custom(name)`, which masked dead YAML entries pointing
            // at long-removed action variants. Now a hard error so additions
            // and deletions stay synchronized between the YAML and the Rust
            // `Action` enum. Intentional custom hooks (e.g. "SuspendMedia",
            // "ResumeMedia", "CheckReadiness") must be listed explicitly above.
            _ => Err(SessionError::InternalError(format!(
                "Unknown YAML action '{}': add a matching arm in \
                 state_table/yaml_loader.rs::parse_action_by_name or remove \
                 the YAML reference.",
                name
            ))),
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
            "CallCancelled" => Ok(EventTemplate::CallCancelled),
            "MediaFlowEstablished" => Ok(EventTemplate::MediaFlowEstablished),
            "CallRinging" => Ok(EventTemplate::Custom("CallRinging".to_string())),
            "CallOnHold" => Ok(EventTemplate::CallOnHold),
            "CallResumed" => Ok(EventTemplate::CallResumed),
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
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug)]
    struct VariantAllowance {
        variant: &'static str,
        owner: &'static str,
    }

    // These inventories are deliberately exact. A newly added Rust variant is
    // unaccounted until it is either wired into default.yaml or added here with
    // an ownership reason. Conversely, using an allowlisted variant in
    // default.yaml fails until its allowance is removed, so these lists cannot
    // become catch-all exemptions.
    const EVENT_VARIANT_ALLOWANCES: &[VariantAllowance] = &[
        VariantAllowance {
            variant: "MuteCall",
            owner: "public: callable media-control input without an embedded row",
        },
        VariantAllowance {
            variant: "UnmuteCall",
            owner: "public: callable media-control input without an embedded row",
        },
        VariantAllowance {
            variant: "PlayAudio",
            owner: "public: callable media-control input without an embedded row",
        },
        VariantAllowance {
            variant: "StartRecording",
            owner: "public: callable recording input without an embedded row",
        },
        VariantAllowance {
            variant: "StopRecording",
            owner: "public: callable recording input without an embedded row",
        },
        VariantAllowance {
            variant: "DialogCreated",
            owner: "direct: typed dialog ingress records correlation before lifecycle routing",
        },
        VariantAllowance {
            variant: "CallEstablished",
            owner: "public-serde: programmatic-table/history compatibility event; typed dialog establishment routes as Dialog200OK",
        },
        VariantAllowance {
            variant: "DialogInvite",
            owner: "public-serde: programmatic-table compatibility event; typed inbound INVITE routes as IncomingCall",
        },
        VariantAllowance {
            variant: "DialogREFER",
            owner: "public-serde: programmatic-table compatibility event; typed REFER ingress routes as TransferRequested",
        },
        VariantAllowance {
            variant: "DialogReINVITE",
            owner: "public-serde: programmatic-table compatibility event; typed re-INVITE ingress routes as ReinviteReceived",
        },
        VariantAllowance {
            variant: "DialogError",
            owner: "direct: typed dialog error carries runtime-only detail",
        },
        VariantAllowance {
            variant: "DialogStateChanged",
            owner: "direct: typed dialog observation is not a lifecycle trigger",
        },
        VariantAllowance {
            variant: "MediaSessionCreated",
            owner: "direct: typed media ingress is normalized before table lookup",
        },
        VariantAllowance {
            variant: "MediaSessionReady",
            owner: "direct: typed media ingress is normalized before table lookup",
        },
        VariantAllowance {
            variant: "MediaNegotiated",
            owner: "direct: typed media ingress is normalized before table lookup",
        },
        VariantAllowance {
            variant: "MediaFlowEstablished",
            owner: "direct: typed media ingress is normalized before table lookup",
        },
        VariantAllowance {
            variant: "MediaError",
            owner: "direct: typed media error carries runtime-only detail",
        },
        VariantAllowance {
            variant: "MediaQualityDegraded",
            owner: "direct: media telemetry is observational, not lifecycle control",
        },
        VariantAllowance {
            variant: "DtmfDetected",
            owner: "direct: media telemetry is observational, not lifecycle control",
        },
        VariantAllowance {
            variant: "RtpTimeout",
            owner: "direct: media watchdog dispatches an exact lifecycle event instead",
        },
        VariantAllowance {
            variant: "PacketLossThresholdExceeded",
            owner: "direct: media telemetry is observational, not lifecycle control",
        },
        VariantAllowance {
            variant: "InternalCheckReady",
            owner: "internal: executor-owned follow-up event",
        },
        VariantAllowance {
            variant: "InternalACKSent",
            owner: "public-serde: programmatic-table compatibility event; live ACK ingress routes as DialogACK",
        },
        VariantAllowance {
            variant: "InternalUASMedia",
            owner: "public-serde: programmatic-table compatibility event; live UAS media readiness uses normalized media ingress",
        },
        VariantAllowance {
            variant: "InternalCleanupComplete",
            owner: "public-serde: programmatic-table compatibility event; exact lifecycle release owns cleanup completion",
        },
        VariantAllowance {
            variant: "CheckConditions",
            owner: "runtime-yaml: CheckReadiness alias remains available to custom tables",
        },
        VariantAllowance {
            variant: "PublishCallEstablished",
            owner: "runtime-yaml: PublishEstablished alias remains available to custom tables",
        },
        VariantAllowance {
            variant: "CreateConference",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "AddParticipant",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "JoinConference",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "LeaveConference",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "MuteInConference",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "UnmuteInConference",
            owner: "public: conference input is outside the embedded SIP call profile",
        },
        VariantAllowance {
            variant: "BridgeSessions",
            owner: "direct: server bridge API owns RTP bridging",
        },
        VariantAllowance {
            variant: "UnbridgeSessions",
            owner: "direct: server bridge API owns RTP unbridging",
        },
        VariantAllowance {
            variant: "ModifySession",
            owner: "public: generic extension input has no embedded transition",
        },
        VariantAllowance {
            variant: "Registration401",
            owner: "runtime-yaml/public-serde: legacy name remains accepted and normalizes to AuthRequired for REGISTER",
        },
        VariantAllowance {
            variant: "RetryRegistration",
            owner: "runtime-yaml: registration retry remains available to custom tables",
        },
        VariantAllowance {
            variant: "UnregisterRequest",
            owner: "public: builder input is normalized to the embedded unregister flow",
        },
        VariantAllowance {
            variant: "RegistrationExpired",
            owner: "internal: registration lifecycle timer input",
        },
        VariantAllowance {
            variant: "StartSubscription",
            owner: "direct: standalone SUBSCRIBE is dialog/transaction-owned",
        },
        VariantAllowance {
            variant: "SendNOTIFY",
            owner: "public: session-scoped NOTIFY input uses staged outbound dispatch",
        },
        VariantAllowance {
            variant: "SubscriptionAccepted",
            owner: "direct: standalone subscription response is transaction-owned",
        },
        VariantAllowance {
            variant: "SubscriptionFailed",
            owner: "direct: standalone subscription response is transaction-owned",
        },
        VariantAllowance {
            variant: "SubscriptionExpired",
            owner: "direct: standalone subscription timer is transaction-owned",
        },
        VariantAllowance {
            variant: "UnsubscribeRequest",
            owner: "direct: standalone unsubscribe is dialog/transaction-owned",
        },
        VariantAllowance {
            variant: "SendMessage",
            owner: "direct: standalone MESSAGE is dialog/transaction-owned",
        },
        VariantAllowance {
            variant: "ReceiveMESSAGE",
            owner: "direct: standalone MESSAGE delivery is not session lifecycle control",
        },
        VariantAllowance {
            variant: "MessageDelivered",
            owner: "direct: standalone MESSAGE response is transaction-owned",
        },
        VariantAllowance {
            variant: "MessageFailed",
            owner: "direct: standalone MESSAGE response is transaction-owned",
        },
        VariantAllowance {
            variant: "CleanupComplete",
            owner: "public-serde: programmatic-table compatibility event; exact lifecycle release owns cleanup completion",
        },
        VariantAllowance {
            variant: "Reset",
            owner: "public: programmatic state-table reset input",
        },
        VariantAllowance {
            variant: "InternalProceedWithTransfer",
            owner: "internal: executor-owned transfer follow-up",
        },
        VariantAllowance {
            variant: "InternalMakeTransferCall",
            owner: "internal: executor-owned transfer follow-up",
        },
        VariantAllowance {
            variant: "InternalTransferCallEstablished",
            owner: "internal: executor-owned transfer follow-up",
        },
        VariantAllowance {
            variant: "SendOutboundMessage",
            owner: "direct: standalone MESSAGE deliberately bypasses session YAML",
        },
        VariantAllowance {
            variant: "SendOutboundOptions",
            owner: "direct: standalone OPTIONS deliberately bypasses session YAML",
        },
        VariantAllowance {
            variant: "SendOutboundSubscribe",
            owner: "direct: standalone SUBSCRIBE deliberately bypasses session YAML",
        },
    ];

    const GUARD_VARIANT_ALLOWANCES: &[VariantAllowance] = &[
        VariantAllowance {
            variant: "HasLocalSDP",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "HasRemoteSDP",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "HasNegotiatedConfig",
            owner: "direct: programmatic StateTableBuilder guard",
        },
        VariantAllowance {
            variant: "AllConditionsMet",
            owner: "runtime-yaml: readiness guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "DialogEstablished",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "MediaReady",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "SDPNegotiated",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "IsIdle",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "InActiveCall",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "IsRegistered",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "IsSubscribed",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "HasActiveSubscription",
            owner: "runtime-yaml: supported guard for externally supplied tables",
        },
        VariantAllowance {
            variant: "HasPendingReinvite",
            owner: "direct: retained compatibility guard for builder-owned re-INVITE state",
        },
    ];

    const ACTION_VARIANT_ALLOWANCES: &[VariantAllowance] = &[
        VariantAllowance {
            variant: "SendREGISTER",
            owner: "runtime-yaml: legacy initial/refresh facade over the canonical retained REGISTER options action",
        },
        VariantAllowance {
            variant: "SendREGISTERWithAuth",
            owner: "runtime-yaml: legacy challenged REGISTER facade over the canonical retained options action",
        },
        VariantAllowance {
            variant: "SendUnREGISTER",
            owner: "runtime-yaml: legacy Expires-zero facade over the canonical retained REGISTER options action",
        },
        VariantAllowance {
            variant: "HoldCall",
            owner: "public-serde: programmatic StateTableBuilder action sends the lane-owned hold re-INVITE",
        },
        VariantAllowance {
            variant: "ResumeCall",
            owner: "public-serde: programmatic StateTableBuilder action sends the lane-owned resume re-INVITE",
        },
        VariantAllowance {
            variant: "TransferCall",
            owner: "public-serde: programmatic StateTableBuilder action sends the options-based REFER",
        },
        VariantAllowance {
            variant: "StartRecording",
            owner: "public-serde: programmatic StateTableBuilder media-recording action",
        },
        VariantAllowance {
            variant: "StopRecording",
            owner: "public-serde: programmatic StateTableBuilder media-recording action",
        },
        VariantAllowance {
            variant: "PlayAudioFile",
            owner: "public: programmatic media action outside the embedded SIP profile",
        },
        VariantAllowance {
            variant: "StartRecordingMedia",
            owner: "public: programmatic media action outside the embedded SIP profile",
        },
        VariantAllowance {
            variant: "StopRecordingMedia",
            owner: "public: programmatic media action outside the embedded SIP profile",
        },
        VariantAllowance {
            variant: "CreateAudioMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "RedirectToMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "ConnectToMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "DisconnectFromMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "MuteToMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "UnmuteToMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "DestroyMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "BridgeToMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "RestoreDirectMedia",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "StartRecordingMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "StopRecordingMixer",
            owner: "direct: conference media is mixer-owned",
        },
        VariantAllowance {
            variant: "UpdateMediaDirection",
            owner: "public: programmatic media-direction action",
        },
        VariantAllowance {
            variant: "HoldCurrentCall",
            owner: "runtime-yaml: transfer helper for externally supplied tables",
        },
        VariantAllowance {
            variant: "SetCondition",
            owner: "runtime-yaml: parameterized condition action for custom tables",
        },
        VariantAllowance {
            variant: "StoreLocalSDP",
            owner: "runtime-yaml: supported action for externally supplied tables",
        },
        VariantAllowance {
            variant: "StoreNegotiatedConfig",
            owner: "runtime-yaml: supported action for externally supplied tables",
        },
        VariantAllowance {
            variant: "CreateBridge",
            owner: "runtime-yaml: retained public legacy bridge metadata action; the server bridge path uses media-core directly",
        },
        VariantAllowance {
            variant: "DestroyBridge",
            owner: "runtime-yaml: retained public legacy bridge metadata action; the server bridge path uses media-core directly",
        },
        VariantAllowance {
            variant: "RestoreMediaFlow",
            owner: "runtime-yaml: transfer helper for externally supplied tables",
        },
        VariantAllowance {
            variant: "ReleaseAllResources",
            owner: "public-serde: programmatic StateTableBuilder action performs exact dialog and media cleanup",
        },
        VariantAllowance {
            variant: "StartEmergencyCleanup",
            owner: "public-serde: programmatic StateTableBuilder action performs best-effort exact cleanup",
        },
        VariantAllowance {
            variant: "AttemptMediaRecovery",
            owner: "public-serde: programmatic compatibility action retained without a live recovery implementation",
        },
        VariantAllowance {
            variant: "CleanupResources",
            owner: "public-serde: programmatic compatibility no-op; exact lifecycle cleanup uses dedicated owners",
        },
        VariantAllowance {
            variant: "TriggerCallEstablished",
            owner: "runtime-yaml: callback action for externally supplied tables",
        },
        VariantAllowance {
            variant: "TriggerCallTerminated",
            owner: "runtime-yaml: callback action for externally supplied tables",
        },
        VariantAllowance {
            variant: "StartDialogCleanup",
            owner: "runtime-yaml: cleanup action for externally supplied tables",
        },
        VariantAllowance {
            variant: "StartMediaCleanup",
            owner: "runtime-yaml: cleanup action for externally supplied tables",
        },
        VariantAllowance {
            variant: "ProcessRegistrationResponse",
            owner: "runtime-yaml: registration action for externally supplied tables",
        },
        VariantAllowance {
            variant: "SendSUBSCRIBE",
            owner: "direct: standalone SUBSCRIBE is dialog/transaction-owned",
        },
        VariantAllowance {
            variant: "SendMESSAGE",
            owner: "direct: standalone MESSAGE is dialog/transaction-owned",
        },
        VariantAllowance {
            variant: "ProcessMESSAGE",
            owner: "direct: standalone MESSAGE delivery is outside session lifecycle",
        },
        VariantAllowance {
            variant: "SendSUBSCRIBEWithOptions",
            owner: "direct: standalone SUBSCRIBE deliberately bypasses session YAML",
        },
        VariantAllowance {
            variant: "SendMESSAGEWithOptions",
            owner: "direct: standalone MESSAGE deliberately bypasses session YAML",
        },
        VariantAllowance {
            variant: "SendOPTIONSWithOptions",
            owner: "direct: standalone OPTIONS deliberately bypasses session YAML",
        },
        VariantAllowance {
            variant: "ClearPendingReINVITEOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingREGISTEROptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingSUBSCRIBEOptions",
            owner: "direct: standalone SUBSCRIBE has transaction-owned cleanup",
        },
        VariantAllowance {
            variant: "ClearPendingMESSAGEOptions",
            owner: "direct: standalone MESSAGE has transaction-owned cleanup",
        },
        VariantAllowance {
            variant: "ClearPendingNOTIFYOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingBYEOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingCANCELOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingREFEROptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingINFOOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingUPDATEOptions",
            owner: "runtime-yaml: public staged-dispatch cleanup action",
        },
        VariantAllowance {
            variant: "ClearPendingOPTIONSOptions",
            owner: "direct: standalone OPTIONS has transaction-owned cleanup",
        },
    ];

    const EVENT_TEMPLATE_VARIANT_ALLOWANCES: &[VariantAllowance] = &[
        VariantAllowance {
            variant: "StateChanged",
            owner: "direct: programmatic StateTableBuilder publication",
        },
        VariantAllowance {
            variant: "IncomingCall",
            owner: "public-serde: programmatic Transition template publishes its legacy named Custom observation",
        },
        VariantAllowance {
            variant: "MediaFlowEstablished",
            owner: "direct: programmatic StateTableBuilder publication",
        },
        VariantAllowance {
            variant: "MediaNegotiated",
            owner: "public-serde: programmatic Transition template publishes its legacy named Custom observation",
        },
        VariantAllowance {
            variant: "MediaSessionReady",
            owner: "public-serde: programmatic Transition template publishes its legacy named Custom observation",
        },
    ];

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
            event: EventType::MakeCall {
                target: String::new(),
            },
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
        assert!(transition
            .condition_updates
            .dialog_established
            .unwrap_or(false));
    }

    #[test]
    fn wildcard_transition_retains_json_action_payload() {
        let yaml = r#"
version: "1.0"
transitions:
  - role: UAC
    state: Any
    event:
      type: ReceiveNOTIFY
    actions:
      - type: ProcessNOTIFY
"#;

        let mut loader = YamlTableLoader::new();
        loader.load_from_string(yaml).expect("load wildcard YAML");
        let table = loader.build().expect("build wildcard table");
        let transition = table
            .get(&StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event: EventType::ReceiveNOTIFY,
            })
            .expect("resolve UAC wildcard transition");
        assert_eq!(transition.actions, vec![Action::ProcessNOTIFY]);
        assert!(table
            .get(&StateKey {
                role: Role::Both,
                state: CallState::Active,
                event: EventType::ReceiveNOTIFY,
            })
            .is_none());
    }

    #[test]
    fn duplicate_validation_retains_typed_media_event_identity() {
        let distinct = r#"
version: "2.0"
transitions:
  - role: Both
    state: Active
    event:
      type: InternalSessionRefreshUpdateDue
  - role: Both
    state: Active
    event:
      type: InternalSessionRefreshReinviteDue
"#;
        let mut loader = YamlTableLoader::new();
        loader
            .load_from_string(distinct)
            .expect("load distinct RFC 4028 event rows");
        let table = loader
            .build()
            .expect("distinct typed MediaEvent payloads must coexist");
        for event in [
            crate::state_machine::executor::SESSION_REFRESH_DUE_EVENT,
            crate::state_machine::executor::SESSION_REFRESH_REINVITE_DUE_EVENT,
        ] {
            assert!(table
                .get_transition(&StateKey {
                    role: Role::Both,
                    state: CallState::Active,
                    event: EventType::MediaEvent(event.to_string()),
                })
                .is_some());
        }

        let duplicate = r#"
version: "2.0"
transitions:
  - role: Both
    state: Active
    event:
      type: InternalSessionRefreshUpdateDue
  - role: Both
    state: Active
    event:
      type: InternalSessionRefreshUpdateDue
"#;
        let mut loader = YamlTableLoader::new();
        loader
            .load_from_string(duplicate)
            .expect("load exact duplicate fixture");
        let error = match loader.build() {
            Ok(_) => panic!("exact duplicate event row was accepted"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            SessionError::InternalError(ref detail)
                if detail.contains("duplicates transition #1")
                    && detail.contains("event=MediaEvent")
        ));
    }

    /// The embedded `default.yaml` loads without hitting the
    /// `UnknownAction` drift-detection arm. If this regresses, either the
    /// YAML introduced a new action name or an `Action` variant was removed
    /// without also deleting the corresponding YAML entry.
    #[test]
    fn default_yaml_loads_with_no_unknown_actions() {
        // `load_embedded_default` constructs the loader + parses the
        // embedded YAML + builds the `MasterStateTable`. An
        // `Err(SessionError::InternalError("Unknown YAML action ..."))`
        // at parse time is the failure mode we want to catch.
        YamlTableLoader::load_embedded_default().expect(
            "embedded default.yaml failed to load cleanly — \
                     check for dead YAML action names or missing \
                     parse_action_by_name arms",
        );
    }

    /// Every `YamlAction::Simple` name that reaches `parse_action_by_name`
    /// lands on a real variant rather than a `Custom` silent fallback
    /// (unless explicitly allow-listed). The reverse direction — that every
    /// `Action` variant is reachable from at least one YAML name — is
    /// asserted by `default_yaml_loads_with_no_unknown_actions` together
    /// with the CI expectation that new YAML names accompany every new
    /// variant.
    /// Compile-time invariant: rvoip-sip's `MediaSessionId` is the
    /// same type as `rvoip_media_core::DialogId`. If this test stops
    /// compiling the alias has been broken — see Sprint 2.5 P5
    /// (`MEDIA_PLANE_LAYERING_FOLLOWUPS.md`).
    #[test]
    fn media_session_id_is_alias_of_media_core_dialog_id() {
        let _: super::super::types::MediaSessionId = rvoip_media_core::DialogId::new_v4();
    }

    #[test]
    fn parse_action_by_name_hard_errors_on_unknown_names() {
        let loader = YamlTableLoader::new();
        let err = loader
            .parse_action_by_name("ThisNameDoesNotExist42")
            .expect_err("unknown YAML action must be a hard error, not Custom fallback");
        assert!(matches!(
            err,
            SessionError::InternalError(ref detail) if detail.contains("Unknown YAML action")
        ));
        assert!(!format!("{err:?}").contains("ThisNameDoesNotExist42"));
    }

    #[test]
    fn event_action_hard_failures_do_not_narrow_custom_guard_or_publish_grammar() {
        let loader = YamlTableLoader::new();
        let event_error = loader
            .parse_event_by_name("VendorLifecycleEvent")
            .expect_err("unknown YAML lifecycle events must remain hard failures");
        assert!(matches!(
            event_error,
            SessionError::InternalError(ref detail) if detail.contains("Unknown YAML event")
        ));

        assert_eq!(
            loader
                .parse_guard_by_name("VendorAdmissionGuard")
                .expect("custom runtime YAML guards remain supported"),
            Guard::Custom("VendorAdmissionGuard".to_string())
        );
        assert_eq!(
            loader
                .parse_event_template("VendorObservation")
                .expect("custom observational publish templates remain supported"),
            EventTemplate::Custom("VendorObservation".to_string())
        );
    }

    #[test]
    fn ya_503_retained_serde_boundaries_and_yaml_grammar_are_exact() {
        // These default-unused EventType variants are part of the public,
        // serializable programmatic-table contract. Their live ingress has
        // canonical replacements, but removing the variants would still be a
        // public/serde break and therefore fails YA-503's first deletion proof.
        let public_events = [
            EventType::CallEstablished {
                session_id: "session-a".to_string(),
                sdp_answer: Some("v=0".to_string()),
            },
            EventType::DialogInvite,
            EventType::DialogREFER,
            EventType::DialogReINVITE,
            EventType::InternalACKSent,
            EventType::InternalUASMedia,
            EventType::InternalCleanupComplete,
            EventType::Registration401,
            EventType::CleanupComplete,
        ];
        for event in public_events {
            let encoded = serde_json::to_string(&event).expect("serialize retained EventType");
            let decoded: EventType =
                serde_json::from_str(&encoded).expect("deserialize retained EventType");
            assert_eq!(decoded, event, "EventType public serde shape drifted");
        }

        let public_actions = [
            Action::HoldCall,
            Action::ResumeCall,
            Action::TransferCall("sip:target@example.com".to_string()),
            Action::StartRecording,
            Action::StopRecording,
            Action::ReleaseAllResources,
            Action::StartEmergencyCleanup,
            Action::AttemptMediaRecovery,
            Action::CleanupResources,
        ];
        for action in public_actions {
            let encoded = serde_json::to_string(&action).expect("serialize retained Action");
            let decoded: Action =
                serde_json::from_str(&encoded).expect("deserialize retained Action");
            assert_eq!(decoded, action, "Action public serde shape drifted");
        }

        let public_templates = [
            EventTemplate::IncomingCall,
            EventTemplate::MediaNegotiated,
            EventTemplate::MediaSessionReady,
        ];
        for template in public_templates {
            let encoded =
                serde_json::to_string(&template).expect("serialize retained EventTemplate");
            let decoded: EventTemplate =
                serde_json::from_str(&encoded).expect("deserialize retained EventTemplate");
            assert_eq!(
                decoded, template,
                "EventTemplate public serde shape drifted"
            );
        }

        // Preserve the existing configured-YAML grammar exactly: these are
        // programmatic-only compatibility variants, not accepted lifecycle
        // event/action names. Registration401 is the one deliberate legacy
        // YAML alias and must continue to normalize into canonical auth.
        let loader = YamlTableLoader::new();
        for name in [
            "CallEstablished",
            "DialogInvite",
            "DialogREFER",
            "DialogReINVITE",
            "InternalACKSent",
            "InternalUASMedia",
            "InternalCleanupComplete",
            "CleanupComplete",
        ] {
            loader
                .parse_event_by_name(name)
                .expect_err("programmatic-only EventType must not expand YAML grammar");
        }
        assert_eq!(
            loader
                .parse_event_by_name("Registration401")
                .expect("legacy Registration401 YAML alias remains accepted"),
            EventType::AuthRequired {
                status_code: 401,
                challenge: String::new(),
                method: "REGISTER".to_string(),
            }
        );

        for name in [
            "HoldCall",
            "ResumeCall",
            "TransferCall",
            "StartRecording",
            "StopRecording",
            "ReleaseAllResources",
            "StartEmergencyCleanup",
            "AttemptMediaRecovery",
            "CleanupResources",
        ] {
            loader
                .parse_action_by_name(name)
                .expect_err("programmatic-only Action must not expand YAML grammar");
        }

        for name in ["IncomingCall", "MediaNegotiated", "MediaSessionReady"] {
            assert_eq!(
                loader
                    .parse_event_template(name)
                    .expect("custom publish-template grammar remains open"),
                EventTemplate::Custom(name.to_string()),
                "named custom publication semantics changed"
            );
        }
    }

    #[test]
    fn default_yaml_and_rust_variant_inventories_are_bidirectional() {
        let yaml: YamlStateTable = serde_yaml::from_str(DEFAULT_STATE_TABLE_YAML)
            .expect("embedded default YAML must deserialize for inventory checks");
        let loader = YamlTableLoader::new();

        let used_events = yaml
            .transitions
            .iter()
            .map(|transition| {
                loader
                    .parse_event(transition.event.clone())
                    .expect("default YAML event must resolve")
            })
            .map(|event| {
                let variant: &'static str = (&event).into();
                variant
            })
            .collect::<BTreeSet<_>>();

        let used_guards = yaml
            .transitions
            .iter()
            .flat_map(|transition| transition.guards.iter())
            .map(|guard| {
                loader
                    .parse_guard(guard.clone())
                    .expect("default YAML guard must resolve")
            })
            .map(|guard| {
                let variant: &'static str = (&guard).into();
                variant
            })
            .collect::<BTreeSet<_>>();

        let used_actions = yaml
            .transitions
            .iter()
            .flat_map(|transition| transition.actions.iter())
            .map(|action| {
                loader
                    .parse_action(action.clone())
                    .expect("default YAML action must resolve")
            })
            .map(|action| {
                let variant: &'static str = (&action).into();
                variant
            })
            .collect::<BTreeSet<_>>();

        let used_event_templates = yaml
            .transitions
            .iter()
            .flat_map(|transition| transition.publish.iter())
            .map(|template| {
                loader
                    .parse_event_template(template)
                    .expect("default YAML event template must resolve")
            })
            .map(|template| {
                let variant: &'static str = (&template).into();
                variant
            })
            .collect::<BTreeSet<_>>();

        assert_variant_inventory(
            "EventType",
            rust_enum_variants("EventType"),
            &used_events,
            EVENT_VARIANT_ALLOWANCES,
        );
        assert_variant_inventory(
            "Guard",
            rust_enum_variants("Guard"),
            &used_guards,
            GUARD_VARIANT_ALLOWANCES,
        );
        assert_variant_inventory(
            "Action",
            rust_enum_variants("Action"),
            &used_actions,
            ACTION_VARIANT_ALLOWANCES,
        );
        assert_variant_inventory(
            "EventTemplate",
            rust_enum_variants("EventTemplate"),
            &used_event_templates,
            EVENT_TEMPLATE_VARIANT_ALLOWANCES,
        );
    }

    fn rust_enum_variants(enum_name: &str) -> BTreeSet<String> {
        let source = include_str!("types.rs");
        let header = format!("pub enum {enum_name} {{");
        let (_, after_header) = source
            .split_once(&header)
            .unwrap_or_else(|| panic!("missing Rust enum declaration '{enum_name}'"));
        let (body, _) = after_header
            .split_once("\n}\n")
            .unwrap_or_else(|| panic!("missing closing brace for Rust enum '{enum_name}'"));

        let variants = body
            .lines()
            .filter_map(|line| {
                let candidate = line.strip_prefix("    ")?;
                if candidate.starts_with(char::is_whitespace) {
                    return None;
                }
                let variant = candidate
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .collect::<String>();
                (!variant.is_empty()).then_some(variant)
            })
            .collect::<Vec<_>>();
        let unique = variants.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            variants.len(),
            unique.len(),
            "Rust enum '{enum_name}' contains duplicate inventory names"
        );
        assert!(!unique.is_empty(), "Rust enum '{enum_name}' is empty");
        unique
    }

    fn assert_variant_inventory(
        enum_name: &str,
        declared: BTreeSet<String>,
        used_by_default: &BTreeSet<&'static str>,
        allowances: &[VariantAllowance],
    ) {
        let mut allowlisted = BTreeSet::new();
        for allowance in allowances {
            assert!(
                !allowance.owner.starts_with("inventory-boundary:"),
                "{enum_name}::{} still has a provisional YA-503 ownership classification",
                allowance.variant
            );
            assert!(
                allowance.owner.contains(':'),
                "{enum_name}::{} allowance needs a terse ownership class and reason",
                allowance.variant
            );
            assert!(
                declared.contains(allowance.variant),
                "stale {enum_name} allowance for removed variant '{}': {}",
                allowance.variant,
                allowance.owner
            );
            assert!(
                !used_by_default.contains(allowance.variant),
                "{enum_name}::{} is now used by default.yaml; remove its allowance ({})",
                allowance.variant,
                allowance.owner
            );
            assert!(
                allowlisted.insert(allowance.variant.to_string()),
                "duplicate {enum_name} allowance for '{}'",
                allowance.variant
            );
        }

        let used = used_by_default
            .iter()
            .map(|variant| (*variant).to_string())
            .collect::<BTreeSet<_>>();
        let unknown_used = used.difference(&declared).cloned().collect::<Vec<_>>();
        assert!(
            unknown_used.is_empty(),
            "default.yaml resolves to unknown {enum_name} variants: {unknown_used:?}"
        );

        let accounted = used.union(&allowlisted).cloned().collect::<BTreeSet<_>>();
        let missing = declared.difference(&accounted).cloned().collect::<Vec<_>>();
        let stale = accounted.difference(&declared).cloned().collect::<Vec<_>>();
        assert!(
            missing.is_empty() && stale.is_empty(),
            "{enum_name} inventory drift: unaccounted={missing:?}, stale={stale:?}"
        );
    }
}
