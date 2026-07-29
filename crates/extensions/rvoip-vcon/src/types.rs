//! vCon core data types from `draft-ietf-vcon-vcon-core`.
//!
//! The wire shape in this module is pinned to working-group commit
//! `2342aba64bdb71d9e80ab6e274a3921e2b1c769e`. The draft deliberately
//! permits extension parameters on every object, so each object keeps
//! unknown members in its flattened `extra` map.

use std::collections::{BTreeMap, HashSet};

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use url::Url;
use uuid::Uuid;

/// Extension parameters not defined by the core vCon draft.
pub type ExtraFields = BTreeMap<String, Value>;

const VCON_CORE_FIELDS: &[&str] = &[
    "vcon",
    "uuid",
    "extensions",
    "critical",
    "created_at",
    "updated_at",
    "subject",
    "redacted",
    "amended",
    "parties",
    "dialog",
    "analysis",
    "attachments",
];
const PARTY_CORE_FIELDS: &[&str] = &[
    "tel",
    "sip",
    "stir",
    "mailto",
    "name",
    "did",
    "validation",
    "gmlpos",
    "civicaddress",
    "uuid",
    "type",
    "org",
    "dept",
];
const CIVIC_ADDRESS_CORE_FIELDS: &[&str] = &[
    "country", "a1", "a2", "a3", "a4", "a5", "a6", "prd", "pod", "sts", "hno", "hns", "lmk", "loc",
    "flr", "nam", "pc",
];
const DIALOG_CORE_FIELDS: &[&str] = &[
    "type",
    "start",
    "duration",
    "parties",
    "originator",
    "recordings",
    "recording_set",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
    "disposition",
    "session_id",
    "party_history",
    "transferee",
    "transferor",
    "transfer_target",
    "original",
    "consultation",
    "target_dialog",
    "application",
    "message_id",
];
const SESSION_ID_CORE_FIELDS: &[&str] = &["local", "remote"];
const PARTY_HISTORY_CORE_FIELDS: &[&str] = &["party", "time", "event", "button"];
const ANALYSIS_CORE_FIELDS: &[&str] = &[
    "type",
    "vendor",
    "dialog",
    "attachment",
    "mediatype",
    "filename",
    "product",
    "schema",
    "body",
    "encoding",
    "url",
    "content_hash",
];
const ATTACHMENT_CORE_FIELDS: &[&str] = &[
    "purpose",
    "start",
    "party",
    "dialog",
    "mediatype",
    "filename",
    "body",
    "encoding",
    "url",
    "content_hash",
];
const REDACTED_CORE_FIELDS: &[&str] = &["uuid", "type", "url", "content_hash"];
const AMENDED_CORE_FIELDS: &[&str] = &["uuid", "url", "content_hash"];

/// Top-level unsigned vCon object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vcon {
    /// Deprecated syntax version. New documents emit `0.4.0`, while
    /// readers accept its omission as required by the current draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcon: Option<String>,
    pub uuid: Uuid,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub critical: Vec<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<Redacted>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amended: Option<Amended>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parties: Vec<Party>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dialog: Vec<Dialog>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis: Vec<Analysis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// One participant or observer in the conversation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Party {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gmlpos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub civicaddress: Option<CivicAddress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dept: Option<String>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// Civic address fields from RFC 5139.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CivicAddress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a1: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a2: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a3: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a4: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a6: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sts: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hno: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lmk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nam: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pc: Option<String>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// A dialog segment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dialog {
    #[serde(rename = "type")]
    pub kind: DialogKind,
    pub start: DateTime<Utc>,
    /// Duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parties: Option<PartyIndices>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originator: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recordings: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_set: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<ContentEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHashes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<Disposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionIds>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub party_history: Vec<PartyHistory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transferee: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transferor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_target: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consultation: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dialog: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

