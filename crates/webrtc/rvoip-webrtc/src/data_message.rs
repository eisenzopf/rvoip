//! WebRTC framing and RFC 8832 mapping for transport-neutral `DataMessage`s.
//!
//! A small versioned envelope carries the fields a raw WebRTC message cannot
//! otherwise preserve, including the logical label. Textual
//! MIME types use a WebRTC string message containing a base64url envelope;
//! other MIME types use the binary envelope directly. Unframed peer messages
//! remain accepted as legacy text or binary messages.

use base64::Engine;
use bytes::{Bytes, BytesMut};
use rvoip_core::{DataMessage, DataMessageValidationError, DataReliability, MessageId};
use webrtc::data_channel::DataChannel;

use crate::peer::DataChannelOptions;

const MAGIC: &[u8; 4] = b"RVDM";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 22;
const TEXT_PREFIX: &str = "rvoip-data-v1:";

/// webrtc-rs 0.20 currently documents a 16 KiB inbound message ceiling.
/// Validate the complete framed message so local sends fail deterministically
/// instead of being truncated or dropped in SCTP.
pub const MAX_WEBRTC_DATA_MESSAGE_BYTES: usize = 16 * 1024;
pub const DATA_MESSAGE_SUBPROTOCOL: &str = "rvoip.data.v1";

