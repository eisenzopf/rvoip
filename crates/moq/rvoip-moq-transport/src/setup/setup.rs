// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Unified SETUP message (draft-ietf-moq-transport-19 §10.3).
//!
//! In draft-19 both peers send the same SETUP message on their respective
//! unidirectional control streams. The message type (`0x2F00`) doubles as
//! the stream type identifier. Setup Options are length-bounded KVPs
//! (no count prefix).
//!
//! ```text
//! SETUP Message {
//!   Type (vi64) = 0x2F00,
//!   Length (16),
//!   Setup Options (..) ...,
//! }
//! ```

use crate::coding::{
    AuthorizationToken, Decode, DecodeError, Encode, EncodeError, KeyValuePairs, Value,
};
use crate::setup::ParameterType;

/// The SETUP message type, which also serves as the control stream type.
pub const SETUP_TYPE: u64 = 0x2F00;

/// Sent by both peers to establish the session (draft-19).
///
/// Replaces the separate CLIENT_SETUP (0x20) and SERVER_SETUP (0x21)
/// from earlier drafts. Version negotiation is handled entirely by ALPN;
/// this message carries only Setup Options (PATH, AUTHORITY, etc.).
pub struct Setup {
    /// Setup Options encoded as length-bounded KVPs.
    pub params: KeyValuePairs,
}

impl std::fmt::Debug for Setup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let authorization_key: u64 = ParameterType::AuthorizationToken.into();
        let path_key: u64 = ParameterType::Path.into();
        let authority_key: u64 = ParameterType::Authority.into();
        let params = self
            .params
            .0
            .iter()
            .map(|pair| {
                (
                    pair.key,
                    if pair.key == authorization_key {
                        "<redacted>".to_string()
                    } else if pair.key == authority_key {
                        "<redacted-authority>".to_string()
                    } else if pair.key == path_key {
                        match &pair.value {
                            Value::BytesValue(bytes) => {
                                let path = bytes
                                    .split(|byte| *byte == b'?')
                                    .next()
                                    .and_then(|path| std::str::from_utf8(path).ok())
                                    .unwrap_or("<invalid-path>");
                                if bytes.contains(&b'?') {
                                    format!("{path}?<redacted>")
                                } else {
                                    path.to_string()
                                }
                            }
                            _ => "<invalid-path>".to_string(),
                        }
                    } else {
                        format!("{:?}", pair.value)
                    },
                )
            })
            .collect::<Vec<_>>();
        formatter
            .debug_struct("Setup")
            .field("params", &params)
            .finish()
    }
}

impl Setup {
    /// Number of authorization options in this SETUP message.
    pub fn authorization_token_count(&self) -> usize {
        self.params
            .get_all(ParameterType::AuthorizationToken.into())
            .count()
    }

    /// Parse every authorization option in wire order.
    pub fn authorization_tokens(&self) -> Result<Vec<AuthorizationToken>, DecodeError> {
        self.params
            .get_all(ParameterType::AuthorizationToken.into())
            .map(|pair| match &pair.value {
                Value::BytesValue(value) => AuthorizationToken::decode_bytes(value),
                Value::IntValue(_) => Err(DecodeError::AuthorizationTokenFormatting(
                    "authorization option is not byte-valued".into(),
                )),
            })
            .collect()
    }

    /// Compatibility accessor that returns a token only when there is exactly
    /// one. Callers that support repeated authorization must use
    /// [`Self::authorization_tokens`].
    pub fn authorization_token(&self) -> Result<Option<AuthorizationToken>, DecodeError> {
        let mut tokens = self.authorization_tokens()?.into_iter();
        let first = tokens.next();
        Ok(match (first, tokens.next()) {
            (Some(token), None) => Some(token),
            _ => None,
        })
    }

    /// Maximum number of unacknowledged REQUEST_UPDATE messages this peer is
    /// willing to receive on one request stream. Zero means unlimited and is
    /// also the default when the option is absent.
    pub fn max_request_updates(&self) -> Result<u64, DecodeError> {
        match self
            .params
            .get(crate::setup::ParameterType::MaxRequestUpdates.into())
            .map(|pair| &pair.value)
        {
            Some(Value::IntValue(value)) => Ok(*value),
            Some(Value::BytesValue(_)) => Err(DecodeError::InvalidParameter),
            None => Ok(0),
        }
    }