impl Default for Dialog {
    fn default() -> Self {
        Self {
            kind: DialogKind::Recording,
            start: DateTime::default(),
            duration: None,
            parties: None,
            originator: None,
            recordings: Vec::new(),
            recording_set: None,
            mediatype: None,
            filename: None,
            body: None,
            encoding: None,
            url: None,
            content_hash: None,
            disposition: None,
            session_id: None,
            party_history: Vec::new(),
            transferee: None,
            transferor: None,
            transfer_target: None,
            original: None,
            consultation: None,
            target_dialog: None,
            application: None,
            message_id: None,
            extra: ExtraFields::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DialogKind {
    #[default]
    Recording,
    Text,
    Transfer,
    Incomplete,
    RecordingSet,
}

/// Party indices for either a single mix or a channel-oriented recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PartyIndices {
    One(u32),
    Many(Vec<u32>),
    Channels(Vec<PartyChannel>),
}

impl PartyIndices {
    pub fn iter(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match self {
            Self::One(index) => Box::new(std::iter::once(*index)),
            Self::Many(indices) => Box::new(indices.iter().copied()),
            Self::Channels(channels) => Box::new(
                channels
                    .iter()
                    .flat_map(|channel| channel.indices().into_iter()),
            ),
        }
    }
}

/// One entry in a multi-channel `parties` array.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PartyChannel {
    One(u32),
    Many(Vec<u32>),
    Empty(()),
}

impl PartyChannel {
    fn indices(&self) -> Vec<u32> {
        match self {
            Self::One(index) => vec![*index],
            Self::Many(indices) => indices.clone(),
            Self::Empty(()) => Vec::new(),
        }
    }
}

/// A reference represented by one index or an array of indices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IndexReferences {
    One(u32),
    Many(Vec<u32>),
}

