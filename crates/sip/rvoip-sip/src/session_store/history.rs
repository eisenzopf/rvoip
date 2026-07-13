//! Session history tracking for debugging and analysis

use crate::state_table::{Action, EventTemplate, EventType, Guard};
use crate::types::CallState;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const REDACTED_AUTH_ACTION_ERROR: &str = "outbound authentication action failed";
const REDACTED_AUTH_TRANSITION_ERROR: &str = "authentication transition failed";
const REDACTED_ACTION_ERROR: &str = "state-machine action failed";
const REDACTED_TRANSITION_ERROR: &str = "state-machine transition failed";

fn safe_auth_method_label(method: &str) -> &'static str {
    match method.trim().to_ascii_uppercase().as_str() {
        "INVITE" => "INVITE",
        "REGISTER" => "REGISTER",
        "BYE" => "BYE",
        "REFER" => "REFER",
        "NOTIFY" => "NOTIFY",
        "INFO" => "INFO",
        "UPDATE" => "UPDATE",
        "MESSAGE" => "MESSAGE",
        "OPTIONS" => "OPTIONS",
        "SUBSCRIBE" => "SUBSCRIBE",
        _ => "extension",
    }
}

fn challenge_realm_len(challenge: &str) -> Option<usize> {
    let lower = challenge.to_ascii_lowercase();
    for (offset, _) in lower.match_indices("realm") {
        let before_is_boundary = offset == 0
            || lower[..offset]
                .chars()
                .next_back()
                .is_some_and(|ch| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-');
        if !before_is_boundary {
            continue;
        }

        let mut value = &challenge[offset + "realm".len()..];
        value = value.trim_start();
        let Some(after_equals) = value.strip_prefix('=') else {
            continue;
        };
        value = after_equals.trim_start();
        if let Some(quoted) = value.strip_prefix('"') {
            return quoted.find('"').map(|end| quoted[..end].len());
        }
        let end = value
            .find(|ch: char| ch == ',' || ch.is_ascii_whitespace())
            .unwrap_or(value.len());
        return Some(value[..end].len());
    }
    None
}

fn auth_challenge_history_metadata(challenge: &str) -> String {
    let realm_len = challenge_realm_len(challenge);
    format!(
        "metadata(challenge_present={},challenge_bytes={},realm_present={},realm_bytes={})",
        !challenge.is_empty(),
        challenge.len(),
        realm_len.is_some(),
        realm_len.unwrap_or_default()
    )
}

fn text_history_metadata(value: &str) -> String {
    format!(
        "metadata(present={},bytes={})",
        !value.is_empty(),
        value.len()
    )
}

fn optional_text_history_metadata(value: &Option<String>) -> Option<String> {
    value.as_deref().map(text_history_metadata)
}

fn text_list_history_metadata(values: &[String]) -> Vec<String> {
    let bytes = values.iter().map(String::len).sum::<usize>();
    vec![format!("metadata(items={},bytes={bytes})", values.len())]
}

/// Return a persistence-safe event snapshot. Live state-machine events retain
/// their full payloads; history keeps only bounded metadata for auth material.
pub(crate) fn history_event_snapshot(event: &EventType) -> EventType {
    match event {
        EventType::MakeCall { target } => EventType::MakeCall {
            target: text_history_metadata(target),
        },
        EventType::IncomingCall { from, sdp } => EventType::IncomingCall {
            from: text_history_metadata(from),
            sdp: optional_text_history_metadata(sdp),
        },
        EventType::IncomingCallAutoAccept { from, sdp } => EventType::IncomingCallAutoAccept {
            from: text_history_metadata(from),
            sdp: optional_text_history_metadata(sdp),
        },
        EventType::RejectCall { status, reason } => EventType::RejectCall {
            status: *status,
            reason: text_history_metadata(reason),
        },
        EventType::RedirectCall { status, contacts } => EventType::RedirectCall {
            status: *status,
            contacts: text_list_history_metadata(contacts),
        },
        EventType::SendEarlyMedia { sdp } => EventType::SendEarlyMedia {
            sdp: optional_text_history_metadata(sdp),
        },
        EventType::AuthRequired {
            status_code,
            challenge,
            method,
        } => EventType::AuthRequired {
            status_code: *status_code,
            challenge: auth_challenge_history_metadata(challenge),
            method: safe_auth_method_label(method).to_string(),
        },
        EventType::PlayAudio { file } => EventType::PlayAudio {
            file: text_history_metadata(file),
        },
        EventType::DialogCreated { dialog_id, call_id } => EventType::DialogCreated {
            dialog_id: text_history_metadata(dialog_id),
            call_id: text_history_metadata(call_id),
        },
        EventType::CallEstablished {
            session_id,
            sdp_answer,
        } => EventType::CallEstablished {
            session_id: text_history_metadata(session_id),
            sdp_answer: optional_text_history_metadata(sdp_answer),
        },
        EventType::Dialog3xxRedirect { status, targets } => EventType::Dialog3xxRedirect {
            status: *status,
            targets: text_list_history_metadata(targets),
        },
        EventType::DialogError(error) => EventType::DialogError(text_history_metadata(error)),
        EventType::DialogStateChanged {
            old_state,
            new_state,
        } => EventType::DialogStateChanged {
            old_state: text_history_metadata(old_state),
            new_state: text_history_metadata(new_state),
        },
        EventType::ReinviteReceived { sdp } => EventType::ReinviteReceived {
            sdp: optional_text_history_metadata(sdp),
        },
        EventType::UpdateReceived { sdp } => EventType::UpdateReceived {
            sdp: optional_text_history_metadata(sdp),
        },
        EventType::TransferRequested {
            refer_to,
            transfer_type,
            transaction_id,
        } => EventType::TransferRequested {
            refer_to: text_history_metadata(refer_to),
            transfer_type: text_history_metadata(transfer_type),
            transaction_id: text_history_metadata(transaction_id),
        },
        EventType::MediaError(error) => EventType::MediaError(text_history_metadata(error)),
        EventType::MediaEvent(event) => EventType::MediaEvent(text_history_metadata(event)),
        EventType::MediaQualityDegraded {
            packet_loss_percent,
            jitter_ms,
            severity,
        } => EventType::MediaQualityDegraded {
            packet_loss_percent: *packet_loss_percent,
            jitter_ms: *jitter_ms,
            severity: text_history_metadata(severity),
        },
        EventType::DtmfDetected { duration_ms, .. } => EventType::DtmfDetected {
            digit: '\0',
            duration_ms: *duration_ms,
        },
        EventType::RtpTimeout { last_packet_time } => EventType::RtpTimeout {
            last_packet_time: text_history_metadata(last_packet_time),
        },
        EventType::CreateConference { name } => EventType::CreateConference {
            name: text_history_metadata(name),
        },
        EventType::AddParticipant { session_id } => EventType::AddParticipant {
            session_id: text_history_metadata(session_id),
        },
        EventType::JoinConference { conference_id } => EventType::JoinConference {
            conference_id: text_history_metadata(conference_id),
        },
        EventType::BridgeSessions { other_session } => EventType::BridgeSessions {
            other_session: crate::state_table::SessionId(text_history_metadata(&other_session.0)),
        },
        _ => event.clone(),
    }
}

fn history_action_snapshot(action: &Action) -> Action {
    match action {
        Action::SendSIPResponse(status, reason) => {
            Action::SendSIPResponse(*status, text_history_metadata(reason))
        }
        Action::TransferCall(target) => Action::TransferCall(text_history_metadata(target)),
        Action::PlayAudioFile(file) => Action::PlayAudioFile(text_history_metadata(file)),
        Action::CreateBridge(session_id) => Action::CreateBridge(crate::state_table::SessionId(
            text_history_metadata(&session_id.0),
        )),
        Action::Custom(value) => Action::Custom(text_history_metadata(value)),
        _ => action.clone(),
    }
}

fn history_guard_snapshot(guard: &Guard) -> Guard {
    match guard {
        Guard::Custom(value) => Guard::Custom(text_history_metadata(value)),
        _ => guard.clone(),
    }
}

fn history_event_template_snapshot(event: &EventTemplate) -> EventTemplate {
    match event {
        EventTemplate::Custom(value) => EventTemplate::Custom(text_history_metadata(value)),
        _ => event.clone(),
    }
}

fn is_auth_action(action: &Action) -> bool {
    matches!(
        action,
        Action::SendINVITEWithAuth
            | Action::SendRequestWithAuth
            | Action::SendREGISTERWithAuth
            | Action::StoreAuthChallenge
    )
}

fn sanitize_transition_record(record: &mut TransitionRecord) {
    let auth_transition = matches!(record.event, EventType::AuthRequired { .. });
    record.event = history_event_snapshot(&record.event);

    for action in &mut record.actions_executed {
        action.action = history_action_snapshot(&action.action);
        if action.error.is_some() {
            action.error = Some(
                if is_auth_action(&action.action) {
                    REDACTED_AUTH_ACTION_ERROR
                } else {
                    REDACTED_ACTION_ERROR
                }
                .to_string(),
            );
        }
    }
    for guard in &mut record.guards_evaluated {
        guard.guard = history_guard_snapshot(&guard.guard);
    }
    for event in &mut record.events_published {
        *event = history_event_template_snapshot(event);
    }
    if !record.errors.is_empty() {
        record.errors.fill(
            if auth_transition {
                REDACTED_AUTH_TRANSITION_ERROR
            } else {
                REDACTED_TRANSITION_ERROR
            }
            .to_string(),
        );
    }
}

/// Configuration for history tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Maximum number of transitions to keep per session
    pub max_transitions: usize,

    /// Enable history tracking
    pub enabled: bool,

    /// Include action details in history
    pub track_actions: bool,

    /// Include guard evaluation results
    pub track_guards: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_transitions: 50,
            // Opt-in lives at SessionState::with_history(); once a caller has
            // built a SessionHistory they expect it to record. The previous
            // cfg(debug_assertions) gate silently disabled tracking in release
            // builds, which lost data without the caller's knowledge. Set
            // `enabled: false` explicitly if you want a configured-but-paused
            // history.
            enabled: true,
            track_actions: true,
            track_guards: false,
        }
    }
}