    /// Decode a SETUP message, assuming the stream type / message type
    /// varint has already been read and matched against `SETUP_TYPE`.
    ///
    /// This is the typical path when accepting a control stream: the
    /// caller reads the stream type varint first to dispatch, then calls
    /// this to parse the remainder.
    pub fn decode_after_type<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let len = u16::decode(r)? as usize;
        let params = KeyValuePairs::decode_bounded(r, len)?;
        let setup = Self { params };
        setup.validate_options()?;
        // Parse now so malformed authorization structures close the session as
        // KEY_VALUE_FORMATTING_ERROR instead of reaching admission as bytes.
        setup.authorization_tokens()?;
        Ok(setup)
    }

    fn validate_options(&self) -> Result<(), DecodeError> {
        for option in [
            ParameterType::Path,
            ParameterType::MaxAuthTokenCacheSize,
            ParameterType::Authority,
            ParameterType::MaxFilterRanges,
            ParameterType::MOQTImplementation,
            ParameterType::MaxRequestUpdates,
        ] {
            let key = option.into();
            if self.params.get_all(key).count() > 1 {
                return Err(DecodeError::DuplicateParameter(key));
            }
        }
        Ok(())
    }

    /// Encode the full SETUP message including the type prefix.
    ///
    /// This writes the complete message: type varint + u16 length + KVP payload.
    /// Used when opening a control stream (the type varint doubles as the
    /// stream type identifier).
    pub fn encode_full<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        // Type / stream-type prefix
        SETUP_TYPE.encode(w)?;

        // Encode options into a temporary buffer to measure length
        let payload = self.params.encode_bounded()?;

        if payload.len() > u16::MAX as usize {
            return Err(EncodeError::MsgBoundsExceeded);
        }
        (payload.len() as u16).encode(w)?;
        Self::encode_remaining(w, payload.len())?;
        w.put_slice(&payload);

        Ok(())
    }
}

// Standard Decode reads the type prefix then delegates.
impl Decode for Setup {
    fn decode<R: bytes::Buf>(r: &mut R) -> Result<Self, DecodeError> {
        let typ = u64::decode(r)?;
        if typ != SETUP_TYPE {
            return Err(DecodeError::InvalidMessage(typ));
        }
        Self::decode_after_type(r)
    }
}

// Standard Encode writes the full message.
impl Encode for Setup {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        self.encode_full(w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::ParameterType;
    use bytes::Buf as _;
    use bytes::BytesMut;

    #[test]
    fn encode_decode_with_path() {
        let mut buf = BytesMut::new();

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(ParameterType::Path.into(), b"/moq".to_vec());

        let setup = Setup { params };
        setup.encode(&mut buf).unwrap();

        // Verify wire layout:
        // Type: 0x2F00 as varint = [0xAF, 0x00] (2 bytes)
        // Length: u16
        // Options: delta=1 (PATH), length=4, "/moq"
        assert_eq!(buf[0], 0xAF); // first byte of varint 0x2F00
        assert_eq!(buf[1], 0x00);

        let decoded = Setup::decode(&mut buf).unwrap();
        assert_eq!(decoded.params, setup.params);
    }

    #[test]
    fn encode_decode_empty() {
        let mut buf = BytesMut::new();
        let setup = Setup {
            params: KeyValuePairs::default(),
        };
        setup.encode(&mut buf).unwrap();

        // Type (2B) + Length (2B, value=0) + no options
        assert_eq!(buf.len(), 4);
        assert_eq!(buf[2], 0x00); // length high byte
        assert_eq!(buf[3], 0x00); // length low byte

        let decoded = Setup::decode(&mut buf).unwrap();
        assert!(decoded.params.0.is_empty());
    }

    #[test]
    fn decode_after_type() {
        let mut buf = BytesMut::new();

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(ParameterType::Path.into(), b"/test".to_vec());
        // MaxAuthTokenCacheSize used as a representative non-trivial parameter
        // (MaxRequestId was removed in draft-18 §A.1; any valid parameter type works here).
        params.set_intvalue(ParameterType::MaxAuthTokenCacheSize.into(), 42);

        let setup = Setup { params };
        setup.encode(&mut buf).unwrap();

        // Skip the type varint (0x2F00 = 2 bytes)
        buf.advance(2);
        let decoded = Setup::decode_after_type(&mut buf).unwrap();
        assert_eq!(decoded.params, setup.params);
    }

    #[test]
    fn decode_rejects_wrong_type() {
        let mut buf = BytesMut::new();
        (0x20_u64).encode(&mut buf).unwrap(); // old CLIENT_SETUP type
        (0_u16).encode(&mut buf).unwrap();

        assert!(matches!(
            Setup::decode(&mut buf).unwrap_err(),
            DecodeError::InvalidMessage(0x20)
        ));
    }

    #[test]
    fn round_trip_multiple_options() {
        let mut buf = BytesMut::new();

        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(ParameterType::Path.into(), b"/live/stream".to_vec());
        // MaxAuthTokenCacheSize as stand-in (MaxRequestId removed in draft-18 §A.1).
        params.set_intvalue(ParameterType::MaxAuthTokenCacheSize.into(), 1000);
        params.set_bytesvalue(
            ParameterType::Authority.into(),
            b"relay.example.com".to_vec(),
        );

        let setup = Setup { params };
        setup.encode(&mut buf).unwrap();
        let decoded = Setup::decode(&mut buf).unwrap();

        assert_eq!(decoded.params.0.len(), 3);
        assert_eq!(decoded.params, setup.params);
    }

    #[test]
    fn max_request_updates_draft_19_golden_encoding() {
        let mut params = KeyValuePairs::default();
        params.set_intvalue(ParameterType::MaxRequestUpdates.into(), 4);

        let mut buf = BytesMut::new();
        Setup { params }.encode(&mut buf).unwrap();

        // SETUP type, two-byte option block length, then option type 0x08/value 4.
        assert_eq!(buf.to_vec(), vec![0xAF, 0x00, 0x00, 0x02, 0x08, 0x04]);
        let decoded = Setup::decode(&mut buf).unwrap();
        assert_eq!(
            decoded
                .params
                .get(ParameterType::MaxRequestUpdates.into())
                .map(|pair| &pair.value),
            Some(&crate::coding::Value::IntValue(4))
        );
    }

    #[test]
    fn debug_redacts_authorization_token() {
        let secret = b"setup-secret";
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(ParameterType::AuthorizationToken.into(), secret.to_vec());
        params.set_intvalue(ParameterType::MaxRequestUpdates.into(), 4);
        let debug = format!("{:?}", Setup { params });
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("setup-secret"));
        assert!(debug.contains("(8, \"4\")"));
    }

