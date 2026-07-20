//! SIP MESSAGE mapping for transport-neutral application data.
//!
//! The headers in this module are an internal interoperability contract. They
//! carry only the fields that SIP MESSAGE does not otherwise represent. They
//! never carry tenant, call, leg, or connection ownership identifiers.

use bytes::Bytes;
use dashmap::DashMap;
use rvoip_core::{
    DataMessage, DataMessageValidationError, DataReliability, MessageId, MAX_CONTENT_TYPE_BYTES,
    MAX_DATA_LABEL_BYTES, MAX_DATA_MESSAGE_BYTES, MAX_DATA_MESSAGE_ID_BYTES,
};
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
use rvoip_sip_core::{Method, Request};
use rvoip_sip_dialog::transaction::dialog::{
    request_builder_from_dialog_template, DialogRequestTemplate,
};
use std::str::FromStr;
use std::sync::Arc;
use thiserror::Error;

pub(crate) const DATA_LABEL_HEADER: &str = "X-Bridgefu-Data-Label";
pub(crate) const CONTENT_TYPE_HEADER: &str = "X-Bridgefu-Data-Content-Type";
pub(crate) const MESSAGE_ID_HEADER: &str = "X-Bridgefu-Message-Id";
pub(crate) const RELIABILITY_HEADER: &str = "X-Bridgefu-Data-Reliability";
const RELIABLE_ORDERED: &str = "reliable-ordered";
const DEFAULT_SIP_MESSAGE_LABEL: &str = "sip.message";
const BRIDGEFU_PREFIX: &str = "x-bridgefu-";

/// A validated SIP MESSAGE payload ready for exact-dialog dispatch.
#[derive(Clone, PartialEq)]
pub(crate) struct SipDataMessage {
    pub(crate) content_type: String,
    pub(crate) bytes: Bytes,
    pub(crate) extra_headers: Vec<TypedHeader>,
}

impl std::fmt::Debug for SipDataMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SipDataMessage")
            .field("content_type_present", &!self.content_type.is_empty())
            .field("content_type_bytes", &self.content_type.len())
            .field("body_bytes", &self.bytes.len())
            .field("internal_header_count", &self.extra_headers.len())
            .finish()
    }
}

/// One FIFO serialization owner per exact dialog. The owner is retained by
/// `DialogAdapter` and removed only by exact dialog cleanup.
#[derive(Default)]
pub(crate) struct SipDataMessageDispatchLanes {
    lanes: DashMap<rvoip_sip_dialog::DialogId, Arc<tokio::sync::Mutex<()>>>,
}

