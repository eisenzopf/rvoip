use crate::types::uri::Uri;
use std::collections::HashMap;
use std::fmt;
use crate::parser::headers::{parse_www_authenticate, parse_authorization, parse_proxy_authenticate, parse_proxy_authorization, parse_authentication_info}; // Parsers
use crate::error::{Result, Error};
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::parser::headers::auth::{
    AuthenticationInfoValue, AuthorizationValue, ProxyAuthenticateValue, ProxyAuthorizationValue,
    WwwAuthenticateValue,
};
use crate::{DigestChallenge, Method};

/// Authentication Scheme (Digest, Basic, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    Digest,
    Basic, // Less common in SIP, but possible
    Other(String),
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scheme::Digest => write!(f, "Digest"),
            Scheme::Basic => write!(f, "Basic"),
            Scheme::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for Scheme {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "digest" => Ok(Scheme::Digest),
            "basic" => Ok(Scheme::Basic),
            _ if !s.is_empty() => Ok(Scheme::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty scheme name".to_string())),
        }
    }
}

/// Digest Algorithm (MD5, SHA-256, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Algorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
    Sha512, // RFC 5687
    Sha512Sess,
    Other(String),
}

impl fmt::Display for Algorithm {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Algorithm::Md5 => write!(f, "MD5"),
            Algorithm::Md5Sess => write!(f, "MD5-sess"),
            Algorithm::Sha256 => write!(f, "SHA-256"),
            Algorithm::Sha256Sess => write!(f, "SHA-256-sess"),
            Algorithm::Sha512 => write!(f, "SHA-512-256"), // Note: RFC 7616 uses SHA-512-256
            Algorithm::Sha512Sess => write!(f, "SHA-512-256-sess"),
            Algorithm::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for Algorithm {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "md5" => Ok(Algorithm::Md5),
            "md5-sess" => Ok(Algorithm::Md5Sess),
            "sha-256" => Ok(Algorithm::Sha256),
            "sha-256-sess" => Ok(Algorithm::Sha256Sess),
            "sha-512-256" => Ok(Algorithm::Sha512),
            "sha-512-256-sess" => Ok(Algorithm::Sha512Sess),
            _ if !s.is_empty() => Ok(Algorithm::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty algorithm name".to_string())),
        }
    }
}

/// Quality of Protection (auth, auth-int)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Qop {
    Auth,
    AuthInt,
    Other(String),
}

impl fmt::Display for Qop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Qop::Auth => write!(f, "auth"),
            Qop::AuthInt => write!(f, "auth-int"),
            Qop::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for Qop {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "auth" => Ok(Qop::Auth),
            "auth-int" => Ok(Qop::AuthInt),
            _ if !s.is_empty() => Ok(Qop::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty qop value".to_string())),
        }
    }
}

/// Typed WWW-Authenticate header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WwwAuthenticate(pub WwwAuthenticateValue);

impl fmt::Display for WwwAuthenticate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl WwwAuthenticate {
    /// Creates a new WwwAuthenticate header with mandatory fields.
    pub fn new(scheme: Scheme, realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self(WwwAuthenticateValue::new(scheme, realm, nonce))
    }

    /// Sets the domain parameter.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.0.with_domain(domain);
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        self.0.with_opaque(opaque);
        self
    }

    /// Sets the stale parameter.
    pub fn with_stale(mut self, stale: bool) -> Self {
        self.0.with_stale(stale);
        self
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.0.with_algorithm(algorithm);
        self
    }

    /// Adds a Qop value.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.with_qop(qop);
        self
    }

    /// Sets multiple Qop values.
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        self.0.with_qops(qops);
        self
    }
}

impl FromStr for WwwAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_www_authenticate(s.as_bytes()).map(|(_,v)| WwwAuthenticate(v)) }
}

/// Typed Authorization header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authorization(pub AuthorizationValue);

impl fmt::Display for Authorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Authorization {
    /// Creates a new Authorization header with mandatory fields.
    pub fn new(
        scheme: Scheme,
        username: impl Into<String>,
        realm: impl Into<String>,
        nonce: impl Into<String>,
        uri: Uri,
        response: impl Into<String>
    ) -> Self {
        Self(AuthorizationValue::new(scheme, username, realm, nonce, uri, response))
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.0.with_algorithm(algorithm);
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.0.with_cnonce(cnonce);
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        self.0.with_opaque(opaque);
        self
    }

    /// Sets the message_qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.with_qop(qop);
        self
    }

    /// Sets the nonce_count parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.0.with_nonce_count(nc);
        self
    }
}

impl FromStr for Authorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        parse_authorization(s.as_bytes())
            .map_err(Error::from)
            .and_then(|(_, v)| { 
                if v.len() < 6 {
                    return Err(Error::ParseError("Missing required fields for Authorization".to_string()));
                }
                Ok(Authorization(v))
            })
    }
}

/// Typed Proxy-Authenticate header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthenticate(pub ProxyAuthenticateValue);

impl fmt::Display for ProxyAuthenticate {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header.
    pub fn new(auth: WwwAuthenticate) -> Self { Self(auth.0) }
}

impl FromStr for ProxyAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_proxy_authenticate(s.as_bytes()).map(|(_,v)| ProxyAuthenticate(v)) }
}

/// Typed Proxy-Authorization header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthorization(pub ProxyAuthorizationValue);

impl fmt::Display for ProxyAuthorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthorization {
    /// Creates a new ProxyAuthorization header.
    pub fn new(auth: Authorization) -> Self { Self(auth.0) }
}

impl FromStr for ProxyAuthorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_proxy_authorization(s.as_bytes()).map(|(_,v)| ProxyAuthorization(v)) }
}

/// Typed Authentication-Info header.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AuthenticationInfo(pub AuthenticationInfoValue);

impl fmt::Display for AuthenticationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AuthenticationInfo {
    /// Creates a new empty AuthenticationInfo header.
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the nextnonce parameter.
    pub fn with_nextnonce(mut self, nextnonce: impl Into<String>) -> Self {
        self.0.with_nextnonce(nextnonce);
        self
    }

    /// Sets the qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.with_qop(qop);
        self
    }

    /// Sets the rspauth parameter.
    pub fn with_rspauth(mut self, rspauth: impl Into<String>) -> Self {
        self.0.with_rspauth(rspauth);
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.0.with_cnonce(cnonce);
        self
    }

    /// Sets the nc (nonce count) parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.0.with_nonce_count(nc);
        self
    }
}

impl FromStr for AuthenticationInfo {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        parse_authentication_info(s.as_bytes())
            .map_err(Error::from)
            .and_then(|(_, v)| {
                if v.len() < 5 {
                    return Err(Error::ParseError("Incorrect field count for AuthenticationInfo".to_string()));
                }
                Ok(AuthenticationInfo(v))
            })
    }
}

// TODO: Implement default values, helper methods, and parsing logic for each. 