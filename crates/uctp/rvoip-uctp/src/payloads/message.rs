//! Message envelope payloads per CONVERSATION_PROTOCOL.md §9.

use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use rvoip_core::{DataMessage, DataMessageValidationError, DataReliability, MessageId};
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

const LEGACY_DATA_LABEL: &str = "rvoip-messages";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyEncoding {
    #[default]
    Utf8,
    Base64,
}

#[derive(Debug, Error)]
pub enum MessagePayloadError {
    #[error(transparent)]
    Validation(#[from] DataMessageValidationError),
    #[error("message.send body is not valid base64")]
    InvalidBase64,
    #[error("message.send UTF-8 body contains invalid UTF-8")]
    InvalidUtf8,
    #[error("UCTP message.send supports reliable ordered delivery only")]
    UnsupportedReliability,
}

/// `message.send` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct MessageSend {
    pub msg_id: String,
    /// Legacy presentation field. Receivers MUST derive authoritative sender
    /// identity from the authenticated Connection route; this string is never
    /// used for ownership or authorization.
    pub from: String,
    /// `["part_..."]` for specific recipients, or `"all"`.
    pub to: serde_json::Value,
    pub content_type: String,
    #[serde(default = "legacy_data_label")]
    pub label: String,
    #[serde(default)]
    pub reliability: DataReliability,
    /// String for `text/*` and `application/json`; base64 for binary;
    /// URL reference for large attachments.
    pub body: String,
    #[serde(default)]
    pub body_encoding: BodyEncoding,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// Threading: reply to a prior message in the same Conversation.
    #[serde(default)]
    pub in_reply_to_msg: Option<String>,
}

impl fmt::Debug for MessageSend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageSend")
            .field("recipient_shape_present", &!self.to.is_null())
            .field("content_type_present", &!self.content_type.is_empty())
            .field("label_present", &!self.label.is_empty())
            .field("reliability", &self.reliability)
            .field("body_bytes", &self.body.len())
            .field("body_encoding", &self.body_encoding)
            .field("attachment_count", &self.attachments.len())
            .field("in_reply_to_present", &self.in_reply_to_msg.is_some())
            .finish()
    }
}

impl MessageSend {
    pub fn from_data_message(
        message: &DataMessage,
        from: impl Into<String>,
        to: serde_json::Value,
    ) -> Result<Self, MessagePayloadError> {
        message.validate()?;
        ensure_uctp_reliability(&message.reliability)?;
        let (body, body_encoding) = if textual_content_type(&message.content_type) {
            (
                std::str::from_utf8(&message.bytes)
                    .map_err(|_| MessagePayloadError::InvalidUtf8)?
                    .to_owned(),
                BodyEncoding::Utf8,
            )
        } else {
            (
                base64::engine::general_purpose::STANDARD.encode(&message.bytes),
                BodyEncoding::Base64,
            )
        };
        Ok(Self {
            msg_id: message.message_id.to_string(),
            from: from.into(),
            to,
            content_type: message.content_type.clone(),
            label: message.label.clone(),
            reliability: message.reliability.clone(),
            body,
            body_encoding,
            attachments: Vec::new(),
            in_reply_to_msg: None,
        })
    }

    pub fn to_data_message(&self) -> Result<DataMessage, MessagePayloadError> {
        ensure_uctp_reliability(&self.reliability)?;
        let bytes = match self.body_encoding {
            BodyEncoding::Utf8 => Bytes::copy_from_slice(self.body.as_bytes()),
            BodyEncoding::Base64 => Bytes::from(
                base64::engine::general_purpose::STANDARD
                    .decode(&self.body)
                    .map_err(|_| MessagePayloadError::InvalidBase64)?,
            ),
        };
        let message = DataMessage {
            label: self.label.clone(),
            content_type: self.content_type.clone(),
            bytes,
            reliability: self.reliability.clone(),
            message_id: MessageId::from_string(self.msg_id.clone()),
        };
        message.validate()?;
        Ok(message)
    }
}

pub fn ensure_uctp_reliability(reliability: &DataReliability) -> Result<(), MessagePayloadError> {
    reliability.validate()?;
    if *reliability != DataReliability::ReliableOrdered {
        return Err(MessagePayloadError::UnsupportedReliability);
    }
    Ok(())
}

fn legacy_data_label() -> String {
    LEGACY_DATA_LABEL.into()
}

