//! In-flight vCon builder, per INTERFACE_DESIGN §3.9 / §11.4.
//!
//! Step-2 skeleton: trait + supporting types only. Production sign/encrypt
//! lands in the future `rvoip-vcon` crate.

use crate::identity::IdentityAssurance;
use crate::ids::{AttachmentId, ParticipantId, StreamId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Opaque reference to a vCon document.
///
/// Placeholder per UCTP plan §2.4: v0 always carries `None`; v0.x's
/// `rvoip-vcon` crate populates `Some(VconRef::Local { uuid })` at
/// `session.ended`. The `Url` variant is reserved for v0.x's remote-
/// resolvable vCon URIs and is intentionally not constructed in v0 —
/// the variant exists so the serde wire shape doesn't churn when
/// `rvoip-vcon` introduces it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum VconRef {
    /// Local store reference; the uuid resolves through whatever
    /// `VconStore` the orchestrator was built with.
    Local { uuid: Uuid },
    /// Future: HTTPS-resolvable vCon URI. Variant reserved; not
    /// constructed in v0.
    Url { url: String },
}

impl fmt::Debug for VconRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Local { .. } => "VconRef::Local",
            Self::Url { .. } => "VconRef::Url",
        })
    }
}

#[derive(Clone)]
pub struct VconParty {
    pub participant_id: ParticipantId,
    pub display_name: Option<String>,
    pub did_or_stir: Option<String>,
    pub validation: IdentityAssurance,
}

impl fmt::Debug for VconParty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconParty")
            .field("display_name_present", &self.display_name.is_some())
            .field("did_or_stir_present", &self.did_or_stir.is_some())
            .field("validation_kind", &self.validation.kind())
            .finish()
    }
}

#[derive(Clone)]
pub struct VconDialog {
    pub kind: VconDialogKind,
    pub stream_id: Option<StreamId>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    pub parties: Vec<ParticipantId>,
    pub mimetype: Option<String>,
}

impl fmt::Debug for VconDialog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconDialog")
            .field("kind", &self.kind)
            .field("stream_id_present", &self.stream_id.is_some())
            .field("started", &self.started)
            .field("ended", &self.ended)
            .field("party_count", &self.parties.len())
            .field("mimetype_present", &self.mimetype.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub enum VconDialogKind {
    Audio,
    Video,
    Text,
    Transfer,
    Other(String),
}

impl fmt::Debug for VconDialogKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Audio => "Audio",
            Self::Video => "Video",
            Self::Text => "Text",
            Self::Transfer => "Transfer",
            Self::Other(_) => "Other",
        })
    }
}

#[derive(Clone)]
pub struct VconAnalysis {
    pub kind: VconAnalysisKind,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub body: Bytes,
    pub mimetype: String,
}

impl fmt::Debug for VconAnalysis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconAnalysis")
            .field("kind", &self.kind)
            .field("vendor_present", &self.vendor.is_some())
            .field("product_present", &self.product.is_some())
            .field("body_bytes", &self.body.len())
            .field("mimetype_present", &!self.mimetype.is_empty())
            .finish()
    }
}

#[derive(Clone)]
pub enum VconAnalysisKind {
    Transcript,
    Sentiment,
    Summary,
    Other(String),
}

impl fmt::Debug for VconAnalysisKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Transcript => "Transcript",
            Self::Sentiment => "Sentiment",
            Self::Summary => "Summary",
            Self::Other(_) => "Other",
        })
    }
}

#[derive(Clone)]
pub struct VconAttachment {
    pub id: AttachmentId,
    pub mimetype: String,
    pub body: Bytes,
    pub note: Option<String>,
}

