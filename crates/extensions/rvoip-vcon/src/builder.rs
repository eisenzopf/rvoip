//! Fluent construction helpers for unsigned vCons.

use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::encode_base64url;
use crate::types::{
    Analysis, Attachment, ContentEncoding, Dialog, DialogKind, Disposition, IndexReferences, Party,
    PartyIndices, Vcon, VconError,
};

/// Fluent builder for [`Vcon`] documents.
#[derive(Clone, Debug)]
pub struct VconBuilder {
    vcon: Vcon,
}

impl VconBuilder {
    /// Start a `0.4.0` vCon with a UUIDv8 and current creation time.
    pub fn new() -> Self {
        Self {
            vcon: Vcon::new_now(),
        }
    }

    pub fn with_uuid(mut self, uuid: Uuid) -> Self {
        self.vcon.uuid = uuid;
        self
    }

    pub fn created_at(mut self, created_at: DateTime<Utc>) -> Self {
        self.vcon.created_at = created_at;
        self
    }

    pub fn updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.vcon.updated_at = Some(updated_at);
        self
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.vcon.subject = Some(subject.into());
        self
    }

    /// Declare an extension, optionally as critical.
    pub fn extension(mut self, name: impl Into<String>, critical: bool) -> Self {
        let name = name.into();
        if !self.vcon.extensions.contains(&name) {
            self.vcon.extensions.push(name.clone());
        }
        if critical && !self.vcon.critical.contains(&name) {
            self.vcon.critical.push(name);
        }
        self
    }

    /// Add a top-level extension parameter while explicitly declaring its
    /// owning extension.
    pub fn extension_parameter(
        mut self,
        extension: impl Into<String>,
        parameter: impl Into<String>,
        value: Value,
        critical: bool,
    ) -> Self {
        self = self.extension(extension, critical);
        self.vcon.extra.insert(parameter.into(), value);
        self
    }

    /// Append one party and return its index.
    pub fn party(&mut self, party: Party) -> u32 {
        let index = self.vcon.parties.len() as u32;
        self.vcon.parties.push(party);
        index
    }

    pub fn with_party(mut self, party: Party) -> Self {
        self.vcon.parties.push(party);
        self
    }

    /// Append a generic dialog and return its index.
    pub fn add_dialog(&mut self, dialog: Dialog) -> u32 {
        let index = self.vcon.dialog.len() as u32;
        self.vcon.dialog.push(dialog);
        index
    }

    pub fn with_dialog(mut self, dialog: Dialog) -> Self {
        self.vcon.dialog.push(dialog);
        self
    }

    /// Append recording metadata. Duration is expressed in seconds.
    pub fn recording(
        mut self,
        start: DateTime<Utc>,
        duration_seconds: f64,
        parties: Vec<u32>,
        mediatype: impl Into<String>,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Recording,
            start,
            duration: Some(duration_seconds),
            parties: Some(PartyIndices::Many(parties)),
            mediatype: Some(mediatype.into()),
            ..Dialog::default()
        });
        self
    }

    /// Append a recording with inline binary content.
    pub fn recording_inline(
        mut self,
        start: DateTime<Utc>,
        duration_seconds: f64,
        parties: Vec<u32>,
        mediatype: impl Into<String>,
        body: impl AsRef<[u8]>,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Recording,
            start,
            duration: Some(duration_seconds),
            parties: Some(PartyIndices::Many(parties)),
            mediatype: Some(mediatype.into()),
            body: Some(Value::String(encode_base64url(body))),
            encoding: Some(ContentEncoding::Base64Url),
            ..Dialog::default()
        });
        self
    }

    /// Append one plain-text dialog. Unknown typing duration is omitted.
    pub fn text(mut self, start: DateTime<Utc>, party: u32, body: impl Into<String>) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Text,
            start,
            parties: Some(PartyIndices::One(party)),
            mediatype: Some("text/plain".into()),
            body: Some(Value::String(body.into())),
            encoding: Some(ContentEncoding::None),
            ..Dialog::default()
        });
        self
    }

    /// Append an incomplete call attempt.
    pub fn incomplete(
        mut self,
        start: DateTime<Utc>,
        parties: Vec<u32>,
        disposition: Disposition,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Incomplete,
            start,
            parties: Some(PartyIndices::Many(parties)),
            disposition: Some(disposition),
            ..Dialog::default()
        });
        self
    }

    /// Append transfer metadata. Detailed original/consultation/target
    /// dialog references can be supplied with [`Self::with_dialog`].
    pub fn transfer(
        mut self,
        start: DateTime<Utc>,
        transferor: u32,
        transferee: u32,
        transfer_target: IndexReferences,
    ) -> Self {
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::Transfer,
            start,
            transferor: Some(transferor),
            transferee: Some(transferee),
            transfer_target: Some(transfer_target),
            ..Dialog::default()
        });
        self
    }

    /// Append a recording-set and wire reciprocal `recording_set`
    /// references into each listed recording.
    pub fn recording_set(
        mut self,
        start: DateTime<Utc>,
        duration_seconds: f64,
        parties: Vec<u32>,
        recordings: Vec<u32>,
    ) -> Self {
        let set_index = self.vcon.dialog.len() as u32;
        for recording in &recordings {
            if let Some(dialog) = self.vcon.dialog.get_mut(*recording as usize) {
                dialog.recording_set = Some(set_index);
            }
        }
        self.vcon.dialog.push(Dialog {
            kind: DialogKind::RecordingSet,
            start,
            duration: Some(duration_seconds),
            parties: Some(PartyIndices::Many(parties)),
            recordings,
            ..Dialog::default()
        });
        self
    }

    pub fn analysis(mut self, analysis: Analysis) -> Self {
        self.vcon.analysis.push(analysis);
        self
    }

    pub fn add_analysis(&mut self, analysis: Analysis) -> u32 {
        let index = self.vcon.analysis.len() as u32;
        self.vcon.analysis.push(analysis);
        index
    }

    pub fn attachment(mut self, attachment: Attachment) -> Self {
        self.vcon.attachments.push(attachment);
        self
    }

    pub fn add_attachment(&mut self, attachment: Attachment) -> u32 {
        let index = self.vcon.attachments.len() as u32;
        self.vcon.attachments.push(attachment);
        index
    }

    /// Finish without validation, useful while a vCon is still being built.
    pub fn build(self) -> Vcon {
        self.vcon
    }

    /// Finish and enforce all current core semantic constraints.
    pub fn build_validated(self) -> Result<Vcon, VconError> {
        self.vcon.validate()?;
        Ok(self.vcon)
    }
}

impl Default for VconBuilder {
    fn default() -> Self {
        Self::new()
    }
}
