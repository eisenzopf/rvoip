// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc.
// SPDX-License-Identifier: MIT OR Apache-2.0

//! REQUEST_ERROR message (draft-ietf-moq-transport-19 §9.8).
//!
//! Sent in response to any request (SUBSCRIBE, FETCH, PUBLISH,
//! SUBSCRIBE_NAMESPACE, PUBLISH_NAMESPACE, TRACK_STATUS, REQUEST_UPDATE).
//! Replaces the per-request error messages from earlier drafts.

use crate::coding::{
    Decode, DecodeError, Encode, EncodeError, ReasonPhrase, SessionUri, TrackName,
    TrackNamespacePrefix,
};

/// Draft-19 REQUEST_ERROR codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum RequestErrorCode {
    InternalError = 0x0,
    Unauthorized = 0x1,
    Timeout = 0x2,
    NotSupported = 0x3,
    MalformedAuthToken = 0x4,
    ExpiredAuthToken = 0x5,
    GoingAway = 0x6,
    ExcessiveLoad = 0x9,
    DoesNotExist = 0x10,
    InvalidRange = 0x11,
    MalformedTrack = 0x12,
    Uninterested = 0x20,
    PrefixOverlap = 0x30,
    NamespaceTooLarge = 0x31,
    InvalidJoiningRequestId = 0x32,
    UnsupportedExtension = 0x33,
    Redirect = 0x34,
    ConflictingFilters = 0x35,
    InvalidFilter = 0x36,
}

/// Optional redirect information carried only by a REDIRECT REQUEST_ERROR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Redirect {
    /// URI to connect to, or an empty URI to reuse the current session URI.
    pub connect_uri: SessionUri,

    /// Namespace for the retried request. An empty prefix preserves the
    /// original namespace when `track_name` is also empty.
    pub track_namespace: TrackNamespacePrefix,

    /// Track name for the retried request. Namespace-scoped request handlers
    /// must reject non-empty names as a protocol violation.
    pub track_name: TrackName,
}

impl Decode for Redirect {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        Ok(Self {
            connect_uri: SessionUri::decode(r)?,
            track_namespace: TrackNamespacePrefix::decode(r)?,
            track_name: TrackName::decode(r)?,
        })
    }
}

impl Encode for Redirect {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.connect_uri.encode(w)?;
        self.track_namespace.encode(w)?;
        self.track_name.encode(w)?;
        Ok(())
    }
}

impl From<RequestErrorCode> for u64 {
    fn from(c: RequestErrorCode) -> u64 {
        c as u64
    }
}

/// Sent to reject any request.
///
/// `retry_interval`: minimum time (ms) before the request SHOULD be sent
/// again, plus one.  A value of 0 means the request MUST NOT be retried;
/// a value of 1 means it can be retried immediately.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestError {
    /// Local Request ID association; omitted from request-stream wire bodies.
    pub id: u64,

    /// Error code identifying the reason for rejection.
    pub error_code: u64,

    /// Minimum retry delay in milliseconds plus one, or 0 for no retry.
    pub retry_interval: u64,

    /// Human-readable reason phrase (UTF-8, max 1024 bytes).
    pub reason: ReasonPhrase,

    /// Present if and only if `error_code` is REDIRECT.
    pub redirect: Option<Redirect>,
}

impl RequestError {
    /// Convenience constructor from a [`RequestErrorCode`].
    pub fn new(id: u64, code: RequestErrorCode, retry_interval: u64, reason: &str) -> Self {
        Self {
            id,
            error_code: code as u64,
            retry_interval,
            reason: ReasonPhrase(reason.to_string()),
            redirect: None,
        }
    }

    /// Construct a REDIRECT response.
    pub fn redirected(id: u64, retry_interval: u64, reason: &str, redirect: Redirect) -> Self {
        Self {
            id,
            error_code: RequestErrorCode::Redirect as u64,
            retry_interval,
            reason: ReasonPhrase(reason.to_string()),
            redirect: Some(redirect),
        }
    }

    /// Return `true` if this error code indicates the request should not be retried.
    pub fn is_fatal(&self) -> bool {
        self.retry_interval == 0
    }
}

impl Decode for RequestError {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let id = u64::decode(r)?;
        let error_code = u64::decode(r)?;
        let retry_interval = u64::decode(r)?;
        let reason = ReasonPhrase::decode(r)?;
        let redirect = if error_code == RequestErrorCode::Redirect as u64 {
            Some(Redirect::decode(r)?)
        } else {
            None
        };
        Ok(Self {
            id,
            error_code,
            retry_interval,
            reason,
            redirect,
        })
    }
}

