//! # SIP Authentication Parameters
//!
//! This module defines various parameter types used in SIP authentication headers.

use crate::types::auth::scheme::{Algorithm, Qop};
use crate::types::uri::Uri;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Generic Authentication Parameter (name=value)
///
/// Represents a generic name-value parameter used in SIP authentication headers.
/// Parameters are typically presented as `name="value"` pairs in header fields.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthParam {
    /// Parameter name
    pub name: String,
    /// Parameter value
    pub value: String,
}

impl fmt::Debug for AuthParam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthParam")
            .field("name_bytes", &self.name.len())
            .field("value_bytes", &self.value.len())
            .finish()
    }
}

impl fmt::Display for AuthParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}=\"{}\"", self.name, self.value)
    }
}

/// Parameters specific to Digest authentication (used in Challenge and Credentials)
///
/// This enum represents the various parameters that can appear in Digest authentication
/// challenges and credentials as defined in RFC 3261 and RFC 7616.
///
/// Different parameters are used depending on whether they appear in:
/// - Server-issued challenges (WWW-Authenticate/Proxy-Authenticate headers)
/// - Client-provided credentials (Authorization/Proxy-Authorization headers)
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigestParam {
    // Challenge & Credentials
    /// Authentication realm (mandatory in challenges and credentials)
    Realm(String),
    /// Server-generated nonce (mandatory in challenges and credentials)
    Nonce(String),
    /// Opaque data from server (Optional in challenge, MUST be returned if present)
    Opaque(String),
    /// Hashing algorithm (Optional in both challenge and credentials)
    Algorithm(Algorithm),
    // Challenge Only
    /// List of URIs that share credentials (Optional in challenges)
    Domain(Vec<String>),
    /// Indicates if the nonce is stale (Optional in challenges)
    Stale(bool),
    /// Quality of protection options (Optional in challenges)
    Qop(Vec<Qop>),
    // Credentials Only
    /// User's username (Mandatory in credentials)
    Username(String),
    /// Request URI (Mandatory in credentials)
    Uri(Uri),
    /// Digest response hash (Mandatory in credentials)
    Response(String),
    /// Client nonce (Mandatory if QOP is used)
    Cnonce(String),
    /// Quality of protection used (Mandatory if QOP is offered)
    MsgQop(Qop),
    /// Nonce count (Mandatory if QOP is used)
    NonceCount(u32),
    // Generic fallback
    /// Generic parameter not specifically typed above
    Param(AuthParam),
}

impl fmt::Debug for DigestParam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("DigestParam");
        match self {
            Self::Realm(value) => debug
                .field("kind", &"realm")
                .field("value_bytes", &value.len()),
            Self::Nonce(value) => debug
                .field("kind", &"nonce")
                .field("value_bytes", &value.len()),
            Self::Opaque(value) => debug
                .field("kind", &"opaque")
                .field("value_bytes", &value.len()),
            Self::Algorithm(value) => debug.field("kind", &"algorithm").field("algorithm", value),
            Self::Domain(values) => debug
                .field("kind", &"domain")
                .field("value_count", &values.len()),
            Self::Stale(value) => debug.field("kind", &"stale").field("stale", value),
            Self::Qop(values) => debug
                .field("kind", &"qop-list")
                .field("value_count", &values.len()),
            Self::Username(value) => debug
                .field("kind", &"username")
                .field("value_bytes", &value.len()),
            Self::Uri(_) => debug.field("kind", &"uri"),
            Self::Response(value) => debug
                .field("kind", &"response")
                .field("value_bytes", &value.len()),
            Self::Cnonce(value) => debug
                .field("kind", &"client-nonce")
                .field("value_bytes", &value.len()),
            Self::MsgQop(value) => debug.field("kind", &"qop").field("qop", value),
            Self::NonceCount(_) => debug.field("kind", &"nonce-count"),
            Self::Param(value) => debug.field("kind", &"extension").field("value", value),
        };
        debug.finish()
    }
}

impl fmt::Display for DigestParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DigestParam::Realm(v) => write!(f, "realm=\"{}\"", v),
            DigestParam::Nonce(v) => write!(f, "nonce=\"{}\"", v),
            DigestParam::Opaque(v) => write!(f, "opaque=\"{}\"", v),
            DigestParam::Algorithm(v) => write!(f, "algorithm={}", v),
            DigestParam::Domain(v) => write!(f, "domain=\"{}\"", v.join(", ")),
            DigestParam::Stale(v) => write!(f, "stale={}", v),
            DigestParam::Qop(v) => write!(
                f,
                "qop={}",
                v.iter()
                    .map(|q| q.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            DigestParam::Username(v) => write!(f, "username=\"{}\"", v),
            DigestParam::Uri(v) => write!(f, "uri=\"{}\"", v),
            DigestParam::Response(v) => write!(f, "response=\"{}\"", v),
            DigestParam::Cnonce(v) => write!(f, "cnonce=\"{}\"", v),
            DigestParam::MsgQop(v) => write!(f, "qop={}", v),
            DigestParam::NonceCount(v) => write!(f, "nc={:08x}", v),
            DigestParam::Param(p) => write!(f, "{}", p),
        }
    }
}

