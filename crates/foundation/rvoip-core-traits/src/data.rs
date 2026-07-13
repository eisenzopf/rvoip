//! Transport-neutral reliable/unreliable data messages.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

use crate::ids::MessageId;

/// Delivery contract requested by an application data message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum DataReliability {
    ReliableOrdered,
    ReliableUnordered,
    MaxRetransmits { ordered: bool, count: u16 },
    MaxLifetime { ordered: bool, milliseconds: u32 },
}

impl DataReliability {
    pub fn validate(&self) -> Result<(), DataMessageValidationError> {
        if let Self::MaxLifetime { milliseconds, .. } = self {
            if *milliseconds == 0 {
                return Err(DataMessageValidationError::ZeroLifetime);
            }
            if *milliseconds > u16::MAX as u32 {
                return Err(DataMessageValidationError::LifetimeTooLarge {
                    milliseconds: *milliseconds,
                    maximum: u16::MAX as u32,
                });
            }
        }
        Ok(())
    }
}

impl Default for DataReliability {
    fn default() -> Self {
        Self::ReliableOrdered
    }
}

/// A data-plane message that can be mapped to a WebRTC DataChannel, UCTP
/// `message.send`, SIP MESSAGE, or an application-owned metadata transport.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DataMessage {
    pub label: String,
    pub content_type: String,
    pub bytes: Bytes,
    #[serde(default)]
    pub reliability: DataReliability,
    pub message_id: MessageId,
}

impl fmt::Debug for DataMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataMessage")
            .field("label_present", &!self.label.is_empty())
            .field("label_bytes", &self.label.len())
            .field("content_type_present", &!self.content_type.is_empty())
            .field("content_type_bytes", &self.content_type.len())
            .field("body_bytes", &self.bytes.len())
            .field("reliability", &self.reliability)
            .field("message_id_present", &!self.message_id.as_str().is_empty())
            .finish()
    }
}

pub const MAX_DATA_LABEL_BYTES: usize = 128;
pub const MAX_CONTENT_TYPE_BYTES: usize = 255;
pub const MAX_DATA_MESSAGE_BYTES: usize = 64 * 1024;
pub const MAX_DATA_MESSAGE_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DataMessageValidationError {
    #[error("data message label must not be empty")]
    EmptyLabel,
    #[error("data message label is {actual} bytes; maximum is {maximum}")]
    LabelTooLong { actual: usize, maximum: usize },
    #[error("data message label contains a control character")]
    LabelContainsControl,
    #[error("data message content type must not be empty")]
    EmptyContentType,
    #[error("data message content type is {actual} bytes; maximum is {maximum}")]
    ContentTypeTooLong { actual: usize, maximum: usize },
    #[error("data message content type contains a control character")]
    ContentTypeContainsControl,
    #[error("data message content type is not a valid MIME media type")]
    InvalidContentType,
    #[error("data message body is {actual} bytes; maximum is {maximum}")]
    BodyTooLarge { actual: usize, maximum: usize },
    #[error("data message ID must not be empty")]
    EmptyMessageId,
    #[error("data message ID is {actual} bytes; maximum is {maximum}")]
    MessageIdTooLong { actual: usize, maximum: usize },
    #[error("data message ID contains a control character")]
    MessageIdContainsControl,
    #[error("max-lifetime reliability must be greater than zero milliseconds")]
    ZeroLifetime,
    #[error("max-lifetime reliability is {milliseconds}ms; maximum is {maximum}ms")]
    LifetimeTooLarge { milliseconds: u32, maximum: u32 },
}

impl DataMessage {
    pub fn reliable(
        label: impl Into<String>,
        content_type: impl Into<String>,
        bytes: impl Into<Bytes>,
    ) -> Self {
        Self {
            label: label.into(),
            content_type: content_type.into(),
            bytes: bytes.into(),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::new(),
        }
    }

    pub fn try_new(
        label: impl Into<String>,
        content_type: impl Into<String>,
        bytes: impl Into<Bytes>,
        reliability: DataReliability,
        message_id: MessageId,
    ) -> Result<Self, DataMessageValidationError> {
        let message = Self {
            label: label.into(),
            content_type: content_type.into(),
            bytes: bytes.into(),
            reliability,
            message_id,
        };
        message.validate()?;
        Ok(message)
    }

    pub fn validate(&self) -> Result<(), DataMessageValidationError> {
        if self.label.is_empty() {
            return Err(DataMessageValidationError::EmptyLabel);
        }
        if self.label.len() > MAX_DATA_LABEL_BYTES {
            return Err(DataMessageValidationError::LabelTooLong {
                actual: self.label.len(),
                maximum: MAX_DATA_LABEL_BYTES,
            });
        }
        if self.label.chars().any(char::is_control) {
            return Err(DataMessageValidationError::LabelContainsControl);
        }

        if self.content_type.is_empty() {
            return Err(DataMessageValidationError::EmptyContentType);
        }
        if self.content_type.len() > MAX_CONTENT_TYPE_BYTES {
            return Err(DataMessageValidationError::ContentTypeTooLong {
                actual: self.content_type.len(),
                maximum: MAX_CONTENT_TYPE_BYTES,
            });
        }
        if self.content_type.chars().any(char::is_control) {
            return Err(DataMessageValidationError::ContentTypeContainsControl);
        }
        if !valid_content_type(&self.content_type) {
            return Err(DataMessageValidationError::InvalidContentType);
        }

        if self.bytes.len() > MAX_DATA_MESSAGE_BYTES {
            return Err(DataMessageValidationError::BodyTooLarge {
                actual: self.bytes.len(),
                maximum: MAX_DATA_MESSAGE_BYTES,
            });
        }
        if self.message_id.as_str().is_empty() {
            return Err(DataMessageValidationError::EmptyMessageId);
        }
        if self.message_id.as_str().len() > MAX_DATA_MESSAGE_ID_BYTES {
            return Err(DataMessageValidationError::MessageIdTooLong {
                actual: self.message_id.as_str().len(),
                maximum: MAX_DATA_MESSAGE_ID_BYTES,
            });
        }
        if self.message_id.as_str().chars().any(char::is_control) {
            return Err(DataMessageValidationError::MessageIdContainsControl);
        }
        self.reliability.validate()
    }
}