impl IndexReferences {
    pub fn iter(&self) -> Box<dyn Iterator<Item = u32> + '_> {
        match self {
            Self::One(index) => Box::new(std::iter::once(*index)),
            Self::Many(indices) => Box::new(indices.iter().copied()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentEncoding {
    #[serde(rename = "base64url")]
    Base64Url,
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "none")]
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Disposition {
    NoAnswer,
    Congestion,
    Failed,
    Busy,
    HungUp,
    VoicemailNoMessage,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionId {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionIds {
    One(SessionId),
    Many(Vec<SessionId>),
    Channels(Vec<SessionIdChannel>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionIdChannel {
    One(SessionId),
    Many(Vec<SessionId>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartyHistory {
    pub party: u32,
    pub time: DateTime<Utc>,
    pub event: PartyHistoryEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartyHistoryEvent {
    Join,
    Drop,
    Hold,
    Unhold,
    Mute,
    Unmute,
    Keydown,
    Keyup,
}

/// One analysis performed on dialog or attachment data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    #[serde(rename = "type")]
    pub kind: String,
    pub vendor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialog: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<IndexReferences>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<ContentEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHashes>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// One ancillary attachment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    pub start: DateTime<Utc>,
    pub party: u32,
    pub dialog: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mediatype: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_value",
        skip_serializing_if = "Option::is_none"
    )]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding: Option<ContentEncoding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHashes>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// A redaction lineage reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Redacted {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHashes>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// An amendment lineage reference.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Amended {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHashes>,
    #[serde(default, flatten)]
    pub extra: ExtraFields,
}

/// One content hash or several hashes produced by different algorithms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentHashes {
    One(String),
    Many(Vec<String>),
}

impl ContentHashes {
    pub fn iter(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        match self {
            Self::One(hash) => Box::new(std::iter::once(hash.as_str())),
            Self::Many(hashes) => Box::new(hashes.iter().map(String::as_str)),
        }
    }
}

impl From<String> for ContentHashes {
    fn from(value: String) -> Self {
        Self::One(value)
    }
}

impl Vcon {
    /// Create a new vCon with syntax `0.4.0`, a UUIDv8, and the current time.
    pub fn new_now() -> Self {
        let random = Uuid::new_v4().into_bytes();
        Self {
            vcon: Some("0.4.0".into()),
            uuid: Uuid::new_v8(random),
            extensions: Vec::new(),
            critical: Vec::new(),
            created_at: Utc::now(),
            updated_at: None,
            subject: None,
            redacted: None,
            amended: None,
            parties: Vec::new(),
            dialog: Vec::new(),
            analysis: Vec::new(),
            attachments: Vec::new(),
            extra: ExtraFields::new(),
        }
    }

    /// Validate core cross-field constraints and every internal index.
    pub fn validate(&self) -> Result<(), VconError> {
        self.validate_extensions(None)
    }

    /// Validate and additionally reject critical extensions not included
    /// in `supported_extensions`.
    pub fn validate_with_supported_extensions<'a>(
        &self,
        supported_extensions: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), VconError> {
        let supported: HashSet<&str> = supported_extensions.into_iter().collect();
        self.validate_extensions(Some(&supported))
    }

    fn validate_extensions(&self, supported: Option<&HashSet<&str>>) -> Result<(), VconError> {
        if self
            .vcon
            .as_deref()
            .is_some_and(|version| version != "0.4.0")
        {
            return Err(invalid("vcon must be \"0.4.0\" when present"));
        }
        if self.redacted.is_some() && self.amended.is_some() {
            return Err(invalid("redacted and amended are mutually exclusive"));
        }

        reject_duplicates("extensions", &self.extensions)?;
        reject_duplicates("critical", &self.critical)?;
        if self.validate_extension_parameters()? && self.extensions.is_empty() {
            return Err(invalid(
                "extension parameters require at least one declaration in extensions",
            ));
        }
        for critical in &self.critical {
            if !self.extensions.contains(critical) {
                return Err(invalid(format!(
                    "critical extension {critical:?} is not declared in extensions"
                )));
            }
            if supported.is_some_and(|set| !set.contains(critical.as_str())) {
                return Err(invalid(format!(
                    "unsupported critical extension {critical:?}"
                )));
            }
        }

        if let Some(redacted) = &self.redacted {
            if redacted.kind.trim().is_empty() {
                return Err(invalid("redacted.type must not be empty"));
            }
            if redacted.uuid == Some(self.uuid) {
                return Err(invalid(
                    "redacted.uuid must reference a prior vCon, not this vCon",
                ));
            }
            validate_external(
                "redacted",
                None,
                None,
                None,
                redacted.url.as_deref(),
                redacted.content_hash.as_ref(),
            )?;
        }
        if let Some(amended) = &self.amended {
            if amended.uuid.is_none() && amended.url.is_none() {
                return Err(invalid(
                    "amended must reference its predecessor by uuid or url",
                ));
            }
            if amended.uuid == Some(self.uuid) {
                return Err(invalid(
                    "amended.uuid must reference a prior vCon, not this vCon",
                ));
            }
            validate_external(
                "amended",
                None,
                None,
                None,
                amended.url.as_deref(),
                amended.content_hash.as_ref(),
            )?;
        }

        for (index, party) in self.parties.iter().enumerate() {
            if let Some(did) = &party.did {
                validate_did_uri(&format!("parties[{index}].did"), did)?;
            }
        }

        for (index, dialog) in self.dialog.iter().enumerate() {
            dialog.validate(index, self.parties.len(), &self.dialog)?;
        }
        for (index, dialog) in self.dialog.iter().enumerate() {
            if dialog.kind == DialogKind::RecordingSet {
                for recording in &dialog.recordings {
                    let target = &self.dialog[*recording as usize];
                    if target.kind != DialogKind::Recording {
                        return Err(invalid(format!(
                            "dialog[{index}].recordings index {recording} does not reference a recording dialog"
                        )));
                    }
                    if target
                        .recording_set
                        .is_some_and(|recording_set| recording_set != index as u32)
                    {
                        return Err(invalid(format!(
                            "dialog[{recording}].recording_set refers to a different recording-set"
                        )));
                    }
                }
            }
            if let Some(recording_set) = dialog.recording_set {
                let target = &self.dialog[recording_set as usize];
                if target.kind != DialogKind::RecordingSet {
                    return Err(invalid(format!(
                        "dialog[{index}].recording_set index {recording_set} does not reference a recording-set dialog"
                    )));
                }
                if !target.recordings.contains(&(index as u32)) {
                    return Err(invalid(format!(
                        "dialog[{recording_set}].recordings must refer back to recording dialog[{index}]"
                    )));
                }
            }
        }
        for (index, analysis) in self.analysis.iter().enumerate() {
            analysis.validate(index, self.dialog.len(), self.attachments.len())?;
        }
        for (index, attachment) in self.attachments.iter().enumerate() {
            attachment.validate(index, self.parties.len(), self.dialog.len())?;
        }
        Ok(())
    }

    fn validate_extension_parameters(&self) -> Result<bool, VconError> {
        let mut present = validate_extra("vcon", &self.extra, VCON_CORE_FIELDS)?;
        if let Some(redacted) = &self.redacted {
            present |= validate_extra("redacted", &redacted.extra, REDACTED_CORE_FIELDS)?;
        }
        if let Some(amended) = &self.amended {
            present |= validate_extra("amended", &amended.extra, AMENDED_CORE_FIELDS)?;
        }
        for (index, party) in self.parties.iter().enumerate() {
            let path = format!("parties[{index}]");
            present |= validate_extra(&path, &party.extra, PARTY_CORE_FIELDS)?;
            if let Some(address) = &party.civicaddress {
                present |= validate_extra(
                    &format!("{path}.civicaddress"),
                    &address.extra,
                    CIVIC_ADDRESS_CORE_FIELDS,
                )?;
            }
        }
        for (index, dialog) in self.dialog.iter().enumerate() {
            let path = format!("dialog[{index}]");
            present |= validate_extra(&path, &dialog.extra, DIALOG_CORE_FIELDS)?;
            if let Some(session_ids) = &dialog.session_id {
                present |= validate_session_id_extras(&format!("{path}.session_id"), session_ids)?;
            }
            for (history_index, event) in dialog.party_history.iter().enumerate() {
                present |= validate_extra(
                    &format!("{path}.party_history[{history_index}]"),
                    &event.extra,
                    PARTY_HISTORY_CORE_FIELDS,
                )?;
            }
        }
        for (index, analysis) in self.analysis.iter().enumerate() {
            present |= validate_extra(
                &format!("analysis[{index}]"),
                &analysis.extra,
                ANALYSIS_CORE_FIELDS,
            )?;
        }
        for (index, attachment) in self.attachments.iter().enumerate() {
            present |= validate_extra(
                &format!("attachments[{index}]"),
                &attachment.extra,
                ATTACHMENT_CORE_FIELDS,
            )?;
        }
        Ok(present)
    }
}

impl Default for Vcon {
    fn default() -> Self {
        Self::new_now()
    }
}

impl Dialog {
    fn validate(
        &self,
        index: usize,
        party_count: usize,
        dialogs: &[Dialog],
    ) -> Result<(), VconError> {
        let path = format!("dialog[{index}]");
        let dialog_count = dialogs.len();
        if self
            .duration
            .is_some_and(|duration| !duration.is_finite() || duration < 0.0)
        {
            return Err(invalid(format!(
                "{path}.duration must be finite and non-negative"
            )));
        }
        if let Some(parties) = &self.parties {
            validate_indices(&format!("{path}.parties"), parties.iter(), party_count)?;
        }
        if let Some(originator) = self.originator {
            validate_index(&format!("{path}.originator"), originator, party_count)?;
        }
        if let Some(session_ids) = &self.session_id {
            validate_session_id_shape(
                &format!("{path}.session_id"),
                self.parties.as_ref(),
                session_ids,
            )?;
            validate_session_id_values(&format!("{path}.session_id"), session_ids)?;
        }
        for event in &self.party_history {
            validate_index(
                &format!("{path}.party_history.party"),
                event.party,
                party_count,
            )?;
            if matches!(
                event.event,
                PartyHistoryEvent::Keydown | PartyHistoryEvent::Keyup
            ) && event.button.as_deref().is_none_or(str::is_empty)
            {
                return Err(invalid(format!(
                    "{path}.party_history button is required for key events"
                )));
            }
        }

        let has_content = self.body.is_some()
            || self.encoding.is_some()
            || self.url.is_some()
            || self.content_hash.is_some()
            || self.mediatype.is_some()
            || self.filename.is_some();
        validate_mediatype(&path, self.mediatype.as_deref())?;
        match self.kind {
            DialogKind::Recording | DialogKind::Text => {
                validate_external(
                    &path,
                    self.body.as_ref(),
                    self.encoding,
                    self.mediatype.as_deref(),
                    self.url.as_deref(),
                    self.content_hash.as_ref(),
                )?;
                if self.kind == DialogKind::Text && self.recording_set.is_some() {
                    return Err(invalid(format!(
                        "{path}.recording_set is only valid for recording"
                    )));
                }
            }
            DialogKind::Incomplete | DialogKind::Transfer | DialogKind::RecordingSet => {
                if has_content {
                    return Err(invalid(format!(
                        "{path} must not contain dialog content for type {:?}",
                        self.kind
                    )));
                }
            }
        }

        if self.kind != DialogKind::RecordingSet && !self.recordings.is_empty() {
            return Err(invalid(format!(
                "{path}.recordings is only valid for recording-set"
            )));
        }
        if self.kind == DialogKind::Incomplete && self.disposition.is_none() {
            return Err(invalid(format!(
                "{path}.disposition is required for incomplete dialogs"
            )));
        }
        if self.kind != DialogKind::Incomplete && self.disposition.is_some() {
            return Err(invalid(format!(
                "{path}.disposition is only valid for incomplete dialogs"
            )));
        }
        if self.kind == DialogKind::RecordingSet {
            if self.recordings.is_empty() {
                return Err(invalid(format!(
                    "{path}.recordings is required for recording-set dialogs"
                )));
            }
            validate_indices(
                &format!("{path}.recordings"),
                self.recordings.iter().copied(),
                dialog_count,
            )?;
        }
        if let Some(recording_set) = self.recording_set {
            if self.kind != DialogKind::Recording {
                return Err(invalid(format!(
                    "{path}.recording_set is only valid for recording dialogs"
                )));
            }
            validate_index(
                &format!("{path}.recording_set"),
                recording_set,
                dialog_count,
            )?;
        }

        let transfer_fields_present = self.transferee.is_some()
            || self.transferor.is_some()
            || self.transfer_target.is_some()
            || self.original.is_some()
            || self.consultation.is_some()
            || self.target_dialog.is_some();
        if self.kind != DialogKind::Transfer && transfer_fields_present {
            return Err(invalid(format!(
                "{path} transfer fields are only valid for transfer dialogs"
            )));
        }
        if self.kind == DialogKind::Transfer {
            if self.parties.is_some() || self.originator.is_some() {
                return Err(invalid(format!(
                    "{path} transfer dialogs must not contain parties or originator"
                )));
            }
            if let Some(value) = self.transferee {
                validate_index(&format!("{path}.transferee"), value, party_count)?;
            }
            if let Some(value) = self.transferor {
                validate_index(&format!("{path}.transferor"), value, party_count)?;
            }
            if let Some(value) = &self.transfer_target {
                validate_indices(
                    &format!("{path}.transfer_target"),
                    value.iter(),
                    party_count,
                )?;
            }
            for (name, value) in [
                ("original", self.original.as_ref()),
                ("consultation", self.consultation.as_ref()),
                ("target_dialog", self.target_dialog.as_ref()),
            ] {
                if let Some(value) = value {
                    validate_indices(&format!("{path}.{name}"), value.iter(), dialog_count)?;
                }
            }
            if let Some(value) = &self.original {
                validate_dialog_kinds(
                    &format!("{path}.original"),
                    value,
                    dialogs,
                    &[DialogKind::Recording, DialogKind::Text],
                )?;
            }
            for (name, value) in [
                ("consultation", self.consultation.as_ref()),
                ("target_dialog", self.target_dialog.as_ref()),
            ] {
                if let Some(value) = value {
                    validate_dialog_kinds(
                        &format!("{path}.{name}"),
                        value,
                        dialogs,
                        &[
                            DialogKind::Recording,
                            DialogKind::Text,
                            DialogKind::Incomplete,
                        ],
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl Analysis {
    fn validate(
        &self,
        index: usize,
        dialog_count: usize,
        attachment_count: usize,
    ) -> Result<(), VconError> {
        let path = format!("analysis[{index}]");
        if self.kind.trim().is_empty() {
            return Err(invalid(format!("{path}.type must not be empty")));
        }
        if self.vendor.trim().is_empty() {
            return Err(invalid(format!("{path}.vendor must not be empty")));
        }
        if let Some(dialog) = &self.dialog {
            validate_indices(&format!("{path}.dialog"), dialog.iter(), dialog_count)?;
        }
        if let Some(attachment) = &self.attachment {
            validate_indices(
                &format!("{path}.attachment"),
                attachment.iter(),
                attachment_count,
            )?;
        }
        validate_mediatype(&path, self.mediatype.as_deref())?;
        validate_external(
            &path,
            self.body.as_ref(),
            self.encoding,
            self.mediatype.as_deref(),
            self.url.as_deref(),
            self.content_hash.as_ref(),
        )
    }
}

impl Attachment {
    fn validate(
        &self,
        index: usize,
        party_count: usize,
        dialog_count: usize,
    ) -> Result<(), VconError> {
        let path = format!("attachments[{index}]");
        validate_index(&format!("{path}.party"), self.party, party_count)?;
        validate_index(&format!("{path}.dialog"), self.dialog, dialog_count)?;
        validate_mediatype(&path, self.mediatype.as_deref())?;
        validate_external(
            &path,
            self.body.as_ref(),
            self.encoding,
            self.mediatype.as_deref(),
            self.url.as_deref(),
            self.content_hash.as_ref(),
        )
    }
}

fn validate_external(
    path: &str,
    body: Option<&Value>,
    encoding: Option<ContentEncoding>,
    mediatype: Option<&str>,
    url: Option<&str>,
    hashes: Option<&ContentHashes>,
) -> Result<(), VconError> {
    if encoding.is_some() && body.is_none() {
        return Err(invalid(format!(
            "{path}.encoding cannot be present without {path}.body"
        )));
    }
    let body_requires_encoding = match body {
        Some(Value::String(value)) => !value.is_empty(),
        Some(_) => true,
        None => false,
    };
    if body_requires_encoding && encoding.is_none() {
        return Err(invalid(format!(
            "{path}.encoding is required for a non-empty body"
        )));
    }
    if body.is_some() && mediatype.is_none_or(str::is_empty) {
        return Err(invalid(format!(
            "{path}.mediatype is required for inline content"
        )));
    }
    if body.is_some() && url.is_some() {
        return Err(invalid(format!(
            "{path} cannot contain both inline body and external url"
        )));
    }
    if let (Some(body), Some(encoding)) = (body, encoding) {
        match encoding {
            ContentEncoding::Json => {}
            ContentEncoding::None => {
                if !body.is_string() {
                    return Err(invalid(format!(
                        "{path}.body must be a string when encoding is none"
                    )));
                }
            }
            ContentEncoding::Base64Url => {
                let encoded = body.as_str().ok_or_else(|| {
                    invalid(format!(
                        "{path}.body must be a string when encoding is base64url"
                    ))
                })?;
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(encoded)
                    .map_err(|_| invalid(format!("{path}.body is not valid base64url")))?;
            }
        }
    }
    if let Some(url) = url {
        if !is_https_url(url) {
            return Err(invalid(format!("{path}.url must use HTTPS")));
        }
        if hashes.is_none() {
            return Err(invalid(format!(
                "{path}.content_hash is required when url is present"
            )));
        }
    }
    if hashes.is_some() && url.is_none() {
        return Err(invalid(format!(
            "{path}.url is required when content_hash is present"
        )));
    }
    if let Some(hashes) = hashes {
        let values: Vec<&str> = hashes.iter().collect();
        if values.is_empty() {
            return Err(invalid(format!("{path}.content_hash must not be empty")));
        }
        for hash in values {
            validate_content_hash(path, hash)?;
        }
    }
    Ok(())
}

fn validate_mediatype(path: &str, mediatype: Option<&str>) -> Result<(), VconError> {
    if let Some(value) = mediatype {
        value
            .parse::<mime::Mime>()
            .map_err(|_| invalid(format!("{path}.mediatype is not a valid media type")))?;
    }
    Ok(())
}

fn validate_session_id_shape(
    path: &str,
    parties: Option<&PartyIndices>,
    session_ids: &SessionIds,
) -> Result<(), VconError> {
    let Some(parties) = parties else {
        if matches!(session_ids, SessionIds::One(_)) {
            return Ok(());
        }
        return Err(invalid(format!(
            "{path} arrays require a parties array to correlate with"
        )));
    };

    match session_ids {
        SessionIds::One(_) => Ok(()),
        SessionIds::Many(values) => {
            let correlates = match parties {
                PartyIndices::One(_) => false,
                PartyIndices::Many(indices) => values.len() == indices.len(),
                PartyIndices::Channels(channels) => {
                    values.len() == channels.len()
                        && channels
                            .iter()
                            .zip(values)
                            .all(|(channel, session_id)| match channel {
                                PartyChannel::One(_) => true,
                                PartyChannel::Empty(()) => {
                                    session_id.local.is_none()
                                        && session_id.remote.is_none()
                                        && session_id.extra.is_empty()
                                }
                                PartyChannel::Many(_) => false,
                            })
                }
            };
            if correlates {
                Ok(())
            } else {
                Err(invalid(format!(
                    "{path} array shape does not correlate with parties"
                )))
            }
        }
        SessionIds::Channels(channels) => {
            let PartyIndices::Channels(party_channels) = parties else {
                return Err(invalid(format!(
                    "{path} channel shape requires channel-oriented parties"
                )));
            };
            if channels.len() != party_channels.len() {
                return Err(invalid(format!(
                    "{path} channel count does not correlate with parties"
                )));
            }
            for (party_channel, session_channel) in party_channels.iter().zip(channels) {
                let correlates = match (party_channel, session_channel) {
                    (PartyChannel::One(_), SessionIdChannel::One(_)) => true,
                    (PartyChannel::Many(parties), SessionIdChannel::Many(sessions)) => {
                        parties.len() == sessions.len()
                    }
                    (PartyChannel::Empty(()), SessionIdChannel::One(session_id)) => {
                        session_id.local.is_none()
                            && session_id.remote.is_none()
                            && session_id.extra.is_empty()
                    }
                    _ => false,
                };
                if !correlates {
                    return Err(invalid(format!(
                        "{path} channel shape does not correlate with parties"
                    )));
                }
            }
            Ok(())
        }
    }
}

fn validate_session_id_values(path: &str, session_ids: &SessionIds) -> Result<(), VconError> {
    match session_ids {
        SessionIds::One(session_id) => validate_session_id(path, session_id),
        SessionIds::Many(session_ids) => {
            for (index, session_id) in session_ids.iter().enumerate() {
                validate_session_id(&format!("{path}[{index}]"), session_id)?;
            }
            Ok(())
        }
        SessionIds::Channels(channels) => {
            for (channel_index, channel) in channels.iter().enumerate() {
                let channel_path = format!("{path}[{channel_index}]");
                match channel {
                    SessionIdChannel::One(session_id) => {
                        validate_session_id(&channel_path, session_id)?;
                    }
                    SessionIdChannel::Many(session_ids) => {
                        for (index, session_id) in session_ids.iter().enumerate() {
                            validate_session_id(&format!("{channel_path}[{index}]"), session_id)?;
                        }
                    }
                }
            }
            Ok(())
        }
    }
}

fn validate_session_id(path: &str, session_id: &SessionId) -> Result<(), VconError> {
    for (name, value) in [
        ("local", session_id.local.as_deref()),
        ("remote", session_id.remote.as_deref()),
    ] {
        if let Some(value) = value {
            let valid = value.len() == 32
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
            if !valid {
                return Err(invalid(format!(
                    "{path}.{name} must be a 32-character lowercase RFC 7989 session identifier"
                )));
            }
        }
    }
    Ok(())
}

fn validate_session_id_extras(path: &str, session_ids: &SessionIds) -> Result<bool, VconError> {
    let mut present = false;
    match session_ids {
        SessionIds::One(session_id) => {
            present |= validate_extra(path, &session_id.extra, SESSION_ID_CORE_FIELDS)?;
        }
        SessionIds::Many(session_ids) => {
            for (index, session_id) in session_ids.iter().enumerate() {
                present |= validate_extra(
                    &format!("{path}[{index}]"),
                    &session_id.extra,
                    SESSION_ID_CORE_FIELDS,
                )?;
            }
        }
        SessionIds::Channels(channels) => {
            for (channel_index, channel) in channels.iter().enumerate() {
                let channel_path = format!("{path}[{channel_index}]");
                match channel {
                    SessionIdChannel::One(session_id) => {
                        present |= validate_extra(
                            &channel_path,
                            &session_id.extra,
                            SESSION_ID_CORE_FIELDS,
                        )?;
                    }
                    SessionIdChannel::Many(session_ids) => {
                        for (index, session_id) in session_ids.iter().enumerate() {
                            present |= validate_extra(
                                &format!("{channel_path}[{index}]"),
                                &session_id.extra,
                                SESSION_ID_CORE_FIELDS,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(present)
}

fn validate_extra(
    path: &str,
    extra: &ExtraFields,
    core_fields: &[&str],
) -> Result<bool, VconError> {
    for name in extra.keys() {
        if name.trim().is_empty() {
            return Err(invalid(format!(
                "{path} contains an empty extension parameter name"
            )));
        }
        if path == "vcon" && name == "group" {
            return Err(invalid(format!(
                "{path}.group is reserved and cannot be emitted as an extension parameter"
            )));
        }
        if core_fields.contains(&name.as_str()) {
            return Err(invalid(format!(
                "{path}.{name} collides with a core property"
            )));
        }
    }
    Ok(!extra.is_empty())
}

fn validate_dialog_kinds(
    path: &str,
    references: &IndexReferences,
    dialogs: &[Dialog],
    allowed: &[DialogKind],
) -> Result<(), VconError> {
    for index in references.iter() {
        let target = &dialogs[index as usize];
        if !allowed.contains(&target.kind) {
            return Err(invalid(format!(
                "{path} index {index} references invalid dialog type {:?}",
                target.kind
            )));
        }
    }
    Ok(())
}

fn is_https_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

fn validate_did_uri(path: &str, value: &str) -> Result<(), VconError> {
    let parsed =
        Url::parse(value).map_err(|_| invalid(format!("{path} is not a valid DID URI")))?;
    if parsed.scheme() != "did" {
        return Err(invalid(format!("{path} must use the did URI scheme")));
    }
    let (method, identifier) = value
        .strip_prefix("did:")
        .and_then(|value| value.split_once(':'))
        .ok_or_else(|| invalid(format!("{path} is missing a DID method or identifier")))?;
    if method.is_empty()
        || !method
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        return Err(invalid(format!(
            "{path} method must contain only lowercase letters or digits"
        )));
    }
    if !valid_did_method_specific_id(identifier) {
        return Err(invalid(format!(
            "{path} contains an invalid DID method-specific identifier"
        )));
    }
    Ok(())
}

fn valid_did_method_specific_id(identifier: &str) -> bool {
    let bytes = identifier.as_bytes();
    if bytes.is_empty() || bytes.last() == Some(&b':') {
        return false;
    }
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':') {
            index += 1;
            continue;
        }
        if byte == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            index += 3;
            continue;
        }
        return false;
    }
    true
}

fn validate_content_hash(path: &str, hash: &str) -> Result<(), VconError> {
    let (algorithm, digest) = hash
        .split_once('-')
        .ok_or_else(|| invalid(format!("{path}.content_hash has invalid format")))?;
    if algorithm != "sha512" {
        return Err(invalid(format!(
            "{path}.content_hash algorithm {algorithm:?} is unsupported; use sha512"
        )));
    }
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(digest)
        .map_err(|_| invalid(format!("{path}.content_hash digest is not base64url")))?;
    if decoded.len() != 64 {
        return Err(invalid(format!(
            "{path}.content_hash SHA-512 digest must be 64 bytes"
        )));
    }
    Ok(())
}

fn validate_index(path: &str, index: u32, collection_len: usize) -> Result<(), VconError> {
    if usize::try_from(index)
        .ok()
        .is_none_or(|index| index >= collection_len)
    {
        return Err(invalid(format!(
            "{path} index {index} is out of bounds for length {collection_len}"
        )));
    }
    Ok(())
}

fn validate_indices(
    path: &str,
    indices: impl Iterator<Item = u32>,
    collection_len: usize,
) -> Result<(), VconError> {
    for index in indices {
        validate_index(path, index, collection_len)?;
    }
    Ok(())
}

fn reject_duplicates(path: &str, values: &[String]) -> Result<(), VconError> {
    let mut seen = HashSet::new();
    for value in values {
        if value.trim().is_empty() {
            return Err(invalid(format!("{path} names must not be empty")));
        }
        if !seen.insert(value) {
            return Err(invalid(format!("{path} contains duplicate {value:?}")));
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> VconError {
    VconError::Invalid(message.into())
}

fn deserialize_optional_value<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
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