#[derive(Debug, thiserror::Error)]
pub enum DataMessageWireError {
    #[error(transparent)]
    Validation(#[from] DataMessageValidationError),
    #[error("invalid UTF-8 in data message {field}")]
    InvalidUtf8 { field: &'static str },
    #[error("invalid rvoip WebRTC data-message envelope")]
    InvalidEnvelope,
    #[error("unsupported rvoip WebRTC data-message envelope version {0}")]
    UnsupportedVersion(u8),
    #[error("unsupported rvoip WebRTC data-message reliability code {0}")]
    UnsupportedReliability(u8),
    #[error("framed WebRTC data message is {actual} bytes; maximum is {maximum}")]
    WireMessageTooLarge { actual: usize, maximum: usize },
    #[error("data-message envelope reliability does not match its WebRTC DataChannel")]
    ReliabilityMismatch,
    #[error("data-message envelope label does not match its WebRTC DataChannel")]
    LabelMismatch,
    #[error("data-message content type does not match its WebRTC text/binary frame kind")]
    FrameKindMismatch,
    #[error("invalid WebRTC DataChannel reliability: {0}")]
    InvalidChannelReliability(String),
}

#[derive(Debug, Eq, PartialEq)]
pub enum EncodedDataMessage {
    Text(String),
    Binary(BytesMut),
}

/// Encode a transport-neutral message for a WebRTC DataChannel using
/// [`DATA_MESSAGE_SUBPROTOCOL`]. Browser/native clients can implement the
/// same public framing contract without depending on adapter internals.
pub fn encode_data_message(
    message: &DataMessage,
) -> Result<EncodedDataMessage, DataMessageWireError> {
    validate_data_message(message)?;
    let binary = encode_binary(message)?;
    if textual_content_type(&message.content_type) {
        let encoded = format!(
            "{TEXT_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&binary)
        );
        ensure_wire_size(encoded.len())?;
        Ok(EncodedDataMessage::Text(encoded))
    } else {
        ensure_wire_size(binary.len())?;
        Ok(EncodedDataMessage::Binary(BytesMut::from(
            binary.as_slice(),
        )))
    }
}

/// Decode an inbound WebRTC message. Versioned envelopes are recognized only
/// when `channel_protocol` is [`DATA_MESSAGE_SUBPROTOCOL`]; arbitrary channels
/// always receive collision-safe legacy text/binary fallback.
pub fn decode_data_message(
    channel_label: &str,
    channel_protocol: &str,
    channel_reliability: DataReliability,
    data: &[u8],
    is_string: bool,
) -> Result<DataMessage, DataMessageWireError> {
    ensure_wire_size(data.len())?;

    let message = if channel_protocol == DATA_MESSAGE_SUBPROTOCOL && is_string {
        let text = std::str::from_utf8(data).map_err(|_| DataMessageWireError::InvalidUtf8 {
            field: "text frame",
        })?;
        let encoded = text
            .strip_prefix(TEXT_PREFIX)
            .ok_or(DataMessageWireError::InvalidEnvelope)?;
        let binary = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?;
        let message = decode_binary(&binary)?;
        if !textual_content_type(&message.content_type) {
            return Err(DataMessageWireError::FrameKindMismatch);
        }
        if message.label != channel_label {
            return Err(DataMessageWireError::LabelMismatch);
        }
        if message.reliability != channel_reliability {
            return Err(DataMessageWireError::ReliabilityMismatch);
        }
        message
    } else if channel_protocol == DATA_MESSAGE_SUBPROTOCOL {
        if !data.starts_with(MAGIC) {
            return Err(DataMessageWireError::InvalidEnvelope);
        }
        let message = decode_binary(data)?;
        if textual_content_type(&message.content_type) {
            return Err(DataMessageWireError::FrameKindMismatch);
        }
        if message.label != channel_label {
            return Err(DataMessageWireError::LabelMismatch);
        }
        if message.reliability != channel_reliability {
            return Err(DataMessageWireError::ReliabilityMismatch);
        }
        message
    } else if is_string {
        std::str::from_utf8(data).map_err(|_| DataMessageWireError::InvalidUtf8 {
            field: "text frame",
        })?;
        DataMessage {
            label: channel_label.to_owned(),
            content_type: "text/plain; charset=utf-8".into(),
            bytes: Bytes::copy_from_slice(data),
            reliability: channel_reliability,
            message_id: MessageId::new(),
        }
    } else {
        DataMessage {
            label: channel_label.to_owned(),
            content_type: "application/octet-stream".into(),
            bytes: Bytes::copy_from_slice(data),
            reliability: channel_reliability,
            message_id: MessageId::new(),
        }
    };

    validate_data_message(&message)?;
    Ok(message)
}

pub fn validate_data_message(message: &DataMessage) -> Result<(), DataMessageWireError> {
    message.validate()?;
    Ok(())
}

pub(crate) fn cache_key(message: &DataMessage) -> Result<String, DataMessageWireError> {
    validate_data_message(message)?;
    cache_key_parts(&message.label, &message.reliability)
}

pub(crate) fn cache_key_parts(
    label: &str,
    reliability: &DataReliability,
) -> Result<String, DataMessageWireError> {
    DataMessage::reliable(label, "application/octet-stream", Bytes::new()).validate()?;
    reliability.validate()?;
    // Labels are deliberately not case-folded or trimmed: RTCDataChannel
    // labels are case-sensitive application identifiers. Length-prefixing
    // makes this composite key unambiguous for every valid label.
    Ok(format!(
        "{}:{}:{}",
        label.len(),
        label,
        reliability_key(reliability)
    ))
}

pub fn options_for_reliability(
    reliability: &DataReliability,
) -> Result<DataChannelOptions, DataMessageWireError> {
    reliability.validate()?;
    let mut options = match reliability {
        DataReliability::ReliableOrdered => DataChannelOptions::reliable(),
        DataReliability::ReliableUnordered => DataChannelOptions {
            ordered: false,
            max_retransmits: None,
            max_packet_lifetime_ms: None,
            protocol: None,
            negotiated_id: None,
        },
        DataReliability::MaxRetransmits { ordered, count } => DataChannelOptions {
            ordered: *ordered,
            max_retransmits: Some(*count),
            max_packet_lifetime_ms: None,
            protocol: None,
            negotiated_id: None,
        },
        DataReliability::MaxLifetime {
            ordered,
            milliseconds,
        } => DataChannelOptions {
            ordered: *ordered,
            max_retransmits: None,
            max_packet_lifetime_ms: Some(*milliseconds as u16),
            protocol: None,
            negotiated_id: None,
        },
    };
    options.protocol = Some(DATA_MESSAGE_SUBPROTOCOL.into());
    Ok(options)
}

pub async fn reliability_from_channel(
    channel: &dyn DataChannel,
) -> Result<DataReliability, DataMessageWireError> {
    let ordered = channel
        .ordered()
        .await
        .map_err(|error| DataMessageWireError::InvalidChannelReliability(error.to_string()))?;
    let max_lifetime = channel
        .max_packet_life_time()
        .await
        .map_err(|error| DataMessageWireError::InvalidChannelReliability(error.to_string()))?;
    let max_retransmits = channel
        .max_retransmits()
        .await
        .map_err(|error| DataMessageWireError::InvalidChannelReliability(error.to_string()))?;

    let reliability = match (max_lifetime, max_retransmits) {
        (Some(_), Some(_)) => {
            return Err(DataMessageWireError::InvalidChannelReliability(
                "maxPacketLifeTime and maxRetransmits are both set".into(),
            ));
        }
        (Some(milliseconds), None) => DataReliability::MaxLifetime {
            ordered,
            milliseconds: milliseconds.into(),
        },
        (None, Some(count)) => DataReliability::MaxRetransmits { ordered, count },
        (None, None) if ordered => DataReliability::ReliableOrdered,
        (None, None) => DataReliability::ReliableUnordered,
    };
    reliability.validate()?;
    Ok(reliability)
}

pub fn textual_content_type(content_type: &str) -> bool {
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

fn encode_binary(message: &DataMessage) -> Result<Vec<u8>, DataMessageWireError> {
    let (kind, ordered, value) = encode_reliability(&message.reliability);
    let message_id = message.message_id.as_str().as_bytes();
    let content_type = message.content_type.as_bytes();
    let mut frame = Vec::with_capacity(
        HEADER_LEN
            + message.label.len()
            + message_id.len()
            + content_type.len()
            + message.bytes.len(),
    );
    frame.extend_from_slice(MAGIC);
    frame.push(VERSION);
    frame.push(kind);
    frame.push(u8::from(ordered));
    frame.push(0);
    frame.extend_from_slice(&value.to_be_bytes());
    frame.extend_from_slice(&(message.label.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(message_id.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(content_type.len() as u16).to_be_bytes());
    frame.extend_from_slice(&(message.bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(message.label.as_bytes());
    frame.extend_from_slice(message_id);
    frame.extend_from_slice(content_type);
    frame.extend_from_slice(&message.bytes);
    Ok(frame)
}

fn decode_binary(frame: &[u8]) -> Result<DataMessage, DataMessageWireError> {
    if frame.len() < HEADER_LEN || &frame[..4] != MAGIC {
        return Err(DataMessageWireError::InvalidEnvelope);
    }
    if frame[4] != VERSION {
        return Err(DataMessageWireError::UnsupportedVersion(frame[4]));
    }
    if frame[6] > 1 || frame[7] != 0 {
        return Err(DataMessageWireError::InvalidEnvelope);
    }
    let value = u32::from_be_bytes(
        frame[8..12]
            .try_into()
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?,
    );
    let label_len = u16::from_be_bytes(
        frame[12..14]
            .try_into()
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?,
    ) as usize;
    let message_id_len = u16::from_be_bytes(
        frame[14..16]
            .try_into()
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?,
    ) as usize;
    let content_type_len = u16::from_be_bytes(
        frame[16..18]
            .try_into()
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?,
    ) as usize;
    let body_len = u32::from_be_bytes(
        frame[18..22]
            .try_into()
            .map_err(|_| DataMessageWireError::InvalidEnvelope)?,
    ) as usize;
    let expected = HEADER_LEN
        .checked_add(label_len)
        .and_then(|length| length.checked_add(message_id_len))
        .and_then(|length| length.checked_add(content_type_len))
        .and_then(|length| length.checked_add(body_len))
        .ok_or(DataMessageWireError::InvalidEnvelope)?;
    if expected != frame.len() {
        return Err(DataMessageWireError::InvalidEnvelope);
    }

    let label_end = HEADER_LEN + label_len;
    let id_end = label_end + message_id_len;
    let content_type_end = id_end + content_type_len;
    let label = std::str::from_utf8(&frame[HEADER_LEN..label_end])
        .map_err(|_| DataMessageWireError::InvalidUtf8 { field: "label" })?;
    let message_id = std::str::from_utf8(&frame[label_end..id_end]).map_err(|_| {
        DataMessageWireError::InvalidUtf8 {
            field: "message id",
        }
    })?;
    let content_type = std::str::from_utf8(&frame[id_end..content_type_end]).map_err(|_| {
        DataMessageWireError::InvalidUtf8 {
            field: "content type",
        }
    })?;
    let reliability = decode_reliability(frame[5], frame[6] != 0, value)?;
    let message = DataMessage {
        label: label.to_owned(),
        content_type: content_type.to_owned(),
        bytes: Bytes::copy_from_slice(&frame[content_type_end..]),
        reliability,
        message_id: MessageId::from_string(message_id),
    };
    validate_data_message(&message)?;
    Ok(message)
}

fn encode_reliability(reliability: &DataReliability) -> (u8, bool, u32) {
    match reliability {
        DataReliability::ReliableOrdered => (0, true, 0),
        DataReliability::ReliableUnordered => (0, false, 0),
        DataReliability::MaxRetransmits { ordered, count } => (1, *ordered, (*count).into()),
        DataReliability::MaxLifetime {
            ordered,
            milliseconds,
        } => (2, *ordered, *milliseconds),
    }
}

fn decode_reliability(
    kind: u8,
    ordered: bool,
    value: u32,
) -> Result<DataReliability, DataMessageWireError> {
    let reliability = match kind {
        0 if value == 0 && ordered => DataReliability::ReliableOrdered,
        0 if value == 0 => DataReliability::ReliableUnordered,
        0 => return Err(DataMessageWireError::InvalidEnvelope),
        1 if value <= u16::MAX as u32 => DataReliability::MaxRetransmits {
            ordered,
            count: value as u16,
        },
        1 => return Err(DataMessageWireError::InvalidEnvelope),
        2 => DataReliability::MaxLifetime {
            ordered,
            milliseconds: value,
        },
        other => return Err(DataMessageWireError::UnsupportedReliability(other)),
    };
    reliability.validate()?;
    Ok(reliability)
}

fn reliability_key(reliability: &DataReliability) -> String {
    match reliability {
        DataReliability::ReliableOrdered => "ro".into(),
        DataReliability::ReliableUnordered => "ru".into(),
        DataReliability::MaxRetransmits { ordered, count } => {
            format!("mr:{}:{count}", u8::from(*ordered))
        }
        DataReliability::MaxLifetime {
            ordered,
            milliseconds,
        } => format!("ml:{}:{milliseconds}", u8::from(*ordered)),
    }
}

fn ensure_wire_size(actual: usize) -> Result<(), DataMessageWireError> {
    if actual > MAX_WEBRTC_DATA_MESSAGE_BYTES {
        return Err(DataMessageWireError::WireMessageTooLarge {
            actual,
            maximum: MAX_WEBRTC_DATA_MESSAGE_BYTES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        content_type: &str,
        body: &'static [u8],
        reliability: DataReliability,
    ) -> DataMessage {
        DataMessage {
            label: "bridgefu.context.v1".into(),
            content_type: content_type.into(),
            bytes: Bytes::from_static(body),
            reliability,
            message_id: MessageId::from_string("msg_preserved"),
        }
    }

    #[test]
    fn text_envelope_round_trips_all_fields() {
        let input = message(
            "application/json",
            br#"{"correlation_id":"abc"}"#,
            DataReliability::ReliableOrdered,
        );
        let EncodedDataMessage::Text(frame) = encode_data_message(&input).expect("encode") else {
            panic!("JSON must use a WebRTC string frame");
        };
        let output = decode_data_message(
            &input.label,
            DATA_MESSAGE_SUBPROTOCOL,
            input.reliability.clone(),
            frame.as_bytes(),
            true,
        )
        .expect("decode");
        assert_eq!(output, input);
    }

    #[test]
    fn binary_envelope_round_trips_all_fields() {
        let input = message(
            "application/octet-stream",
            b"\0\xff\x01",
            DataReliability::MaxRetransmits {
                ordered: false,
                count: 3,
            },
        );
        let EncodedDataMessage::Binary(frame) = encode_data_message(&input).expect("encode") else {
            panic!("binary MIME type must use a WebRTC binary frame");
        };
        let output = decode_data_message(
            &input.label,
            DATA_MESSAGE_SUBPROTOCOL,
            input.reliability.clone(),
            &frame,
            false,
        )
        .expect("decode");
        assert_eq!(output, input);
    }

    #[test]
    fn legacy_frames_receive_defaults_without_losing_bytes() {
        let text = decode_data_message(
            "rvoip-chat",
            "",
            DataReliability::ReliableOrdered,
            b"hello",
            true,
        )
        .expect("legacy text");
        assert_eq!(text.content_type, "text/plain; charset=utf-8");
        assert_eq!(text.bytes, Bytes::from_static(b"hello"));

        let binary = decode_data_message(
            "telemetry",
            "chat",
            DataReliability::ReliableUnordered,
            b"\0\xff",
            false,
        )
        .expect("legacy binary");
        assert_eq!(binary.content_type, "application/octet-stream");
        assert_eq!(binary.bytes, Bytes::from_static(b"\0\xff"));
    }

    #[test]
    fn reliability_maps_to_rfc_8832_options() {
        let cases = [
            (DataReliability::ReliableOrdered, (true, None, None)),
            (DataReliability::ReliableUnordered, (false, None, None)),
            (
                DataReliability::MaxRetransmits {
                    ordered: false,
                    count: 7,
                },
                (false, Some(7), None),
            ),
            (
                DataReliability::MaxLifetime {
                    ordered: true,
                    milliseconds: 250,
                },
                (true, None, Some(250)),
            ),
        ];

        for (reliability, expected) in cases {
            let options = options_for_reliability(&reliability).expect("options");
            assert_eq!(
                (
                    options.ordered,
                    options.max_retransmits,
                    options.max_packet_lifetime_ms,
                ),
                expected
            );
            assert_eq!(options.protocol.as_deref(), Some(DATA_MESSAGE_SUBPROTOCOL));
        }
    }

    #[test]
    fn framed_reliability_must_match_channel() {
        let input = message("text/plain", b"hello", DataReliability::ReliableOrdered);
        let EncodedDataMessage::Text(frame) = encode_data_message(&input).expect("encode") else {
            panic!("expected text");
        };
        assert!(matches!(
            decode_data_message(
                &input.label,
                DATA_MESSAGE_SUBPROTOCOL,
                DataReliability::ReliableUnordered,
                frame.as_bytes(),
                true
            ),
            Err(DataMessageWireError::ReliabilityMismatch)
        ));
    }

    #[test]
    fn framed_label_must_match_physical_channel() {
        let input = message("text/plain", b"hello", DataReliability::ReliableOrdered);
        let EncodedDataMessage::Text(frame) = encode_data_message(&input).expect("encode") else {
            panic!("expected text");
        };
        assert!(matches!(
            decode_data_message(
                "different-label",
                DATA_MESSAGE_SUBPROTOCOL,
                DataReliability::ReliableOrdered,
                frame.as_bytes(),
                true,
            ),
            Err(DataMessageWireError::LabelMismatch)
        ));
    }

    #[test]
    fn framed_content_type_must_match_webrtc_frame_kind() {
        let text = message("text/plain", b"hello", DataReliability::ReliableOrdered);
        let binary_frame = encode_binary(&text).expect("binary envelope");
        assert!(matches!(
            decode_data_message(
                &text.label,
                DATA_MESSAGE_SUBPROTOCOL,
                text.reliability.clone(),
                &binary_frame,
                false,
            ),
            Err(DataMessageWireError::FrameKindMismatch)
        ));

        let binary = message(
            "application/octet-stream",
            b"hello",
            DataReliability::ReliableOrdered,
        );
        let encoded = format!(
            "{TEXT_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(encode_binary(&binary).expect("binary envelope"))
        );
        assert!(matches!(
            decode_data_message(
                &binary.label,
                DATA_MESSAGE_SUBPROTOCOL,
                binary.reliability,
                encoded.as_bytes(),
                true,
            ),
            Err(DataMessageWireError::FrameKindMismatch)
        ));
    }

    #[test]
    fn rejects_invalid_ids_and_oversized_text_wire_frames() {
        let mut invalid = message("text/plain", b"hello", DataReliability::ReliableOrdered);
        invalid.message_id = MessageId::from_string("");
        assert!(matches!(
            encode_data_message(&invalid),
            Err(DataMessageWireError::Validation(
                DataMessageValidationError::EmptyMessageId
            ))
        ));

        let oversized = DataMessage {
            label: "large".into(),
            content_type: "text/plain".into(),
            bytes: Bytes::from(vec![b'x'; 13_000]),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::new(),
        };
        assert!(matches!(
            encode_data_message(&oversized),
            Err(DataMessageWireError::WireMessageTooLarge { .. })
        ));
    }

    #[test]
    fn enforces_webrtc_16k_wire_boundary_exactly() {
        let content_type = "application/octet-stream";
        let message_id = "m";
        let body_at_limit = MAX_WEBRTC_DATA_MESSAGE_BYTES
            - HEADER_LEN
            - "binary".len()
            - message_id.len()
            - content_type.len();
        let at_limit = DataMessage {
            label: "binary".into(),
            content_type: content_type.into(),
            bytes: Bytes::from(vec![0; body_at_limit]),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::from_string(message_id),
        };
        let EncodedDataMessage::Binary(frame) =
            encode_data_message(&at_limit).expect("16 KiB wire frame")
        else {
            panic!("expected binary frame");
        };
        assert_eq!(frame.len(), MAX_WEBRTC_DATA_MESSAGE_BYTES);

        let over_limit = DataMessage {
            bytes: Bytes::from(vec![0; body_at_limit + 1]),
            ..at_limit
        };
        assert!(matches!(
            encode_data_message(&over_limit),
            Err(DataMessageWireError::WireMessageTooLarge {
                actual: 16_385,
                maximum: 16_384,
            })
        ));
    }

    #[test]
    fn protocol_gates_framing_and_prevents_magic_collisions() {
        let colliding_text = format!("{TEXT_PREFIX}not-base64!");
        let text = decode_data_message(
            "ordinary-text",
            "chat",
            DataReliability::ReliableOrdered,
            colliding_text.as_bytes(),
            true,
        )
        .expect("ordinary text collision is raw");
        assert_eq!(text.bytes.as_ref(), colliding_text.as_bytes());

        let colliding_binary = b"RVDMordinary binary";
        let binary = decode_data_message(
            "ordinary-binary",
            "",
            DataReliability::ReliableOrdered,
            colliding_binary,
            false,
        )
        .expect("ordinary binary collision is raw");
        assert_eq!(binary.bytes.as_ref(), colliding_binary);

        assert!(matches!(
            decode_data_message(
                "framed",
                DATA_MESSAGE_SUBPROTOCOL,
                DataReliability::ReliableOrdered,
                b"unframed",
                true,
            ),
            Err(DataMessageWireError::InvalidEnvelope)
        ));
    }

    #[test]
    fn enforces_text_16k_encoded_boundary_exactly() {
        let label = "t";
        let content_type = "text/plain";
        let message_id = "m";
        let encoded_capacity = MAX_WEBRTC_DATA_MESSAGE_BYTES - TEXT_PREFIX.len();
        assert_eq!(encoded_capacity % 4, 2);
        let binary_at_limit = (encoded_capacity / 4) * 3 + 1;
        let body_at_limit =
            binary_at_limit - HEADER_LEN - label.len() - message_id.len() - content_type.len();
        let at_limit = DataMessage {
            label: label.into(),
            content_type: content_type.into(),
            bytes: Bytes::from(vec![b'x'; body_at_limit]),
            reliability: DataReliability::ReliableOrdered,
            message_id: MessageId::from_string(message_id),
        };
        let EncodedDataMessage::Text(frame) =
            encode_data_message(&at_limit).expect("16 KiB text wire frame")
        else {
            panic!("expected text frame");
        };
        assert_eq!(frame.len(), MAX_WEBRTC_DATA_MESSAGE_BYTES);

        let over_limit = DataMessage {
            bytes: Bytes::from(vec![b'x'; body_at_limit + 1]),
            ..at_limit
        };
        assert!(matches!(
            encode_data_message(&over_limit),
            Err(DataMessageWireError::WireMessageTooLarge { .. })
        ));
    }
}
