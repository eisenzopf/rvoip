//! vCon data types per the IETF vCon WG draft (`draft-ietf-vcon-vcon-container-00`).
//!
//! The shape mirrors the spec's JSON layout exactly so a serialized
//! `Vcon` round-trips against any conformant reader. Fields the v0.x
//! implementation doesn't populate yet (`analysis`, `attachments`,
//! `redacted`) are present and `#[serde(default)]` so they're skipped
//! when empty.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Top-level vCon container. Spec §3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vcon {
    /// Document UUID. Stable for the lifetime of the conversation
    /// (referenced from `rvoip_core::vcon::VconRef::Local { uuid }`).
    pub uuid: Uuid,
    /// Spec version string. v0.x ships "0.0.1" matching the IETF draft.
    pub vcon: String,
    /// Wall-clock creation time.
    pub created_at: DateTime<Utc>,
    /// Optional human-readable subject. Filled from the session's
    /// `subject` field if present, else left None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// Conversation participants in canonical order (caller first by
    /// convention).
    pub parties: Vec<Party>,
    /// Timeline of recorded segments (media-bearing or text). Each
    /// dialog points back at one or more `parties` indices.
    pub dialog: Vec<Dialog>,
    /// Out-of-band analyses (transcripts, sentiment, etc.). Empty in
    /// v0.x — the AI / transcription pipeline emits these in v1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis: Vec<Analysis>,
    /// Inline or referenced attachments. Empty in v0.x.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// Append-only redaction lineage (vCon §6). Empty in v0.x.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redacted: Vec<RedactionRecord>,
}

/// One conversation participant. Spec §3.2.
///
/// At least one identifier (`tel` / `mailto` / `name` / `uuid`) should
/// be present; v0.x's builder always populates `name` from the
/// `rvoip_core::Participant` field and includes whatever wire IDs the
/// session captured.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Party {
    /// E.164 telephone identifier (`tel:+15551234`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tel: Option<String>,
    /// `mailto:` URI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Opaque internal identifier — rvoip-core's ParticipantId in
    /// stringified form when no PSTN/email is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    /// Role label (`caller`, `agent`, `customer`, `bot`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// One dialog segment. Spec §3.3.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dialog {
    /// Segment kind.
    #[serde(rename = "type")]
    pub kind: DialogKind,
    /// Start time (wall-clock).
    pub start: DateTime<Utc>,
    /// Duration in milliseconds. `None` means "ongoing" (only valid
    /// for in-progress vCons; v0.x always populates this at session
    /// end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Indices into [`Vcon::parties`] for who spoke / sent this
    /// segment. For `recording` kind: all parties present on the
    /// call. For `text`: the sender (single-element vec).
    pub parties: Vec<u32>,
    /// Media MIME type when the dialog references an audio/video
    /// blob (`audio/opus`, `audio/PCMA`, ...). `None` for `text`
    /// kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    /// Inline text body. Used by `text` kind; `None` for media.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Media reference (URL / opaque store handle). v0.x leaves this
    /// `None` because recordings persist via `VconStore`, not direct
    /// URLs — consumers resolve via `VconStore::get(uuid)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// vCon dialog kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DialogKind {
    /// Audio / video recording segment.
    Recording,
    /// Text message (chat, IM, transcript turn).
    Text,
    /// Incoming or outgoing transfer.
    Transfer,
}

/// One analysis attachment. Spec §3.4.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    #[serde(rename = "type")]
    pub kind: String,
    pub dialog: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

/// One attachment. Spec §3.5.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(rename = "type")]
    pub kind: String,
    pub start: DateTime<Utc>,
    pub party: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Append-only redaction lineage entry. Spec §6.
///
/// v0.x stores the data type but doesn't auto-redact; consumers
/// emit these when they replace PII with placeholder regions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedactionRecord {
    pub uuid: Uuid,
    pub redacted_at: DateTime<Utc>,
    pub reason: String,
}

impl Vcon {
    /// New vCon scaffold with a fresh uuid + creation timestamp.
    /// Populated further via [`crate::VconBuilder`]; direct
    /// construction works for tests / one-off code paths.
    pub fn new_now() -> Self {
        Self {
            uuid: Uuid::new_v4(),
            vcon: "0.0.1".into(),
            created_at: Utc::now(),
            subject: None,
            parties: Vec::new(),
            dialog: Vec::new(),
            analysis: Vec::new(),
            attachments: Vec::new(),
            redacted: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum VconError {
    #[error("vcon serialization failed: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("vcon JWS signing failed: {0}")]
    Sign(String),

    #[error("vcon JWS verification failed: {0}")]
    Verify(String),

    #[error("invalid vcon document: {0}")]
    Invalid(String),
}
