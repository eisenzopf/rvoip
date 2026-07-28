// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Structured authorization tokens from draft-ietf-moq-transport-19 §10.2.2.

use bytes::{Buf as _, Bytes};

use super::{Decode, DecodeError, Encode, EncodeError};

/// Maximum authorization token value admitted by this implementation.
///
/// The protocol KVP envelope permits larger values, but authorization material
/// is peer-controlled and retained until admission completes.
pub const MAX_AUTHORIZATION_TOKEN_VALUE_LEN: usize = 4 * 1024;

/// The wire alias operation at the start of an authorization token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum AuthorizationTokenAliasType {
    Delete = 0,
    Register = 1,
    UseAlias = 2,
    UseValue = 3,
}

impl TryFrom<u64> for AuthorizationTokenAliasType {
    type Error = DecodeError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Delete),
            1 => Ok(Self::Register),
            2 => Ok(Self::UseAlias),
            3 => Ok(Self::UseValue),
            value => Err(DecodeError::AuthorizationTokenFormatting(format!(
                "unknown alias type {value}"
            ))),
        }
    }
}

/// A parsed authorization token. Token values are never included in `Debug`.
#[derive(Clone, Eq, PartialEq)]
pub enum AuthorizationToken {
    Delete {
        alias: u64,
    },
    Register {
        alias: u64,
        token_type: u64,
        value: Bytes,
    },
    UseAlias {
        alias: u64,
    },
    UseValue {
        token_type: u64,
        value: Bytes,
    },
}

impl AuthorizationToken {
    pub fn delete(alias: u64) -> Self {
        Self::Delete { alias }
    }

    pub fn register(
        alias: u64,
        token_type: u64,
        value: impl AsRef<[u8]>,
    ) -> Result<Self, DecodeError> {
        Ok(Self::Register {
            alias,
            token_type,
            value: bounded_value(value.as_ref())?,
        })
    }

    pub fn use_alias(alias: u64) -> Self {
        Self::UseAlias { alias }
    }

    pub fn use_value(token_type: u64, value: impl AsRef<[u8]>) -> Result<Self, DecodeError> {
        Ok(Self::UseValue {
            token_type,
            value: bounded_value(value.as_ref())?,
        })
    }

    pub const fn alias_type(&self) -> AuthorizationTokenAliasType {
        match self {
            Self::Delete { .. } => AuthorizationTokenAliasType::Delete,
            Self::Register { .. } => AuthorizationTokenAliasType::Register,
            Self::UseAlias { .. } => AuthorizationTokenAliasType::UseAlias,
            Self::UseValue { .. } => AuthorizationTokenAliasType::UseValue,
        }
    }

    pub const fn alias(&self) -> Option<u64> {
        match self {
            Self::Delete { alias } | Self::Register { alias, .. } | Self::UseAlias { alias } => {
                Some(*alias)
            }
            Self::UseValue { .. } => None,
        }
    }

    pub const fn token_type(&self) -> Option<u64> {
        match self {
            Self::Register { token_type, .. } | Self::UseValue { token_type, .. } => {
                Some(*token_type)
            }
            Self::Delete { .. } | Self::UseAlias { .. } => None,
        }
    }

    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Register { value, .. } | Self::UseValue { value, .. } => Some(value),
            Self::Delete { .. } | Self::UseAlias { .. } => None,
        }
    }

    /// Decode one token from exactly one KVP byte value.
    pub fn decode_bytes(value: impl AsRef<[u8]>) -> Result<Self, DecodeError> {
        let mut value = Bytes::copy_from_slice(value.as_ref());
        let token = Self::decode(&mut value)?;
        debug_assert!(!value.has_remaining());
        Ok(token)
    }

    /// Encode one token as the byte value carried by an authorization KVP.
    pub fn encode_bytes(&self) -> Result<Vec<u8>, EncodeError> {
        let mut encoded = Vec::new();
        self.encode(&mut encoded)?;
        Ok(encoded)
    }
}

fn bounded_value(value: &[u8]) -> Result<Bytes, DecodeError> {
    if value.len() > MAX_AUTHORIZATION_TOKEN_VALUE_LEN {
        return Err(DecodeError::AuthorizationTokenFormatting(format!(
            "token value exceeds {MAX_AUTHORIZATION_TOKEN_VALUE_LEN} bytes"
        )));
    }
    Ok(Bytes::copy_from_slice(value))
}