/// Record of a single state transition
#[derive(Clone, Deserialize)]
pub struct TransitionRecord {
    /// When the transition occurred (milliseconds since UNIX epoch)
    #[serde(skip, default = "Instant::now")]
    pub timestamp: Instant,

    /// Timestamp for serialization (milliseconds since UNIX epoch)
    pub timestamp_ms: u64,

    /// Monotonic sequence number
    pub sequence: u64,

    /// State before transition
    pub from_state: CallState,

    /// Event that triggered transition
    pub event: EventType,

    /// State after transition (None if no change)
    pub to_state: Option<CallState>,

    /// Guards that were evaluated
    pub guards_evaluated: Vec<GuardResult>,

    /// Actions that were executed
    pub actions_executed: Vec<ActionRecord>,

    /// Events published as result
    pub events_published: Vec<EventTemplate>,

    /// Duration of transition processing
    pub duration_ms: u64,

    /// Any errors that occurred
    pub errors: Vec<String>,
}

/// Result of guard evaluation
#[derive(Debug, Clone, Deserialize)]
pub struct GuardResult {
    pub guard: Guard,
    pub passed: bool,
    pub evaluation_time_us: u64,
}

/// Record of action execution
#[derive(Clone, Deserialize)]
pub struct ActionRecord {
    pub action: Action,
    pub success: bool,
    pub execution_time_us: u64,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct SerializableTransitionRecord<'a> {
    timestamp_ms: u64,
    sequence: u64,
    from_state: CallState,
    event: &'a EventType,
    to_state: Option<CallState>,
    guards_evaluated: &'a [GuardResult],
    actions_executed: &'a [ActionRecord],
    events_published: &'a [EventTemplate],
    duration_ms: u64,
    errors: &'a [String],
}

