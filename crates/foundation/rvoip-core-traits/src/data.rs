//! Transport-neutral reliable/unreliable data messages.

use bytes::Bytes;
use serde::{Deserialize, Serialize};

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

impl Default for DataReliability {
    fn default() -> Self {
        Self::ReliableOrdered
    }
}

/// A data-plane message that can be mapped to a WebRTC DataChannel, UCTP
/// `message.send`, SIP MESSAGE, or an application-owned metadata transport.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DataMessage {
    pub label: String,
    pub content_type: String,
    pub bytes: Bytes,
    #[serde(default)]
    pub reliability: DataReliability,
    pub message_id: MessageId,
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
}
