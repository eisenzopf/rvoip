//! In-flight vCon builder, per INTERFACE_DESIGN §3.9 / §11.4.
//!
//! The builder keeps core-native identifiers while a Session is active.
//! At finalization its snapshot is converted into the canonical
//! `rvoip-vcon` model, validated, and serialized with serde.

use crate::identity::IdentityAssurance;
use crate::ids::{AttachmentId, ParticipantId, StreamId};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Opaque reference to a vCon document.
///
/// `RecordingComplete.vcon_ref` is not wired in 0.3.3 and remains `None`;
/// finalized documents are announced separately through `VconReady`.
/// These variants preserve the planned reference shape without claiming
/// that recording completion owns Session-level vCon finalization.
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
    /// Decentralized identifier URI, if one was asserted for this party.
    pub did: Option<String>,
    /// STIR PASSporT in compact JWS form, if one was captured.
    pub stir: Option<String>,
    pub validation: IdentityAssurance,
}

impl fmt::Debug for VconParty {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconParty")
            .field("display_name_present", &self.display_name.is_some())
            .field("did_present", &self.did.is_some())
            .field("stir_present", &self.stir.is_some())
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
    pub mediatype: Option<String>,
    /// Inline dialog content. Arbitrary bytes are encoded as base64url
    /// when the snapshot becomes a canonical vCon.
    pub body: Option<Bytes>,
    /// External dialog content. `content_hash` is required whenever
    /// this URL is present.
    pub url: Option<String>,
    pub content_hash: Option<String>,
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
            .field("mediatype_present", &self.mediatype.is_some())
            .field("body_bytes", &self.body.as_ref().map_or(0, Bytes::len))
            .field("url_present", &self.url.is_some())
            .field("content_hash_present", &self.content_hash.is_some())
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
    /// Required by the vCon core schema.
    pub vendor: String,
    pub product: Option<String>,
    pub body: Bytes,
    pub mediatype: String,
}

impl fmt::Debug for VconAnalysis {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconAnalysis")
            .field("kind", &self.kind)
            .field("vendor_present", &!self.vendor.is_empty())
            .field("product_present", &self.product.is_some())
            .field("body_bytes", &self.body.len())
            .field("mediatype_present", &!self.mediatype.is_empty())
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
    /// Time the attachment was sent or exchanged.
    pub started: DateTime<Utc>,
    /// Party that contributed the attachment.
    pub party: ParticipantId,
    /// Dialog to which this attachment belongs.
    pub dialog: StreamId,
    pub mediatype: String,
    pub body: Bytes,
    pub purpose: Option<String>,
}

impl fmt::Debug for VconAttachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconAttachment")
            .field("started", &self.started)
            .field("mediatype_present", &!self.mediatype.is_empty())
            .field("body_bytes", &self.body.len())
            .field("purpose_present", &self.purpose.is_some())
            .finish()
    }
}

#[derive(Clone)]
pub struct VconSnapshot {
    /// Stable document identity allocated when the active builder is created.
    pub uuid: Uuid,
    /// Stable creation time allocated with the document identity.
    pub created_at: DateTime<Utc>,
    pub parties: Vec<VconParty>,
    pub dialogs: Vec<VconDialog>,
    pub analyses: Vec<VconAnalysis>,
    pub attachments: Vec<VconAttachment>,
}

impl fmt::Debug for VconSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VconSnapshot")
            .field("uuid_version", &self.uuid.get_version_num())
            .field("created_at", &self.created_at)
            .field("party_count", &self.parties.len())
            .field("dialog_count", &self.dialogs.len())
            .field("analysis_count", &self.analyses.len())
            .field("attachment_count", &self.attachments.len())
            .finish()
    }
}

/// Append-only handle exposed by an active [`crate::Session`] for the
/// transcription pipeline / harness / SIP-signaling capture to write into.
/// Owned by the Session; validated, serialized, and persisted at session end.
pub trait VconBuilderHandle: Send + Sync {
    fn add_party(&self, party: VconParty);
    fn add_dialog(&self, dialog: VconDialog);
    fn add_analysis(&self, analysis: VconAnalysis);
    fn add_attachment(&self, attachment: VconAttachment);
    fn snapshot(&self) -> VconSnapshot;
}