impl Serialize for TransitionRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut safe = self.clone();
        sanitize_transition_record(&mut safe);
        SerializableTransitionRecord {
            timestamp_ms: safe.timestamp_ms,
            sequence: safe.sequence,
            from_state: safe.from_state,
            event: &safe.event,
            to_state: safe.to_state,
            guards_evaluated: &safe.guards_evaluated,
            actions_executed: &safe.actions_executed,
            events_published: &safe.events_published,
            duration_ms: safe.duration_ms,
            errors: &safe.errors,
        }
        .serialize(serializer)
    }
}

impl std::fmt::Debug for TransitionRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut safe = self.clone();
        sanitize_transition_record(&mut safe);
        formatter
            .debug_struct("TransitionRecord")
            .field("timestamp_ms", &safe.timestamp_ms)
            .field("sequence", &safe.sequence)
            .field("from_state", &safe.from_state)
            .field("event", &safe.event)
            .field("to_state", &safe.to_state)
            .field("guards_evaluated", &safe.guards_evaluated)
            .field("actions_executed", &safe.actions_executed)
            .field("events_published", &safe.events_published)
            .field("duration_ms", &safe.duration_ms)
            .field("errors", &safe.errors)
            .finish()
    }
}

#[derive(Serialize)]
struct SerializableGuardResult<'a> {
    guard: &'a Guard,
    passed: bool,
    evaluation_time_us: u64,
}