impl SipDataMessageDispatchLanes {
    pub(crate) fn lane(
        &self,
        dialog_id: &rvoip_sip_dialog::DialogId,
    ) -> Arc<tokio::sync::Mutex<()>> {
        self.lanes
            .entry(dialog_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub(crate) fn remove_exact(
        &self,
        dialog_id: &rvoip_sip_dialog::DialogId,
        lane: &Arc<tokio::sync::Mutex<()>>,
    ) {
        self.lanes
            .remove_if(dialog_id, |_, current| Arc::ptr_eq(current, lane));
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lanes.len()
    }
}

#[derive(Debug, Error)]
pub(crate) enum SipDataMessageError {
    #[error("SIP data message uses an unsupported reliability policy")]
    UnsupportedReliability,
    #[error("SIP data message is not a MESSAGE request")]
    WrongMethod,
    #[error("SIP data message has no Content-Type")]
    MissingContentType,
    #[error("SIP data message has duplicate Content-Type headers")]
    DuplicateContentType,
    #[error("SIP data message native and internal Content-Type values disagree")]
    ContentTypeMismatch,
    #[error("SIP data message has a malformed internal header")]
    MalformedInternalHeader,
    #[error("SIP data message has a duplicate internal header")]
    DuplicateInternalHeader,
    #[error("SIP data message contains a forbidden Bridgefu header")]
    ForbiddenBridgefuHeader,
    #[error("SIP data message request construction failed")]
    RequestBuild,
    #[error("SIP data message validation failed")]
    Validation(#[from] DataMessageValidationError),
}

impl SipDataMessageError {
    pub(crate) fn is_unsupported(&self) -> bool {
        matches!(self, Self::UnsupportedReliability)
    }
}

/// Convert an application message into the byte-preserving SIP representation.
pub(crate) fn to_sip_data_message(
    message: &DataMessage,
) -> Result<SipDataMessage, SipDataMessageError> {
    message.validate()?;
    if message.reliability != DataReliability::ReliableOrdered {
        return Err(SipDataMessageError::UnsupportedReliability);
    }

    Ok(SipDataMessage {
        content_type: message.content_type.clone(),
        bytes: message.bytes.clone(),
        extra_headers: vec![
            internal_header(DATA_LABEL_HEADER, &message.label)?,
            internal_header(CONTENT_TYPE_HEADER, &message.content_type)?,
            internal_header(MESSAGE_ID_HEADER, message.message_id.as_str())?,
            internal_header(RELIABILITY_HEADER, RELIABLE_ORDERED)?,
        ],
    })
}

/// Build an in-dialog request without converting the application bytes
/// through UTF-8. The placeholder exists only to let the established dialog
/// builder stamp the exact Content-Length; it is replaced before this request
/// can reach a transport.
pub(crate) fn build_sip_data_request(
    template: &DialogRequestTemplate,
    message: SipDataMessage,
) -> Result<Request, SipDataMessageError> {
    let placeholder = "x".repeat(message.bytes.len());
    let mut request = request_builder_from_dialog_template(
        template,
        Method::Message,
        Some(placeholder),
        Some(message.content_type),
        Some(message.extra_headers),
    )
    .map_err(|_| SipDataMessageError::RequestBuild)?;
    request.body = message.bytes;
    Ok(request)
}

/// Convert a parsed in-dialog SIP MESSAGE into a transport-neutral message.
///
/// Standard routing headers are deliberately ignored. Only the fixed
/// `X-Bridgefu-*` fields above are accepted; every other header in that
/// namespace, including ownership identifier overrides, fails closed.
pub(crate) fn from_sip_request(request: &Request) -> Result<DataMessage, SipDataMessageError> {
    if request.method != Method::Message {
        return Err(SipDataMessageError::WrongMethod);
    }
    if request.body.len() > MAX_DATA_MESSAGE_BYTES {
        return Err(DataMessageValidationError::BodyTooLarge {
            actual: request.body.len(),
            maximum: MAX_DATA_MESSAGE_BYTES,
        }
        .into());
    }

    let mut content_type = None;
    let mut original_content_type = None;
    let mut label = None;
    let mut message_id = None;
    let mut reliability = None;

    for header in &request.headers {
        match header {
            TypedHeader::ContentType(value) => {
                set_once(&mut content_type, value.to_string(), true)?;
            }
            TypedHeader::Other(HeaderName::Other(name), value)
                if name.to_ascii_lowercase().starts_with(BRIDGEFU_PREFIX) =>
            {
                let value = internal_header_value(value)?;
                if name.eq_ignore_ascii_case(DATA_LABEL_HEADER) {
                    set_once(&mut label, value, false)?;
                } else if name.eq_ignore_ascii_case(CONTENT_TYPE_HEADER) {
                    set_once(&mut original_content_type, value, false)?;
                } else if name.eq_ignore_ascii_case(MESSAGE_ID_HEADER) {
                    set_once(&mut message_id, value, false)?;
                } else if name.eq_ignore_ascii_case(RELIABILITY_HEADER) {
                    set_once(&mut reliability, value, false)?;
                } else {
                    return Err(SipDataMessageError::ForbiddenBridgefuHeader);
                }
            }
            _ => {}
        }
    }

    let native_content_type = content_type.ok_or(SipDataMessageError::MissingContentType)?;
    let content_type = match original_content_type {
        Some(original) => {
            if !equivalent_content_type(&native_content_type, &original) {
                return Err(SipDataMessageError::ContentTypeMismatch);
            }
            original
        }
        None => native_content_type,
    };
    if reliability
        .as_deref()
        .is_some_and(|value| value != RELIABLE_ORDERED)
    {
        return Err(SipDataMessageError::UnsupportedReliability);
    }

    DataMessage::try_new(
        label.unwrap_or_else(|| DEFAULT_SIP_MESSAGE_LABEL.to_string()),
        content_type,
        request.body.clone(),
        DataReliability::ReliableOrdered,
        message_id
            .map(MessageId::from_string)
            .unwrap_or_else(MessageId::new),
    )
    .map_err(Into::into)
}

fn set_once(
    slot: &mut Option<String>,
    value: String,
    content_type: bool,
) -> Result<(), SipDataMessageError> {
    if slot.replace(value).is_some() {
        return Err(if content_type {
            SipDataMessageError::DuplicateContentType
        } else {
            SipDataMessageError::DuplicateInternalHeader
        });
    }
    Ok(())
}

fn internal_header(name: &str, value: &str) -> Result<TypedHeader, SipDataMessageError> {
    validate_internal_value(value)?;
    let name =
        HeaderName::from_str(name).map_err(|_| SipDataMessageError::MalformedInternalHeader)?;
    Ok(TypedHeader::Other(
        name,
        HeaderValue::Raw(value.as_bytes().to_vec()),
    ))
}

fn internal_header_value(value: &HeaderValue) -> Result<String, SipDataMessageError> {
    let HeaderValue::Raw(bytes) = value else {
        return Err(SipDataMessageError::MalformedInternalHeader);
    };
    let value = std::str::from_utf8(bytes)
        .map_err(|_| SipDataMessageError::MalformedInternalHeader)?
        .to_string();
    validate_internal_value(&value)?;
    Ok(value)
}

fn validate_internal_value(value: &str) -> Result<(), SipDataMessageError> {
    if value.is_empty()
        || value.len()
            > MAX_DATA_LABEL_BYTES
                .max(MAX_DATA_MESSAGE_ID_BYTES)
                .max(MAX_CONTENT_TYPE_BYTES)
        || value.chars().any(char::is_control)
    {
        return Err(SipDataMessageError::MalformedInternalHeader);
    }
    Ok(())
}

fn equivalent_content_type(left: &str, right: &str) -> bool {
    let Ok(left) = rvoip_sip_core::types::ContentType::from_str(left) else {
        return false;
    };
    let Ok(right) = rvoip_sip_core::types::ContentType::from_str(right) else {
        return false;
    };
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::{ContentType, Uri};

    fn request_from_wire(message: &DataMessage) -> Request {
        let wire = to_sip_data_message(message).expect("wire mapping");
        let request = build_sip_data_request(
            &DialogRequestTemplate {
                call_id: "data-message@example.test".to_string(),
                from_uri: "sip:alice@example.test".to_string(),
                from_tag: "alice-tag".to_string(),
                to_uri: "sip:bob@example.test".to_string(),
                to_tag: "bob-tag".to_string(),
                request_uri: "sip:bob@127.0.0.1:5060".to_string(),
                cseq: 2,
                local_address: "127.0.0.1:5070".parse().expect("local address"),
                route_set: Vec::new(),
                contact: None,
            },
            wire,
        )
        .expect("request mapping");
        let serialized = rvoip_sip_core::Message::Request(request).to_bytes();
        match rvoip_sip_core::parse_message(&serialized).expect("wire parse") {
            rvoip_sip_core::Message::Request(request) => request,
            rvoip_sip_core::Message::Response(_) => panic!("expected request"),
        }
    }

    #[test]
    fn text_message_round_trips() {
        let input = DataMessage::try_new(
            "bridgefu.context.v1",
            "application/json; charset=utf-8",
            Bytes::from_static(br#"{"kind":"screen-pop"}"#),
            DataReliability::ReliableOrdered,
            MessageId::from_string("msg-context-1"),
        )
        .expect("input");
        let output = from_sip_request(&request_from_wire(&input)).expect("decode");
        assert_eq!(output, input);
    }

    #[test]
    fn arbitrary_binary_message_round_trips_without_utf8_conversion() {
        let input = DataMessage::try_new(
            "binary",
            "application/octet-stream",
            Bytes::from_static(&[0, 0xff, 0x80, b'\r', b'\n', 1]),
            DataReliability::ReliableOrdered,
            MessageId::from_string("msg-binary-1"),
        )
        .expect("input");
        let output = from_sip_request(&request_from_wire(&input)).expect("decode");
        assert_eq!(output, input);
    }

    #[test]
    fn normal_sip_message_gets_safe_defaults() {
        let mut request = Request::new(
            Method::Message,
            Uri::from_str("sip:bob@example.test").expect("URI"),
        )
        .with_body(Bytes::from_static(b"hello"));
        request
            .headers
            .push(TypedHeader::ContentType(ContentType::text_plain()));

        let output = from_sip_request(&request).expect("decode");
        assert_eq!(output.label, DEFAULT_SIP_MESSAGE_LABEL);
        assert_eq!(output.content_type, "text/plain");
        assert_eq!(output.bytes, Bytes::from_static(b"hello"));
        assert!(!output.message_id.as_str().is_empty());
    }

    #[test]
    fn rejects_injection_oversize_and_identity_overrides() {
        let mut injected = DataMessage::reliable("bad\r\nVia: injected", "text/plain", "x");
        assert!(matches!(
            to_sip_data_message(&injected),
            Err(SipDataMessageError::Validation(
                DataMessageValidationError::LabelContainsControl
            ))
        ));

        injected.label = "x".repeat(MAX_DATA_LABEL_BYTES + 1);
        assert!(matches!(
            to_sip_data_message(&injected),
            Err(SipDataMessageError::Validation(
                DataMessageValidationError::LabelTooLong { .. }
            ))
        ));

        let mut request =
            request_from_wire(&DataMessage::reliable("context", "application/json", "{}"));
        request.headers.push(TypedHeader::Other(
            HeaderName::Other("X-Bridgefu-Tenant-Id".into()),
            HeaderValue::Raw(b"other-tenant".to_vec()),
        ));
        assert!(matches!(
            from_sip_request(&request),
            Err(SipDataMessageError::ForbiddenBridgefuHeader)
        ));
    }

    #[test]
    fn rejects_duplicate_malformed_and_unsupported_internal_fields() {
        let mut duplicate =
            request_from_wire(&DataMessage::reliable("context", "application/json", "{}"));
        duplicate.headers.push(TypedHeader::Other(
            HeaderName::Other(DATA_LABEL_HEADER.into()),
            HeaderValue::Raw(b"second".to_vec()),
        ));
        assert!(matches!(
            from_sip_request(&duplicate),
            Err(SipDataMessageError::DuplicateInternalHeader)
        ));

        let mut malformed =
            request_from_wire(&DataMessage::reliable("context", "application/json", "{}"));
        malformed.headers.retain(|header| {
            !matches!(header, TypedHeader::Other(name, _) if name.as_str().eq_ignore_ascii_case(DATA_LABEL_HEADER))
        });
        malformed.headers.push(TypedHeader::Other(
            HeaderName::Other(DATA_LABEL_HEADER.into()),
            HeaderValue::Raw(b"bad\r\nVia: injected".to_vec()),
        ));
        assert!(matches!(
            from_sip_request(&malformed),
            Err(SipDataMessageError::MalformedInternalHeader)
        ));

        let mut unreliable = DataMessage::reliable("context", "application/json", "{}");
        unreliable.reliability = DataReliability::MaxRetransmits {
            ordered: true,
            count: 1,
        };
        assert!(matches!(
            to_sip_data_message(&unreliable),
            Err(SipDataMessageError::UnsupportedReliability)
        ));
    }

    #[test]
    fn rejects_native_and_exact_content_type_mismatch() {
        let mut request =
            request_from_wire(&DataMessage::reliable("context", "application/json", "{}"));
        for header in &mut request.headers {
            if let TypedHeader::Other(name, HeaderValue::Raw(value)) = header {
                if name.as_str().eq_ignore_ascii_case(CONTENT_TYPE_HEADER) {
                    *value = b"text/plain".to_vec();
                }
            }
        }

        assert!(matches!(
            from_sip_request(&request),
            Err(SipDataMessageError::ContentTypeMismatch)
        ));
    }

    #[test]
    fn rejects_oversized_body_before_allocation_or_delivery() {
        let mut request = Request::new(
            Method::Message,
            Uri::from_str("sip:bob@example.test").expect("URI"),
        )
        .with_body(Bytes::from(vec![0; MAX_DATA_MESSAGE_BYTES + 1]));
        request
            .headers
            .push(TypedHeader::ContentType(ContentType::text_plain()));
        assert!(matches!(
            from_sip_request(&request),
            Err(SipDataMessageError::Validation(
                DataMessageValidationError::BodyTooLarge { .. }
            ))
        ));
    }

    #[tokio::test]
    async fn exact_dialog_lane_serializes_and_cleans_up() {
        let lanes = Arc::new(SipDataMessageDispatchLanes::default());
        let dialog_id = rvoip_sip_dialog::DialogId::new();
        let lane = lanes.lane(&dialog_id);
        let first = lane.clone().lock_owned().await;
        let (acquired_tx, mut acquired_rx) = tokio::sync::mpsc::channel(1);
        let waiter_lane = lanes.lane(&dialog_id);
        let waiter = tokio::spawn(async move {
            let _guard = waiter_lane.lock_owned().await;
            acquired_tx.send(()).await.expect("acquisition signal");
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), acquired_rx.recv())
                .await
                .is_err(),
            "the second dispatch must not pass the first"
        );
        drop(first);
        tokio::time::timeout(std::time::Duration::from_secs(1), acquired_rx.recv())
            .await
            .expect("second acquisition deadline")
            .expect("second acquisition");
        waiter.await.expect("waiter");

        lanes.remove_exact(&dialog_id, &lane);
        assert_eq!(lanes.len(), 0);
    }

    #[tokio::test]
    async fn late_sender_after_cleanup_removes_only_its_replacement_lane() {
        let lanes = Arc::new(SipDataMessageDispatchLanes::default());
        let dialog_id = rvoip_sip_dialog::DialogId::new();
        let original = lanes.lane(&dialog_id);
        let original_guard = original.clone().lock_owned().await;

        // Exact cleanup removes its lane while still holding the owner.
        lanes.remove_exact(&dialog_id, &original);
        assert_eq!(lanes.len(), 0);

        let (created_tx, created_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let late_lanes = Arc::clone(&lanes);
        let late_dialog = dialog_id.clone();
        let late = tokio::spawn(async move {
            let replacement = late_lanes.lane(&late_dialog);
            assert!(!Arc::ptr_eq(&replacement, &original));
            let _guard = replacement.clone().lock_owned().await;
            created_tx.send(()).expect("created signal");
            release_rx.await.expect("release signal");
            // This is the exact action taken after the late sender discovers
            // that cleanup already removed the dialog resource.
            late_lanes.remove_exact(&late_dialog, &replacement);
        });
        created_rx
            .await
            .expect("late sender entered replacement lane");
        assert_eq!(lanes.len(), 1);
        release_tx.send(()).expect("release late sender");
        late.await.expect("late sender");
        assert_eq!(lanes.len(), 0);
        drop(original_guard);
    }

    #[test]
    fn sip_data_message_debug_is_payload_free() {
        const SECRET: &[u8] = b"message-body-secret";
        let wire = to_sip_data_message(
            &DataMessage::try_new(
                "secret-label",
                "application/secret-type",
                Bytes::from_static(SECRET),
                DataReliability::ReliableOrdered,
                MessageId::from_string("secret-message-id"),
            )
            .expect("message"),
        )
        .expect("wire");
        let debug = format!("{wire:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("application/secret-type"));
        assert!(debug.contains("body_bytes"));
    }
}