impl Decode for AuthorizationToken {
    fn decode<B: bytes::Buf>(buf: &mut B) -> Result<Self, DecodeError> {
        let alias_type =
            AuthorizationTokenAliasType::try_from(u64::decode(buf).map_err(|error| {
                DecodeError::AuthorizationTokenFormatting(format!(
                    "missing or invalid alias type: {error}"
                ))
            })?)?;

        let decode_field = |buf: &mut B, field: &str| {
            u64::decode(buf).map_err(|error| {
                DecodeError::AuthorizationTokenFormatting(format!(
                    "missing or invalid {field}: {error}"
                ))
            })
        };

        match alias_type {
            AuthorizationTokenAliasType::Delete => {
                let alias = decode_field(buf, "alias")?;
                if buf.has_remaining() {
                    return Err(DecodeError::AuthorizationTokenFormatting(
                        "DELETE contains trailing bytes".into(),
                    ));
                }
                Ok(Self::Delete { alias })
            }
            AuthorizationTokenAliasType::Register => {
                let alias = decode_field(buf, "alias")?;
                let token_type = decode_field(buf, "token type")?;
                let mut value = vec![0; buf.remaining()];
                buf.copy_to_slice(&mut value);
                Ok(Self::Register {
                    alias,
                    token_type,
                    value: bounded_value(&value)?,
                })
            }
            AuthorizationTokenAliasType::UseAlias => {
                let alias = decode_field(buf, "alias")?;
                if buf.has_remaining() {
                    return Err(DecodeError::AuthorizationTokenFormatting(
                        "USE_ALIAS contains trailing bytes".into(),
                    ));
                }
                Ok(Self::UseAlias { alias })
            }
            AuthorizationTokenAliasType::UseValue => {
                let token_type = decode_field(buf, "token type")?;
                let mut value = vec![0; buf.remaining()];
                buf.copy_to_slice(&mut value);
                Ok(Self::UseValue {
                    token_type,
                    value: bounded_value(&value)?,
                })
            }
        }
    }
}

impl Encode for AuthorizationToken {
    fn encode<W: bytes::BufMut>(&self, w: &mut W) -> Result<(), EncodeError> {
        (self.alias_type() as u64).encode(w)?;
        match self {
            Self::Delete { alias } | Self::UseAlias { alias } => alias.encode(w),
            Self::Register {
                alias,
                token_type,
                value,
            } => {
                if value.len() > MAX_AUTHORIZATION_TOKEN_VALUE_LEN {
                    return Err(EncodeError::FieldBoundsExceeded(
                        "authorization token value".into(),
                    ));
                }
                alias.encode(w)?;
                token_type.encode(w)?;
                bytes::BufMut::put_slice(w, value);
                Ok(())
            }
            Self::UseValue { token_type, value } => {
                if value.len() > MAX_AUTHORIZATION_TOKEN_VALUE_LEN {
                    return Err(EncodeError::FieldBoundsExceeded(
                        "authorization token value".into(),
                    ));
                }
                token_type.encode(w)?;
                bytes::BufMut::put_slice(w, value);
                Ok(())
            }
        }
    }
}

impl std::fmt::Debug for AuthorizationToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("AuthorizationToken");
        debug.field("alias_type", &self.alias_type());
        if let Some(alias) = self.alias() {
            debug.field("alias", &alias);
        }
        if let Some(token_type) = self.token_type() {
            debug.field("token_type", &token_type);
        }
        if let Some(value) = self.value() {
            debug.field("value", &format_args!("<redacted:{}>", value.len()));
        }
        debug.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn published_vi64_values_form_golden_token_vectors() {
        let vectors = [
            (AuthorizationToken::delete(37), vec![0x00, 0x25]),
            (
                AuthorizationToken::register(37, 15_293, b"abc").unwrap(),
                vec![0x01, 0x25, 0xbb, 0xbd, b'a', b'b', b'c'],
            ),
            (AuthorizationToken::use_alias(37), vec![0x02, 0x25]),
            (
                AuthorizationToken::use_value(0, b"abc").unwrap(),
                vec![0x03, 0x00, b'a', b'b', b'c'],
            ),
        ];

        for (token, wire) in vectors {
            assert_eq!(token.encode_bytes().unwrap(), wire);
            assert_eq!(AuthorizationToken::decode_bytes(&wire).unwrap(), token);
        }
    }

    #[test]
    fn non_minimal_vi64_fields_are_accepted() {
        let token = AuthorizationToken::decode_bytes([0x80, 0x03, 0x80, 0x25, b'x']).unwrap();
        assert_eq!(token, AuthorizationToken::use_value(37, b"x").unwrap());
    }

    #[test]
    fn malformed_alias_forms_and_unknown_types_are_rejected() {
        for malformed in [
            vec![],
            vec![0x04],
            vec![0x00],
            vec![0x02],
            vec![0x00, 0x25, 0xff],
            vec![0x02, 0x25, 0xff],
            vec![0x01, 0x25],
            vec![0x03],
        ] {
            assert!(matches!(
                AuthorizationToken::decode_bytes(malformed),
                Err(DecodeError::AuthorizationTokenFormatting(_))
            ));
        }
    }

    #[test]
    fn debug_redacts_token_value() {
        let token = AuthorizationToken::use_value(7, b"never-log-me").unwrap();
        let debug = format!("{token:?}");
        assert!(debug.contains("redacted:12"));
        assert!(!debug.contains("never-log-me"));
    }

    #[test]
    fn token_values_are_bounded_before_admission() {
        let oversized = vec![0; MAX_AUTHORIZATION_TOKEN_VALUE_LEN + 1];
        assert!(matches!(
            AuthorizationToken::use_value(0, &oversized),
            Err(DecodeError::AuthorizationTokenFormatting(_))
        ));

        let mut wire = vec![0x03, 0x00];
        wire.extend_from_slice(&oversized);
        assert!(matches!(
            AuthorizationToken::decode_bytes(wire),
            Err(DecodeError::AuthorizationTokenFormatting(_))
        ));
    }
}
