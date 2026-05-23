//! In-flight vCon builder, per INTERFACE_DESIGN §3.9 / §11.4.
//!
//! Step-2 skeleton: trait + supporting types only. Production sign/encrypt
//! lands in the future `rvoip-vcon` crate.

use crate::identity::IdentityAssurance;
use crate::ids::{AttachmentId, ParticipantId, StreamId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Opaque reference to a vCon document.
///
/// Placeholder per UCTP plan §2.4: v0 always carries `None`; v0.x's
/// `rvoip-vcon` crate populates `Some(VconRef::Local { uuid })` at
/// `session.ended`. The `Url` variant is reserved for v0.x's remote-
/// resolvable vCon URIs and is intentionally not constructed in v0 —
/// the variant exists so the serde wire shape doesn't churn when
/// `rvoip-vcon` introduces it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum VconRef {
    /// Local store reference; the uuid resolves through whatever
    /// `VconStore` the orchestrator was built with.
    Local { uuid: Uuid },
    /// Future: HTTPS-resolvable vCon URI. Variant reserved; not
    /// constructed in v0.
    Url { url: String },
}

#[derive(Clone, Debug)]
pub struct VconParty {
    pub participant_id: ParticipantId,
    pub display_name: Option<String>,
    pub did_or_stir: Option<String>,
    pub validation: IdentityAssurance,
}

#[derive(Clone, Debug)]
pub struct VconDialog {
    pub kind: VconDialogKind,
    pub stream_id: Option<StreamId>,
    pub started: DateTime<Utc>,
    pub ended: Option<DateTime<Utc>>,
    pub parties: Vec<ParticipantId>,
    pub mimetype: Option<String>,
}

#[derive(Clone, Debug)]
pub enum VconDialogKind {
    Audio,
    Video,
    Text,
    Transfer,
    Other(String),
}

#[derive(Clone, Debug)]
pub struct VconAnalysis {
    pub kind: VconAnalysisKind,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub body: Bytes,
    pub mimetype: String,
}

#[derive(Clone, Debug)]
pub enum VconAnalysisKind {
    Transcript,
    Sentiment,
    Summary,
    Other(String),
}

#[derive(Clone, Debug)]
pub struct VconAttachment {
    pub id: AttachmentId,
    pub mimetype: String,
    pub body: Bytes,
    pub note: Option<String>,
}

#[derive(Clone, Debug)]
pub struct VconSnapshot {
    pub parties: Vec<VconParty>,
    pub dialogs: Vec<VconDialog>,
    pub analyses: Vec<VconAnalysis>,
    pub attachments: Vec<VconAttachment>,
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