/// P3 — default in-memory implementation of [`VconBuilderHandle`].
/// Bound to a Session by the Orchestrator on `start_session`;
/// finalized (snapshotted, validated, encoded as JSON, handed to
/// `VconStore`) on `end_session`. JWS signing remains an explicit
/// `rvoip-vcon` operation.
pub struct DefaultVconBuilder {
    inner: std::sync::Mutex<VconSnapshot>,
}

impl DefaultVconBuilder {
    pub fn new() -> Self {
        Self::with_identity(new_v8_uuid(), Utc::now())
    }

    /// Create a builder with a caller-supplied document UUID.
    pub fn with_uuid(uuid: Uuid) -> Self {
        Self::with_identity(uuid, Utc::now())
    }

    /// Create a builder with an explicit identity and timestamp.
    ///
    /// This is useful when an upstream recorder owns the identifier or
    /// when deterministic fixtures need to retain their source timestamp.
    pub fn with_identity(uuid: Uuid, created_at: DateTime<Utc>) -> Self {
        Self {
            inner: std::sync::Mutex::new(VconSnapshot {
                uuid,
                created_at,
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
        let mut snapshot = self.inner.lock().expect("vcon builder lock poisoned");
        if let Some(existing) = snapshot
            .parties
            .iter_mut()
            .find(|existing| existing.participant_id == party.participant_id)
        {
            if existing.display_name.is_none() {
                existing.display_name = party.display_name;
            }
            if existing.did.is_none() {
                existing.did = party.did;
            }
            if existing.stir.is_none() {
                existing.stir = party.stir;
            }
            if matches!(&existing.validation, IdentityAssurance::Anonymous)
                && !matches!(&party.validation, IdentityAssurance::Anonymous)
            {
                existing.validation = party.validation;
            }
            return;
        }
        snapshot.parties.push(party);
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
            uuid: g.uuid,
            created_at: g.created_at,
            parties: g.parties.clone(),
            dialogs: g.dialogs.clone(),
            analyses: g.analyses.clone(),
            attachments: g.attachments.clone(),
        }
    }
}

fn new_v8_uuid() -> Uuid {
    // uuid's workspace feature set includes random v4 generation but does
    // not require its optional v8 feature. Start with cryptographically
    // random bytes, then set the RFC 9562 custom-version and RFC variant
    // bits explicitly.
    let mut bytes = *Uuid::new_v4().as_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(feature = "vcon")]
impl TryFrom<&VconSnapshot> for rvoip_vcon::Vcon {
    type Error = rvoip_vcon::VconError;

    fn try_from(snapshot: &VconSnapshot) -> Result<Self, Self::Error> {
        use rvoip_vcon::{
            Analysis, Attachment, ContentEncoding, ContentHashes, Dialog, DialogKind, Party,
            PartyIndices, Vcon, VconError,
        };
        use std::collections::{HashMap, HashSet};

        let invalid = |message: String| VconError::Invalid(message);

        let mut party_indices = HashMap::with_capacity(snapshot.parties.len());
        let mut parties = Vec::with_capacity(snapshot.parties.len());
        for (index, party) in snapshot.parties.iter().enumerate() {
            let index = u32::try_from(index)
                .map_err(|_| invalid("vCon contains more than u32::MAX parties".into()))?;
            if party_indices
                .insert(party.participant_id.clone(), index)
                .is_some()
            {
                return Err(invalid(format!(
                    "duplicate participant {} in vCon snapshot",
                    party.participant_id
                )));
            }
            parties.push(Party {
                uuid: Some(party.participant_id.to_string()),
                name: party.display_name.clone(),
                did: party.did.clone(),
                stir: party.stir.clone(),
                validation: Some(party.validation.kind().to_owned()),
                ..Party::default()
            });
        }

        // Index all dialogs first so attachments may reference a dialog
        // regardless of their relative insertion order.
        let mut dialog_indices = HashMap::with_capacity(snapshot.dialogs.len());
        for (index, dialog) in snapshot.dialogs.iter().enumerate() {
            if let Some(stream_id) = &dialog.stream_id {
                let index = u32::try_from(index)
                    .map_err(|_| invalid("vCon contains more than u32::MAX dialogs".into()))?;
                if dialog_indices.insert(stream_id.clone(), index).is_some() {
                    return Err(invalid(format!(
                        "duplicate stream {} in vCon snapshot",
                        stream_id
                    )));
                }
            }
        }

        let mut dialogs = Vec::with_capacity(snapshot.dialogs.len() + 1);
        let mut recording_members = Vec::new();
        for dialog in &snapshot.dialogs {
            let kind = match &dialog.kind {
                VconDialogKind::Audio | VconDialogKind::Video => DialogKind::Recording,
                VconDialogKind::Text => DialogKind::Text,
                VconDialogKind::Transfer => DialogKind::Transfer,
                VconDialogKind::Other(kind) => {
                    return Err(invalid(format!(
                        "unsupported core dialog kind {kind:?}; declare a vCon extension instead"
                    )));
                }
            };

            let duration = match dialog.ended {
                Some(ended) if ended < dialog.started => {
                    return Err(invalid(
                        "vCon dialog ended before its start timestamp".into(),
                    ));
                }
                Some(ended) => {
                    let micros = (ended - dialog.started)
                        .num_microseconds()
                        .ok_or_else(|| invalid("vCon dialog duration overflowed".into()))?;
                    Some(micros as f64 / 1_000_000.0)
                }
                None => None,
            };

            let referenced_parties = dialog
                .parties
                .iter()
                .map(|participant_id| {
                    party_indices.get(participant_id).copied().ok_or_else(|| {
                        invalid(format!(
                            "dialog references unknown participant {}",
                            participant_id
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let body = dialog
                .body
                .as_ref()
                .map(|body| serde_json::Value::String(rvoip_vcon::encode_base64url(body.as_ref())));
            let encoding = body.as_ref().map(|_| ContentEncoding::Base64Url);
            let output_index = u32::try_from(dialogs.len())
                .map_err(|_| invalid("vCon contains more than u32::MAX dialogs".into()))?;
            if kind == DialogKind::Recording {
                recording_members.push((
                    output_index,
                    dialog.started,
                    dialog.ended,
                    referenced_parties.clone(),
                ));
            }

            dialogs.push(Dialog {
                kind,
                start: dialog.started,
                duration,
                parties: (!referenced_parties.is_empty())
                    .then_some(PartyIndices::Many(referenced_parties)),
                mediatype: dialog.mediatype.clone(),
                body,
                encoding,
                url: dialog.url.clone(),
                content_hash: dialog.content_hash.clone().map(ContentHashes::One),
                ..Dialog::default()
            });
        }

        // Every core snapshot represents one Session. When that Session has
        // several recording files, append a recording-set after the source
        // dialogs so their indices (and attachment references) stay stable.
        if recording_members.len() > 1 {
            let set_index = u32::try_from(dialogs.len())
                .map_err(|_| invalid("vCon contains more than u32::MAX dialogs".into()))?;
            let start = recording_members
                .iter()
                .map(|(_, start, _, _)| *start)
                .min()
                .expect("recording members is non-empty");
            let duration = if recording_members
                .iter()
                .all(|(_, _, ended, _)| ended.is_some())
            {
                let ended = recording_members
                    .iter()
                    .filter_map(|(_, _, ended, _)| ended.as_ref())
                    .max()
                    .expect("all recording members have an end");
                let micros = (*ended - start)
                    .num_microseconds()
                    .ok_or_else(|| invalid("vCon recording-set duration overflowed".into()))?;
                Some(micros as f64 / 1_000_000.0)
            } else {
                None
            };

            let mut seen_parties = HashSet::new();
            let mut set_parties = Vec::new();
            for (_, _, _, parties) in &recording_members {
                for party in parties {
                    if seen_parties.insert(*party) {
                        set_parties.push(*party);
                    }
                }
            }
            let recordings = recording_members
                .iter()
                .map(|(index, _, _, _)| *index)
                .collect::<Vec<_>>();
            for recording in &recordings {
                dialogs[*recording as usize].recording_set = Some(set_index);
            }
            dialogs.push(Dialog {
                kind: DialogKind::RecordingSet,
                start,
                duration,
                parties: (!set_parties.is_empty()).then_some(PartyIndices::Many(set_parties)),
                recordings,
                ..Dialog::default()
            });
        }

        let analysis = snapshot
            .analyses
            .iter()
            .map(|item| {
                let kind = match &item.kind {
                    VconAnalysisKind::Transcript => "transcript",
                    VconAnalysisKind::Sentiment => "sentiment",
                    VconAnalysisKind::Summary => "summary",
                    VconAnalysisKind::Other(kind) => kind,
                };
                Analysis {
                    kind: kind.to_owned(),
                    vendor: item.vendor.clone(),
                    product: item.product.clone(),
                    mediatype: Some(item.mediatype.clone()),
                    body: Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
                        item.body.as_ref(),
                    ))),
                    encoding: Some(ContentEncoding::Base64Url),
                    ..Analysis::default()
                }
            })
            .collect();

        let attachments = snapshot
            .attachments
            .iter()
            .map(|item| {
                let party = party_indices.get(&item.party).copied().ok_or_else(|| {
                    invalid(format!(
                        "attachment {} references unknown participant {}",
                        item.id, item.party
                    ))
                })?;
                let dialog = dialog_indices.get(&item.dialog).copied().ok_or_else(|| {
                    invalid(format!(
                        "attachment {} references unknown dialog {}",
                        item.id, item.dialog
                    ))
                })?;
                Ok(Attachment {
                    purpose: item.purpose.clone(),
                    start: item.started,
                    party,
                    dialog,
                    mediatype: Some(item.mediatype.clone()),
                    body: Some(serde_json::Value::String(rvoip_vcon::encode_base64url(
                        item.body.as_ref(),
                    ))),
                    encoding: Some(ContentEncoding::Base64Url),
                    ..Attachment::default()
                })
            })
            .collect::<Result<Vec<_>, VconError>>()?;

        let vcon = Vcon {
            uuid: snapshot.uuid,
            vcon: Some("0.4.0".into()),
            created_at: snapshot.created_at,
            parties,
            dialog: dialogs,
            analysis,
            attachments,
            ..Vcon::default()
        };
        vcon.validate()?;
        Ok(vcon)
    }
}

/// Convert, validate, and serialize a Session snapshot into an unsigned
/// canonical vCon document.
#[cfg(feature = "vcon")]
pub fn encode_snapshot(snapshot: &VconSnapshot) -> Result<bytes::Bytes, rvoip_vcon::VconError> {
    let vcon = rvoip_vcon::Vcon::try_from(snapshot)?;
    Ok(bytes::Bytes::from(serde_json::to_vec(&vcon)?))
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

    #[test]
    fn builder_retains_caller_supplied_identity() {
        let uuid = Uuid::nil();
        let created_at = DateTime::<Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let snapshot = DefaultVconBuilder::with_identity(uuid, created_at).snapshot();
        assert_eq!(snapshot.uuid, uuid);
        assert_eq!(snapshot.created_at, created_at);
    }

    #[test]
    fn duplicate_party_updates_fill_missing_metadata_without_downgrading() {
        let builder = DefaultVconBuilder::new();
        let participant_id = ParticipantId::new();
        builder.add_party(VconParty {
            participant_id: participant_id.clone(),
            display_name: None,
            did: None,
            stir: None,
            validation: IdentityAssurance::Anonymous,
        });
        builder.add_party(VconParty {
            participant_id: participant_id.clone(),
            display_name: Some("Alice".into()),
            did: Some("did:example:alice".into()),
            stir: Some("header.payload.signature".into()),
            validation: IdentityAssurance::DtlsFingerprint {
                algorithm: "sha-256".into(),
                value: "AA:BB".into(),
            },
        });
        builder.add_party(VconParty {
            participant_id,
            display_name: None,
            did: None,
            stir: None,
            validation: IdentityAssurance::Anonymous,
        });

        let snapshot = builder.snapshot();
        assert_eq!(snapshot.parties.len(), 1);
        assert_eq!(snapshot.parties[0].display_name.as_deref(), Some("Alice"));
        assert_eq!(
            snapshot.parties[0].did.as_deref(),
            Some("did:example:alice")
        );
        assert_eq!(
            snapshot.parties[0].stir.as_deref(),
            Some("header.payload.signature")
        );
        assert_eq!(snapshot.parties[0].validation.kind(), "dtls-fingerprint");
    }
}
