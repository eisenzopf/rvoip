//! Session envelope payloads per CONVERSATION_PROTOCOL.md §7.2–§7.3.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// `session.invite` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInvite {
    pub from: String,
    pub to: Vec<String>,
    pub medium: String,
    pub intent: String,
    /// CapabilityDescriptor — initiator's offer.
    pub capabilities_offer: serde_json::Value,
}

/// `session.accept` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionAccept {
    pub by: String,
    pub capabilities_answer: serde_json::Value,
}

/// `session.reject` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionReject {
    pub by: String,
    pub reason_code: u16,
    pub reason: String,
}

/// `session.end` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEnd {
    pub by: String,
    pub reason_code: u16,
    pub reason: String,
}

/// `session.update` (bidi) payload — mid-session change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub kind: String,
    #[serde(default)]
    pub details: serde_json::Value,
}

/// `session.cancel` (bidi) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionCancel {
    pub by: String,
    pub reason_code: u16,
    pub reason: String,
}

/// `session.started` (S→C, multicast) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionStarted {
    pub started_at: DateTime<Utc>,
    pub participants_present: Vec<String>,
    pub active_connections: Vec<ActiveConnection>,
    pub negotiated_capabilities: serde_json::Value,
}

/// `session.ended` (S→C, multicast) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionEnded {
    pub ended_at: DateTime<Utc>,
    #[serde(default)]
    pub by: Option<String>,
    pub reason_code: u16,
    pub reason: String,
    #[serde(default)]
    pub vcon_handle: Option<serde_json::Value>,
}

/// `session.participant.joined` (S→C, multicast) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionParticipantJoined {
    pub participant: JoinedParticipant,
    pub via_connection: String,
}

/// `session.participant.left` (S→C, multicast) payload.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionParticipantLeft {
    pub participant_id: String,
    pub left_at: DateTime<Utc>,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveConnection {
    pub connid: String,
    pub participant_id: String,
    pub transport: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinedParticipant {
    pub participant_id: String,
    pub identity_id: String,
    pub kind: String,
    pub role: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub joined_at: DateTime<Utc>,
}