fn textual_content_type(content_type: &str) -> bool {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media_type.starts_with("text/")
        || media_type == "application/json"
        || media_type.ends_with("+json")
        || media_type == "application/xml"
        || media_type.ends_with("+xml")
}

/// `message.delivered` (S→C) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct MessageDelivered {
    pub msg_id: String,
    pub to_participant: String,
    pub delivered_at: DateTime<Utc>,
    #[serde(default)]
    pub via_connection: Option<String>,
}

/// `message.read` (bidi) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct MessageRead {
    pub msg_id: String,
    pub by_participant: String,
    pub read_at: DateTime<Utc>,
}

/// `message.history` (C→S) payload.
#[derive(Clone, Serialize, Deserialize)]
pub struct MessageHistory {
    #[serde(default)]
    pub since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub since_msg_id: Option<String>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub include_attachments: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub content_type: String,
    #[serde(default)]
    pub url: Option<String>,
    pub size_bytes: u64,
}

impl fmt::Debug for MessageDelivered {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageDelivered")
            .field("delivered_at", &self.delivered_at)
            .field("via_connection_present", &self.via_connection.is_some())
            .finish()
    }
}

impl fmt::Debug for MessageRead {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageRead")
            .field("read_at", &self.read_at)
            .finish()
    }
}

impl fmt::Debug for MessageHistory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MessageHistory")
            .field("since", &self.since)
            .field("until", &self.until)
            .field("since_message_present", &self.since_msg_id.is_some())
            .field("cursor_present", &self.cursor.is_some())
            .field("limit", &self.limit)
            .field("include_attachments", &self.include_attachments)
            .finish()
    }
}

impl fmt::Debug for Attachment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Attachment")
            .field("content_type_present", &!self.content_type.is_empty())
            .field("url_present", &self.url.is_some())
            .field("size_bytes", &self.size_bytes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_binary_messages_roundtrip_with_explicit_encoding() {
        let text = DataMessage::reliable("chat", "text/plain; charset=utf-8", "hello");
        let wire =
            MessageSend::from_data_message(&text, "part_a", serde_json::json!("all")).unwrap();
        assert_eq!(wire.body_encoding, BodyEncoding::Utf8);
        assert_eq!(wire.to_data_message().unwrap(), text);

        let binary = DataMessage::reliable(
            "blob",
            "application/octet-stream",
            Bytes::from_static(&[0, 255, 1, 254]),
        );
        let wire =
            MessageSend::from_data_message(&binary, "part_a", serde_json::json!("all")).unwrap();
        assert_eq!(wire.body_encoding, BodyEncoding::Base64);
        assert_eq!(wire.to_data_message().unwrap(), binary);
    }

    #[test]
    fn legacy_payload_defaults_to_reliable_utf8() {
        let wire: MessageSend = serde_json::from_value(serde_json::json!({
            "msg_id": "msg_legacy",
            "from": "part_a",
            "to": "all",
            "content_type": "text/plain",
            "body": "hello",
            "attachments": []
        }))
        .unwrap();
        assert_eq!(wire.label, LEGACY_DATA_LABEL);
        assert_eq!(wire.reliability, DataReliability::ReliableOrdered);
        assert_eq!(wire.body_encoding, BodyEncoding::Utf8);
        assert_eq!(wire.to_data_message().unwrap().bytes, "hello");
    }

    #[test]
    fn unsupported_reliability_and_bad_base64_are_explicit() {
        let mut message = DataMessage::reliable("chat", "text/plain", "hello");
        message.reliability = DataReliability::ReliableUnordered;
        assert!(matches!(
            MessageSend::from_data_message(&message, "part_a", serde_json::json!("all")),
            Err(MessagePayloadError::UnsupportedReliability)
        ));

        let wire = MessageSend {
            msg_id: "msg_bad".into(),
            from: "part_a".into(),
            to: serde_json::json!("all"),
            content_type: "application/octet-stream".into(),
            label: "blob".into(),
            reliability: DataReliability::ReliableOrdered,
            body: "%%%".into(),
            body_encoding: BodyEncoding::Base64,
            attachments: Vec::new(),
            in_reply_to_msg: None,
        };
        assert!(matches!(
            wire.to_data_message(),
            Err(MessagePayloadError::InvalidBase64)
        ));
    }
}