impl Serialize for GuardResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let safe_guard = history_guard_snapshot(&self.guard);
        SerializableGuardResult {
            guard: &safe_guard,
            passed: self.passed,
            evaluation_time_us: self.evaluation_time_us,
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct SerializableActionRecord<'a> {
    action: &'a Action,
    success: bool,
    execution_time_us: u64,
    error: Option<&'a str>,
}

impl Serialize for ActionRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let safe_action = history_action_snapshot(&self.action);
        let safe_error = self.error.as_ref().map(|_| {
            if is_auth_action(&safe_action) {
                REDACTED_AUTH_ACTION_ERROR
            } else {
                REDACTED_ACTION_ERROR
            }
        });
        SerializableActionRecord {
            action: &safe_action,
            success: self.success,
            execution_time_us: self.execution_time_us,
            error: safe_error,
        }
        .serialize(serializer)
    }
}

impl std::fmt::Debug for ActionRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let safe_action = history_action_snapshot(&self.action);
        let safe_error = self.error.as_ref().map(|_| {
            if is_auth_action(&safe_action) {
                REDACTED_AUTH_ACTION_ERROR
            } else {
                REDACTED_ACTION_ERROR
            }
        });
        formatter
            .debug_struct("ActionRecord")
            .field("action", &safe_action)
            .field("success", &self.success)
            .field("execution_time_us", &self.execution_time_us)
            .field("error", &safe_error)
            .finish()
    }
}

/// Session history with ring buffer
#[derive(Debug, Clone)]
pub struct SessionHistory {
    /// Ring buffer of transitions
    transitions: VecDeque<TransitionRecord>,

    /// Configuration
    config: HistoryConfig,

    /// Next sequence number
    next_sequence: u64,

    /// Statistics
    pub total_transitions: u64,
    pub total_errors: u64,
    pub session_created: Instant,
    pub last_activity: Instant,
}