/// Parameters specific to Authentication-Info header
///
/// These parameters are used in the Authentication-Info header field, which is sent by
/// servers after successful authentication to provide additional information to clients.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthenticationInfoParam {
    /// Next nonce to be used by the client
    NextNonce(String),
    /// Quality of protection used
    Qop(Qop), // Only one value allowed
    /// Server authentication response (mutual authentication)
    ResponseAuth(String), // rspauth (hex)
    /// Client nonce (echoed from the client's request)
    Cnonce(String),
    /// Nonce count (echoed from the client's request)
    NonceCount(u32), // nc-value (hex, parsed to u32)
}

impl fmt::Debug for AuthenticationInfoParam {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("AuthenticationInfoParam");
        match self {
            Self::NextNonce(value) => debug
                .field("kind", &"next-nonce")
                .field("value_bytes", &value.len()),
            Self::Qop(value) => debug.field("kind", &"qop").field("qop", value),
            Self::ResponseAuth(value) => debug
                .field("kind", &"response-auth")
                .field("value_bytes", &value.len()),
            Self::Cnonce(value) => debug
                .field("kind", &"client-nonce")
                .field("value_bytes", &value.len()),
            Self::NonceCount(_) => debug.field("kind", &"nonce-count"),
        };
        debug.finish()
    }
}

impl fmt::Display for AuthenticationInfoParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthenticationInfoParam::NextNonce(v) => write!(f, "nextnonce=\"{}\"", v),
            AuthenticationInfoParam::Qop(v) => write!(f, "qop={}", v),
            AuthenticationInfoParam::ResponseAuth(v) => write!(f, "rspauth=\"{}\"", v),
            AuthenticationInfoParam::Cnonce(v) => write!(f, "cnonce=\"{}\"", v),
            AuthenticationInfoParam::NonceCount(v) => write!(f, "nc={:08x}", v),
        }
    }
}

#[cfg(test)]
mod diagnostic_safety_tests {
    use super::*;
    use std::str::FromStr;

    const SECRET: &str = "auth-param-direct-debug-canary";

    #[test]
    fn authentication_parameter_debug_is_metadata_only_without_changing_wire_or_serde() {
        let uri = Uri::from_str(&format!("sip:{SECRET}@example.invalid")).unwrap();
        let digest = vec![
            DigestParam::Realm(SECRET.into()),
            DigestParam::Nonce(SECRET.into()),
            DigestParam::Opaque(SECRET.into()),
            DigestParam::Algorithm(Algorithm::Other(SECRET.into())),
            DigestParam::Domain(vec![SECRET.into()]),
            DigestParam::Qop(vec![Qop::Other(SECRET.into())]),
            DigestParam::Username(SECRET.into()),
            DigestParam::Uri(uri),
            DigestParam::Response(SECRET.into()),
            DigestParam::Cnonce(SECRET.into()),
            DigestParam::MsgQop(Qop::Other(SECRET.into())),
            DigestParam::Param(AuthParam {
                name: SECRET.into(),
                value: SECRET.into(),
            }),
        ];
        let info = vec![
            AuthenticationInfoParam::NextNonce(SECRET.into()),
            AuthenticationInfoParam::Qop(Qop::Other(SECRET.into())),
            AuthenticationInfoParam::ResponseAuth(SECRET.into()),
            AuthenticationInfoParam::Cnonce(SECRET.into()),
        ];

        for value in &digest {
            assert!(!format!("{value:?}").contains(SECRET));
        }
        for value in &info {
            assert!(!format!("{value:?}").contains(SECRET));
        }

        let extension = AuthParam {
            name: SECRET.into(),
            value: SECRET.into(),
        };
        assert!(!format!("{extension:?}").contains(SECRET));
        assert!(extension.to_string().contains(SECRET));
        assert!(digest
            .iter()
            .all(|value| value.to_string().contains(SECRET)));
        assert!(info.iter().all(|value| value.to_string().contains(SECRET)));
        assert!(serde_json::to_string(&digest).unwrap().contains(SECRET));
        assert!(serde_json::to_string(&info).unwrap().contains(SECRET));
    }

    #[test]
    fn authentication_parameter_types_cannot_regain_derived_debug() {
        let source = include_str!("params.rs");
        for declaration in [
            "pub struct AuthParam",
            "pub enum DigestParam",
            "pub enum AuthenticationInfoParam",
        ] {
            let declaration_offset = source.find(declaration).unwrap();
            let derive_offset = source[..declaration_offset].rfind("#[derive(").unwrap();
            assert!(!source[derive_offset..declaration_offset].contains("Debug"));
        }
    }
}