impl fmt::Debug for VconAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconAttachment")
            .field("mimetype_present", &!self.mimetype.is_empty())
            .field("body_bytes", &self.body.len())
            .field("note_present", &self.note.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct VconSnapshot {
    pub parties: Vec<VconParty>,
    pub dialogs: Vec<VconDialog>,
    pub analyses: Vec<VconAnalysis>,
    pub attachments: Vec<VconAttachment>,
}

impl fmt::Debug for VconSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconSnapshot")
            .field("party_count", &self.parties.len())
            .field("dialog_count", &self.dialogs.len())
            .field("analysis_count", &self.analyses.len())
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

/// Append-only handle exposed by an active [`crate::Session`] for the
/// transcription pipeline / harness / SIP-signaling capture to write into.
/// Owned by the Session; finalized (signed, persisted) at session end.
pub trait VconBuilderHandle: Send + Sync {
    fn add_party(&self, party: VconParty);
    fn add_dialog(&self, dialog: VconDialog);
    fn add_analysis(&self, analysis: VconAnalysis);
    fn add_attachment(&self, attachment: VconAttachment);
    fn snapshot(&self) -> VconSnapshot;
}

/// P3 — default in-memory implementation of [`VconBuilderHandle`].
/// Bound to a Session by the Orchestrator on `start_session`;
/// finalized (snapshotted, encoded as JSON, handed to `VconStore`) on
/// `end_session`. Production signing/encryption replaces the encode
/// step in `rvoip-vcon` behind the `vcon-signing` feature.
pub struct DefaultVconBuilder {
    inner: std::sync::Mutex<VconSnapshot>,
}

impl DefaultVconBuilder {
    pub fn new() -> Self {
        Self {
            inner: std::sync::Mutex::new(VconSnapshot {
                parties: Vec::new(),
                dialogs: Vec::new(),
                analyses: Vec::new(),
                attachments: Vec::new(),
            }),
        }
    }
}

impl Default for DefaultVconBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl VconBuilderHandle for DefaultVconBuilder {
    fn add_party(&self, party: VconParty) {
        self.inner
            .lock()
            .expect("vcon builder lock poisoned")
            .parties
            .push(party);
    }
    fn add_dialog(&self, dialog: VconDialog) {
        self.inner
            .lock()
            .expect("vcon builder lock poisoned")
            .dialogs
            .push(dialog);
    }
    fn add_analysis(&self, analysis: VconAnalysis) {
        self.inner
            .lock()
            .expect("vcon builder lock poisoned")
            .analyses
            .push(analysis);
    }
    fn add_attachment(&self, attachment: VconAttachment) {
        self.inner
            .lock()
            .expect("vcon builder lock poisoned")
            .attachments
            .push(attachment);
    }
    fn snapshot(&self) -> VconSnapshot {
        let g = self.inner.lock().expect("vcon builder lock poisoned");
        VconSnapshot {
            parties: g.parties.clone(),
            dialogs: g.dialogs.clone(),
            analyses: g.analyses.clone(),
            attachments: g.attachments.clone(),
        }
    }
}

/// P3 — encode a snapshot into the bytes handed to `VconStore::put`.
/// v1 path: unsigned JSON envelope with parties/dialogs/analyses/
/// attachments arrays. Signed/encrypted JWS/JWE comes via `rvoip-vcon`
/// behind the `vcon-signing` feature.
pub fn encode_snapshot(snapshot: &VconSnapshot) -> bytes::Bytes {
    // Lightweight hand-rolled JSON encoding to avoid pulling
    // `serde_json::to_vec` for non-serde types. Each section keeps
    // only the fields the v1 wire form needs; the rich Bytes payload
    // inside Analysis/Attachment is base64-omitted (length-only) for
    // now — production encoder in rvoip-vcon handles it properly.
    let mut s = String::from("{\"version\":\"1\",\"parties\":[");
    for (i, p) in snapshot.parties.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"participant_id\":\"{}\",\"display_name\":\"{}\"}}",
            p.participant_id,
            p.display_name.clone().unwrap_or_default(),
        ));
    }
    s.push_str("],\"dialogs\":[");
    for (i, d) in snapshot.dialogs.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"kind\":\"{:?}\",\"started\":\"{}\"}}",
            d.kind, d.started,
        ));
    }
    s.push_str("],\"analyses\":[");
    for (i, a) in snapshot.analyses.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"kind\":\"{:?}\",\"body_len\":{}}}",
            a.kind,
            a.body.len(),
        ));
    }
    s.push_str("],\"attachments\":[");
    for (i, a) in snapshot.attachments.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"id\":\"{}\",\"mimetype\":\"{}\",\"body_len\":{}}}",
            a.id,
            a.mimetype,
            a.body.len(),
        ));
    }
    s.push_str("]}");
    bytes::Bytes::from(s.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcon_ref_local_roundtrips_through_json() {
        let uuid = Uuid::nil();
        let v = VconRef::Local { uuid };
        let json = serde_json::to_value(&v).expect("encode");
        assert_eq!(json["kind"], "local");
        assert_eq!(json["uuid"], uuid.to_string());
        let back: VconRef = serde_json::from_value(json).expect("decode");
        assert_eq!(v, back);
    }

    #[test]
    fn vcon_ref_url_roundtrips_through_json() {
        let v = VconRef::Url {
            url: "https://vcons.example/abc123".into(),
        };
        let json = serde_json::to_value(&v).expect("encode");
        assert_eq!(json["kind"], "url");
        let back: VconRef = serde_json::from_value(json).expect("decode");
        assert_eq!(v, back);
    }
}