impl Encode for RequestError {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.id.encode(w)?;
        self.error_code.encode(w)?;
        self.retry_interval.encode(w)?;
        self.reason.encode(w)?;
        match (
            self.error_code == RequestErrorCode::Redirect as u64,
            &self.redirect,
        ) {
            (true, Some(redirect)) => redirect.encode(w)?,
            (true, None) => {
                return Err(EncodeError::MissingField("Redirect".to_string()));
            }
            (false, Some(_)) => return Err(EncodeError::InvalidValue),
            (false, None) => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    #[test]
    fn encode_decode() {
        let mut buf = BytesMut::new();
        let msg = RequestError {
            id: 42,
            error_code: RequestErrorCode::DoesNotExist as u64,
            retry_interval: 0,
            reason: ReasonPhrase("track not found".to_string()),
            redirect: None,
        };
        msg.encode(&mut buf).unwrap();
        let decoded = RequestError::decode(&mut buf).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn encode_decode_with_retry() {
        let mut buf = BytesMut::new();
        let msg = RequestError::new(10, RequestErrorCode::Timeout, 5001, "upstream timeout");
        msg.encode(&mut buf).unwrap();
        let decoded = RequestError::decode(&mut buf).unwrap();
        assert_eq!(decoded.id, 10);
        assert_eq!(decoded.error_code, RequestErrorCode::Timeout as u64);
        assert_eq!(decoded.retry_interval, 5001);
        assert!(!decoded.is_fatal());
    }

    #[test]
    fn is_fatal_when_retry_interval_zero() {
        let msg = RequestError {
            id: 1,
            error_code: 0,
            retry_interval: 0,
            reason: ReasonPhrase(String::new()),
            redirect: None,
        };
        assert!(msg.is_fatal());
    }

    #[test]
    fn subscribe_rejection_uses_request_error() {
        // Verify that subscription rejections can be expressed as REQUEST_ERROR
        // with the correct error code.
        let mut buf = bytes::BytesMut::new();
        let msg = RequestError::new(0, RequestErrorCode::DoesNotExist, 0, "track not found");
        msg.encode(&mut buf).unwrap();
        let decoded = RequestError::decode(&mut buf).unwrap();
        assert_eq!(decoded.error_code, RequestErrorCode::DoesNotExist as u64);
        assert!(decoded.is_fatal());
    }

    #[test]
    fn not_supported_response_round_trips() {
        // Verify that a NOT_SUPPORTED response encodes, decodes, and is fatal (retry_interval=0).
        let mut buf = bytes::BytesMut::new();
        let msg = RequestError::new(10, RequestErrorCode::NotSupported, 0, "not supported");
        msg.encode(&mut buf).unwrap();
        let decoded = RequestError::decode(&mut buf).unwrap();
        assert_eq!(decoded.id, 10);
        assert_eq!(decoded.error_code, RequestErrorCode::NotSupported as u64);
        assert!(decoded.is_fatal());
    }

    #[test]
    fn redirect_round_trips() {
        let mut buf = BytesMut::new();
        let redirect = Redirect {
            connect_uri: SessionUri("https://relay.example/moq".to_string()),
            track_namespace: TrackNamespacePrefix::from_utf8_path("tenant/live"),
            track_name: "audio".into(),
        };
        let msg = RequestError {
            id: 8,
            error_code: RequestErrorCode::Redirect as u64,
            retry_interval: 1,
            reason: ReasonPhrase("moved".to_string()),
            redirect: Some(redirect),
        };

        msg.encode(&mut buf).unwrap();
        assert_eq!(RequestError::decode(&mut buf).unwrap(), msg);
    }

    #[test]
    fn draft_19_redirect_golden_body() {
        let mut buf = BytesMut::new();
        RequestError {
            id: 2,
            error_code: RequestErrorCode::Redirect as u64,
            retry_interval: 0,
            reason: ReasonPhrase::default(),
            redirect: Some(Redirect {
                connect_uri: SessionUri::default(),
                track_namespace: TrackNamespacePrefix::new(),
                track_name: TrackName::default(),
            }),
        }
        .encode(&mut buf)
        .unwrap();

        assert_eq!(&buf[..], &[0x02, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn redirect_presence_matches_error_code() {
        let mut buf = BytesMut::new();
        let mut msg = RequestError::new(0, RequestErrorCode::Redirect, 0, "moved");
        assert!(matches!(
            msg.encode(&mut buf),
            Err(EncodeError::MissingField(_))
        ));

        msg.error_code = RequestErrorCode::InternalError as u64;
        msg.redirect = Some(Redirect {
            connect_uri: SessionUri::default(),
            track_namespace: TrackNamespacePrefix::new(),
            track_name: TrackName::default(),
        });
        assert!(matches!(
            msg.encode(&mut buf),
            Err(EncodeError::InvalidValue)
        ));
    }
}