fn valid_content_type(value: &str) -> bool {
    let mut sections = value.split(';');
    let media_type = sections.next().unwrap_or_default().trim();
    let Some((type_name, subtype)) = media_type.split_once('/') else {
        return false;
    };
    if !mime_token(type_name) || !mime_token(subtype) {
        return false;
    }
    sections.all(|parameter| {
        let parameter = parameter.trim();
        !parameter.is_empty()
            && parameter
                .chars()
                .all(|character| character.is_ascii() && !character.is_ascii_control())
    })
}

fn mime_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(label: &str, content_type: &str, body: Vec<u8>) -> DataMessage {
        DataMessage {
            label: label.into(),
            content_type: content_type.into(),
            bytes: Bytes::from(body),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::new(),
        }
    }

    #[test]
    fn accepts_boundary_sized_valid_message() {
        let value = message(
            &"l".repeat(MAX_DATA_LABEL_BYTES),
            "application/vnd.bridgefu+json; charset=utf-8",
            vec![0; MAX_DATA_MESSAGE_BYTES],
        );
        assert_eq!(value.validate(), Ok(()));
    }

    #[test]
    fn rejects_invalid_labels() {
        assert_eq!(
            message("", "text/plain", vec![]).validate(),
            Err(DataMessageValidationError::EmptyLabel)
        );
        assert!(matches!(
            message(&"x".repeat(129), "text/plain", vec![]).validate(),
            Err(DataMessageValidationError::LabelTooLong { .. })
        ));
        assert_eq!(
            message("bad\nlabel", "text/plain", vec![]).validate(),
            Err(DataMessageValidationError::LabelContainsControl)
        );
    }

    #[test]
    fn rejects_invalid_content_types() {
        assert_eq!(
            message("label", "", vec![]).validate(),
            Err(DataMessageValidationError::EmptyContentType)
        );
        for invalid in [
            "plain",
            "text/",
            "/plain",
            "text /plain",
            "text/plain\r\nx:y",
        ] {
            assert!(message("label", invalid, vec![]).validate().is_err());
        }
        assert!(matches!(
            message("label", &"x".repeat(256), vec![]).validate(),
            Err(DataMessageValidationError::ContentTypeTooLong { .. })
        ));
    }

    #[test]
    fn rejects_oversized_body_and_invalid_lifetime() {
        assert!(matches!(
            message("label", "application/octet-stream", vec![0; 65_537]).validate(),
            Err(DataMessageValidationError::BodyTooLarge { .. })
        ));
        assert_eq!(
            DataReliability::MaxLifetime {
                ordered: true,
                milliseconds: 0,
            }
            .validate(),
            Err(DataMessageValidationError::ZeroLifetime)
        );
        assert!(matches!(
            DataReliability::MaxLifetime {
                ordered: false,
                milliseconds: 65_536,
            }
            .validate(),
            Err(DataMessageValidationError::LifetimeTooLarge { .. })
        ));
        assert_eq!(
            DataReliability::MaxRetransmits {
                ordered: false,
                count: 0,
            }
            .validate(),
            Ok(())
        );
    }

    #[test]
    fn rejects_invalid_message_ids() {
        let mut value = message("label", "text/plain", vec![]);
        value.message_id = MessageId::from_string("");
        assert_eq!(
            value.validate(),
            Err(DataMessageValidationError::EmptyMessageId)
        );

        value.message_id = MessageId::from_string("x".repeat(MAX_DATA_MESSAGE_ID_BYTES + 1));
        assert!(matches!(
            value.validate(),
            Err(DataMessageValidationError::MessageIdTooLong { .. })
        ));

        value.message_id = MessageId::from_string("msg_bad\nvalue");
        assert_eq!(
            value.validate(),
            Err(DataMessageValidationError::MessageIdContainsControl)
        );
    }

    #[test]
    fn debug_keeps_message_content_and_identifiers_private() {
        const CANARY: &str = "data-message-diagnostic-canary\r\nAuthorization: exposed";
        let value = DataMessage {
            label: CANARY.into(),
            content_type: "application/octet-stream".into(),
            bytes: Bytes::copy_from_slice(CANARY.as_bytes()),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::from_string(CANARY),
        };
        let rendered = format!("{value:?}");
        assert!(!rendered.contains(CANARY), "message leaked: {rendered}");
        assert_eq!(value.label, CANARY);
        assert_eq!(value.bytes.as_ref(), CANARY.as_bytes());
        assert_eq!(value.message_id.as_str(), CANARY);
    }
}
