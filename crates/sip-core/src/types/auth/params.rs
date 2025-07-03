//! # SIP Authentication Parameters
//!
//! This module defines various parameter types used in SIP authentication headers.

use std::fmt;
use serde::{Deserialize, Serialize};
use crate::types::uri::Uri;
use crate::types::auth::scheme::{Algorithm, Qop};

/// Generic Authentication Parameter (name=value)
///
/// Represents a generic name-value parameter used in SIP authentication headers.
/// Parameters are typically presented as `name="value"` pairs in header fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthParam {
    /// Parameter name
    pub name: String,
    /// Parameter value
    pub value: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl fmt::Display for DigestParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DigestParam::Realm(v) => write!(f, "realm=\"{}\"", v),
            DigestParam::Nonce(v) => write!(f, "nonce=\"{}\"", v),
            DigestParam::Opaque(v) => write!(f, "opaque=\"{}\"", v),
            DigestParam::Algorithm(v) => write!(f, "algorithm={}", v),
            DigestParam::Domain(v) => write!(f, "domain=\"{}\"", v.join(", ")),
            DigestParam::Stale(v) => write!(f, "stale={}", v),
            DigestParam::Qop(v) => write!(f, "qop={}", v.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(",")),
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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