    #[test]
    fn debug_redacts_path_query() {
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(
            ParameterType::Path.into(),
            b"/tenant/live?token=setup-secret".to_vec(),
        );
        let debug = format!("{:?}", Setup { params });
        assert!(debug.contains("/tenant/live?<redacted>"));
        assert!(!debug.contains("setup-secret"));
    }

    #[test]
    fn debug_never_emits_setup_authority_userinfo() {
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(
            ParameterType::Authority.into(),
            b"user:password@relay.example".to_vec(),
        );
        let debug = format!("{:?}", Setup { params });
        assert!(debug.contains("<redacted-authority>"));
        assert!(!debug.contains("user"));
        assert!(!debug.contains("password"));
        assert!(!debug.contains("75 73 65 72"));
    }

    #[test]
    fn decode_rejects_duplicate_path_and_authority_options() {
        for key in [ParameterType::Path, ParameterType::Authority] {
            let key: u64 = key.into();
            let params = KeyValuePairs(vec![
                crate::coding::KeyValuePair::new_bytes(key, b"/first".to_vec()),
                crate::coding::KeyValuePair::new_bytes(key, b"/second".to_vec()),
            ]);
            let mut encoded = bytes::BytesMut::new();
            Setup { params }.encode(&mut encoded).unwrap();
            assert!(matches!(
                Setup::decode(&mut encoded),
                Err(DecodeError::DuplicateParameter(duplicate)) if duplicate == key
            ));
        }
    }

    #[test]
    fn decode_rejects_duplicate_max_request_updates() {
        let key: u64 = ParameterType::MaxRequestUpdates.into();
        let params = KeyValuePairs(vec![
            crate::coding::KeyValuePair::new_int(key, 4),
            crate::coding::KeyValuePair::new_int(key, 8),
        ]);
        let mut encoded = bytes::BytesMut::new();
        Setup { params }.encode(&mut encoded).unwrap();
        assert!(matches!(
            Setup::decode(&mut encoded),
            Err(DecodeError::DuplicateParameter(duplicate)) if duplicate == key
        ));
    }

    #[test]
    fn decode_preserves_repeated_authorization_and_unknown_options() {
        let authorization_key: u64 = ParameterType::AuthorizationToken.into();
        let params = KeyValuePairs(vec![
            crate::coding::KeyValuePair::new_bytes(
                authorization_key,
                AuthorizationToken::use_value(0, b"first")
                    .unwrap()
                    .encode_bytes()
                    .unwrap(),
            ),
            crate::coding::KeyValuePair::new_bytes(
                authorization_key,
                AuthorizationToken::use_value(7, b"second")
                    .unwrap()
                    .encode_bytes()
                    .unwrap(),
            ),
            crate::coding::KeyValuePair::new_bytes(0x7f, vec![1]),
            crate::coding::KeyValuePair::new_bytes(0x7f, vec![2]),
        ]);
        let mut encoded = bytes::BytesMut::new();
        Setup { params }.encode(&mut encoded).unwrap();
        let decoded = Setup::decode(&mut encoded).unwrap();
        assert_eq!(decoded.authorization_token_count(), 2);
        assert_eq!(decoded.authorization_tokens().unwrap().len(), 2);
        assert_eq!(decoded.authorization_token().unwrap(), None);
        assert_eq!(decoded.params.get_all(0x7f).count(), 2);
    }

    #[test]
    fn malformed_authorization_structure_is_a_formatting_error() {
        let mut params = KeyValuePairs::default();
        params.set_bytesvalue(ParameterType::AuthorizationToken.into(), vec![0x03]);
        let mut encoded = bytes::BytesMut::new();
        Setup { params }.encode(&mut encoded).unwrap();
        assert!(matches!(
            Setup::decode(&mut encoded),
            Err(DecodeError::AuthorizationTokenFormatting(_))
        ));
    }
}