impl SessionHistory {
    /// Create new session history
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            transitions: VecDeque::with_capacity(config.max_transitions),
            config,
            next_sequence: 0,
            total_transitions: 0,
            total_errors: 0,
            session_created: Instant::now(),
            last_activity: Instant::now(),
        }
    }

    /// Record a transition
    pub fn record_transition(&mut self, mut record: TransitionRecord) {
        if !self.config.enabled {
            return;
        }

        sanitize_transition_record(&mut record);

        // Filter guards/actions based on config
        if !self.config.track_guards {
            record.guards_evaluated.clear();
        }
        if !self.config.track_actions {
            record.actions_executed.clear();
        }

        // Update statistics
        self.total_transitions += 1;
        if !record.errors.is_empty() {
            self.total_errors += 1;
        }
        self.last_activity = Instant::now();

        // Add sequence number
        record.sequence = self.next_sequence;
        self.next_sequence += 1;

        // Maintain ring buffer size
        if self.transitions.len() >= self.config.max_transitions {
            self.transitions.pop_front();
        }

        self.transitions.push_back(record);
    }

    /// Get recent transitions
    pub fn get_recent(&self, count: usize) -> Vec<TransitionRecord> {
        self.transitions.iter().rev().take(count).cloned().collect()
    }

    /// Get transitions involving a specific state
    pub fn get_by_state(&self, state: CallState) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| t.from_state == state || t.to_state == Some(state))
            .cloned()
            .collect()
    }

    /// Get transitions with errors
    pub fn get_errors(&self) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| !t.errors.is_empty())
            .cloned()
            .collect()
    }

    /// Get transition count for a specific event type
    pub fn count_by_event(&self, event_type: &EventType) -> usize {
        self.transitions
            .iter()
            .filter(|t| std::mem::discriminant(&t.event) == std::mem::discriminant(event_type))
            .count()
    }

    /// Get average transition duration
    pub fn average_duration_ms(&self) -> f64 {
        if self.transitions.is_empty() {
            return 0.0;
        }

        let total: u64 = self.transitions.iter().map(|t| t.duration_ms).sum();
        total as f64 / self.transitions.len() as f64
    }

    /// Clear history
    pub fn clear(&mut self) {
        self.transitions.clear();
        self.next_sequence = 0;
        self.total_transitions = 0;
        self.total_errors = 0;
        self.last_activity = Instant::now();
    }

    /// Export history as JSON
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.transitions).unwrap_or_else(|_| "[]".to_string())
    }

    /// Export history as CSV
    pub fn export_csv(&self) -> String {
        let mut csv =
            String::from("sequence,timestamp_ms,from_state,event,to_state,duration_ms,errors\n");

        for t in &self.transitions {
            let from_state = format!("{:?}", t.from_state);
            let event = format!("{:?}", t.event);
            let to_state = t
                .to_state
                .map(|s| format!("{:?}", s))
                .unwrap_or_else(|| "None".to_string());
            let errors = t.errors.join(";");
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                t.sequence,
                t.timestamp_ms,
                csv_escape(&from_state),
                csv_escape(&event),
                csv_escape(&to_state),
                t.duration_ms,
                csv_escape(&errors)
            ));
        }

        csv
    }

    /// Get session age
    pub fn session_age(&self) -> Duration {
        self.session_created.elapsed()
    }

    /// Get time since last activity
    pub fn idle_time(&self) -> Duration {
        self.last_activity.elapsed()
    }

    /// Check if session is idle
    pub fn is_idle(&self, threshold: Duration) -> bool {
        self.idle_time() > threshold
    }

    /// Get error rate
    pub fn error_rate(&self) -> f32 {
        if self.total_transitions == 0 {
            return 0.0;
        }
        self.total_errors as f32 / self.total_transitions as f32
    }
}

fn csv_escape(value: &str) -> String {
    if value
        .bytes()
        .any(|byte| matches!(byte, b',' | b'"' | b'\r' | b'\n'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_ring_buffer_limit() {
        let config = HistoryConfig {
            max_transitions: 3,
            enabled: true,
            ..Default::default()
        };

        let mut history = SessionHistory::new(config);

        // Add 5 transitions to a buffer with max 3
        for i in 0..5 {
            let now = Instant::now();
            let record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: CallState::Idle,
                event: EventType::MakeCall {
                    target: format!("test{}", i),
                },
                to_state: Some(CallState::Initiating),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };
            history.record_transition(record);
        }

        // Should only have 3 transitions (the last 3)
        assert_eq!(history.transitions.len(), 3);
        assert_eq!(history.total_transitions, 5);

        // Check that we have the last 3 (sequences 2, 3, 4)
        let recent = history.get_recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].sequence, 4);
        assert_eq!(recent[1].sequence, 3);
        assert_eq!(recent[2].sequence, 2);
    }

    #[test]
    fn test_error_tracking() {
        let mut history = SessionHistory::new(HistoryConfig::default());

        // Add some transitions with and without errors
        for i in 0..5 {
            let now = Instant::now();
            let mut record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: CallState::Active,
                event: EventType::HangupCall,
                to_state: Some(CallState::Terminating),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };

            if i % 2 == 0 {
                record.errors.push(format!("Error {}", i));
            }

            history.record_transition(record);
        }

        assert_eq!(history.total_transitions, 5);
        assert_eq!(history.total_errors, 3);
        assert_eq!(history.get_errors().len(), 3);
        assert_eq!(history.error_rate(), 0.6);
    }

    #[test]
    fn test_state_filtering() {
        let mut history = SessionHistory::new(HistoryConfig::default());

        // Add transitions through different states
        let states = vec![
            (CallState::Idle, CallState::Initiating),
            (CallState::Initiating, CallState::Ringing),
            (CallState::Ringing, CallState::Active),
            (CallState::Active, CallState::OnHold),
            (CallState::OnHold, CallState::Active),
        ];

        for (from, to) in states {
            let record = TransitionRecord {
                timestamp: Instant::now(),
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: from,
                event: EventType::MakeCall {
                    target: "test".to_string(),
                },
                to_state: Some(to),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };
            history.record_transition(record);
        }

        // Should find 2 transitions involving Active state
        let active_transitions = history.get_by_state(CallState::Active);
        assert_eq!(active_transitions.len(), 3); // Ringing->Active, Active->OnHold, OnHold->Active
    }

    #[test]
    fn default_history_and_exports_never_retain_auth_values() {
        const CHALLENGE_SECRET: &str = "history-challenge-provider-secret-canary";
        const REALM_SECRET: &str = "history-realm-secret-canary";
        const METHOD_SECRET: &str = "X-HISTORY-METHOD-SECRET-CANARY";
        const ERROR_SECRET: &str = "history-action-error-secret-canary";
        let challenge = format!(
            "Digest realm=\"{REALM_SECRET}\", nonce=\"{CHALLENGE_SECRET}\", algorithm=MALICIOUS"
        );
        let mut history = SessionHistory::new(HistoryConfig::default());
        history.record_transition(TransitionRecord {
            timestamp: Instant::now(),
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0,
            from_state: CallState::Initiating,
            event: EventType::AuthRequired {
                status_code: 401,
                challenge: challenge.clone(),
                method: METHOD_SECRET.to_string(),
            },
            to_state: Some(CallState::Authenticating),
            guards_evaluated: vec![],
            actions_executed: vec![ActionRecord {
                action: Action::SendINVITEWithAuth,
                success: false,
                execution_time_us: 5,
                error: Some(format!("provider failed: {ERROR_SECRET}")),
            }],
            events_published: vec![],
            duration_ms: 1,
            errors: vec![format!("provider failed: {ERROR_SECRET}")],
        });

        let record = history.get_recent(1).pop().expect("history record");
        let EventType::AuthRequired {
            status_code,
            challenge: stored_challenge,
            method,
        } = &record.event
        else {
            panic!("expected AuthRequired history snapshot");
        };
        assert_eq!(*status_code, 401);
        assert_eq!(method, "extension");
        assert_eq!(
            stored_challenge,
            &format!(
                "metadata(challenge_present=true,challenge_bytes={},realm_present=true,realm_bytes={})",
                challenge.len(),
                REALM_SECRET.len()
            )
        );
        assert_eq!(
            record.actions_executed[0].error.as_deref(),
            Some(REDACTED_AUTH_ACTION_ERROR)
        );
        assert_eq!(record.errors, vec![REDACTED_AUTH_TRANSITION_ERROR]);

        for rendered in [
            format!("{record:?}"),
            history.export_json(),
            history.export_csv(),
        ] {
            assert!(rendered.contains("AuthRequired"));
            for secret in [CHALLENGE_SECRET, REALM_SECRET, METHOD_SECRET, ERROR_SECRET] {
                assert!(
                    !rendered.contains(secret),
                    "history leaked {secret}: {rendered}"
                );
            }
        }
    }

    #[test]
    fn default_history_never_retains_arbitrary_action_or_transition_errors() {
        const SECRET: &str = "generic-action-provider-secret-canary";
        let mut history = SessionHistory::new(HistoryConfig::default());
        history.record_transition(TransitionRecord {
            timestamp: Instant::now(),
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0,
            from_state: CallState::Active,
            event: EventType::HangupCall,
            to_state: Some(CallState::Terminating),
            guards_evaluated: vec![],
            actions_executed: vec![ActionRecord {
                action: Action::SendBYE,
                success: false,
                execution_time_us: 1,
                error: Some(format!("provider failed: {SECRET}")),
            }],
            events_published: vec![],
            duration_ms: 1,
            errors: vec![format!("provider failed: {SECRET}")],
        });

        let record = history.get_recent(1).pop().expect("history record");
        assert_eq!(
            record.actions_executed[0].error.as_deref(),
            Some(REDACTED_ACTION_ERROR)
        );
        assert_eq!(record.errors, vec![REDACTED_TRANSITION_ERROR]);
        for rendered in [
            format!("{record:?}"),
            history.export_json(),
            history.export_csv(),
        ] {
            assert!(!rendered.contains(SECRET), "leaked error value: {rendered}");
        }
    }

    #[test]
    fn payload_bearing_events_are_metadata_only_in_history_snapshots() {
        const SECRET: &str = "history-payload-secret\r\nAuthorization: Bearer hidden";
        let events = vec![
            EventType::MakeCall {
                target: SECRET.to_string(),
            },
            EventType::IncomingCall {
                from: SECRET.to_string(),
                sdp: Some(SECRET.to_string()),
            },
            EventType::IncomingCallAutoAccept {
                from: SECRET.to_string(),
                sdp: Some(SECRET.to_string()),
            },
            EventType::RejectCall {
                status: 486,
                reason: SECRET.to_string(),
            },
            EventType::RedirectCall {
                status: 302,
                contacts: vec![SECRET.to_string()],
            },
            EventType::SendEarlyMedia {
                sdp: Some(SECRET.to_string()),
            },
            EventType::AuthRequired {
                status_code: 401,
                challenge: SECRET.to_string(),
                method: SECRET.to_string(),
            },
            EventType::PlayAudio {
                file: SECRET.to_string(),
            },
            EventType::DialogCreated {
                dialog_id: SECRET.to_string(),
                call_id: SECRET.to_string(),
            },
            EventType::CallEstablished {
                session_id: SECRET.to_string(),
                sdp_answer: Some(SECRET.to_string()),
            },
            EventType::Dialog3xxRedirect {
                status: 302,
                targets: vec![SECRET.to_string()],
            },
            EventType::DialogError(SECRET.to_string()),
            EventType::DialogStateChanged {
                old_state: SECRET.to_string(),
                new_state: SECRET.to_string(),
            },
            EventType::ReinviteReceived {
                sdp: Some(SECRET.to_string()),
            },
            EventType::UpdateReceived {
                sdp: Some(SECRET.to_string()),
            },
            EventType::TransferRequested {
                refer_to: SECRET.to_string(),
                transfer_type: SECRET.to_string(),
                transaction_id: SECRET.to_string(),
            },
            EventType::MediaError(SECRET.to_string()),
            EventType::MediaEvent(SECRET.to_string()),
            EventType::MediaQualityDegraded {
                packet_loss_percent: 7,
                jitter_ms: 11,
                severity: SECRET.to_string(),
            },
            EventType::RtpTimeout {
                last_packet_time: SECRET.to_string(),
            },
            EventType::CreateConference {
                name: SECRET.to_string(),
            },
            EventType::AddParticipant {
                session_id: SECRET.to_string(),
            },
            EventType::JoinConference {
                conference_id: SECRET.to_string(),
            },
            EventType::BridgeSessions {
                other_session: crate::state_table::SessionId(SECRET.to_string()),
            },
        ];

        for event in events {
            let live = event.clone();
            let snapshot = history_event_snapshot(&event);
            assert_eq!(event, live, "snapshot mutated live event");
            let debug = format!("{snapshot:?}");
            let serialized = serde_json::to_string(&snapshot).expect("serialize history snapshot");
            assert!(serialized.contains("metadata") || serialized.contains("extension"));
            assert!(!debug.contains(SECRET), "event debug leaked: {debug}");
            assert!(
                !serialized.contains(SECRET),
                "event serialization leaked: {serialized}"
            );
            assert!(
                !serialized.contains("Bearer hidden"),
                "event leaked: {serialized}"
            );
        }

        assert_eq!(
            history_event_snapshot(&EventType::DtmfDetected {
                digit: '\r',
                duration_ms: 80,
            }),
            EventType::DtmfDetected {
                digit: '\0',
                duration_ms: 80,
            }
        );
    }

    #[test]
    fn all_retained_history_payloads_and_csv_are_sanitized() {
        const SECRET: &str = "retained-history-secret\r\nnext,csv,record\"";
        let mut history = SessionHistory::new(HistoryConfig {
            track_actions: true,
            track_guards: true,
            ..HistoryConfig::default()
        });
        history.record_transition(TransitionRecord {
            timestamp: Instant::now(),
            timestamp_ms: 1,
            sequence: 0,
            from_state: CallState::Ringing,
            event: EventType::IncomingCall {
                from: SECRET.to_string(),
                sdp: Some(SECRET.to_string()),
            },
            to_state: Some(CallState::Terminating),
            guards_evaluated: vec![GuardResult {
                guard: Guard::Custom(SECRET.to_string()),
                passed: false,
                evaluation_time_us: 1,
            }],
            actions_executed: vec![
                ActionRecord {
                    action: Action::SendSIPResponse(500, SECRET.to_string()),
                    success: false,
                    execution_time_us: 1,
                    error: Some(SECRET.to_string()),
                },
                ActionRecord {
                    action: Action::TransferCall(SECRET.to_string()),
                    success: true,
                    execution_time_us: 1,
                    error: None,
                },
                ActionRecord {
                    action: Action::PlayAudioFile(SECRET.to_string()),
                    success: true,
                    execution_time_us: 1,
                    error: None,
                },
                ActionRecord {
                    action: Action::CreateBridge(crate::state_table::SessionId(SECRET.to_string())),
                    success: true,
                    execution_time_us: 1,
                    error: None,
                },
                ActionRecord {
                    action: Action::Custom(SECRET.to_string()),
                    success: true,
                    execution_time_us: 1,
                    error: None,
                },
            ],
            events_published: vec![EventTemplate::Custom(SECRET.to_string())],
            duration_ms: 1,
            errors: vec![SECRET.to_string()],
        });

        let record = history.get_recent(1).pop().expect("history record");
        for rendered in [
            format!("{record:?}"),
            history.export_json(),
            history.export_csv(),
        ] {
            assert!(!rendered.contains(SECRET), "history leaked: {rendered}");
            assert!(!rendered.contains("retained-history-secret"));
            assert!(!rendered.contains("next,csv,record"));
        }
        let csv = history.export_csv();
        assert_eq!(csv.lines().count(), 2, "CSV record escaped into extra rows");
        assert!(csv.contains(",1,Ringing,IncomingCall,Terminating,"));
    }

    #[test]
    fn csv_escape_quotes_commas_quotes_and_line_breaks() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape("a\r\nb"), "\"a\r\nb\"");
    }

    #[test]
    fn public_history_records_are_safe_before_insertion() {
        const SECRET: &str = "pre-insertion-history-secret-canary";
        let record = TransitionRecord {
            timestamp: Instant::now(),
            timestamp_ms: 7,
            sequence: 9,
            from_state: CallState::Initiating,
            event: EventType::AuthRequired {
                status_code: 401,
                challenge: format!("Digest realm=\"{SECRET}\", nonce=\"{SECRET}\""),
                method: SECRET.to_string(),
            },
            to_state: Some(CallState::Authenticating),
            guards_evaluated: vec![GuardResult {
                guard: Guard::Custom(SECRET.to_string()),
                passed: false,
                evaluation_time_us: 3,
            }],
            actions_executed: vec![ActionRecord {
                action: Action::Custom(SECRET.to_string()),
                success: false,
                execution_time_us: 4,
                error: Some(SECRET.to_string()),
            }],
            events_published: vec![EventTemplate::Custom(SECRET.to_string())],
            duration_ms: 5,
            errors: vec![SECRET.to_string()],
        };

        for diagnostic in [
            format!("{record:?}"),
            serde_json::to_string(&record).expect("serialize transition record"),
            format!("{:?}", record.actions_executed[0]),
            serde_json::to_string(&record.actions_executed[0]).expect("serialize action record"),
            serde_json::to_string(&record.guards_evaluated[0]).expect("serialize guard result"),
        ] {
            assert!(
                !diagnostic.contains(SECRET),
                "diagnostic leaked: {diagnostic}"
            );
        }
        assert_eq!(format!("{:?}", record.event), "AuthRequired");
        assert_eq!(format!("{:?}", record.actions_executed[0].action), "Custom");
        assert_eq!(format!("{:?}", record.guards_evaluated[0].guard), "Custom");
        assert_eq!(format!("{:?}", record.events_published[0]), "Custom");
    }

    #[test]
    fn auth_history_source_uses_only_sanitized_event_projections() {
        let executor = include_str!("../state_machine/executor.rs");
        assert_eq!(executor.matches("event: history_event").count(), 4);

        let history = include_str!("history.rs");
        assert!(history.contains("record.event = history_event_snapshot(&record.event)"));

        let actions = include_str!("../state_machine/actions.rs");
        assert!(!actions.contains("return m.to_ascii_uppercase()"));
        assert!(actions.contains("return safe_outbound_auth_method_label(m).to_string()"));
        assert!(actions.contains("Method::Extension(\"extension\".to_string())"));
    }
}
