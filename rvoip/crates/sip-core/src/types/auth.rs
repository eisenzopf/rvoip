use crate::types::uri::Uri;
use std::collections::HashMap;
use std::fmt;
use crate::error::{Result, Error};
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::types::method::Method;

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

/// Generic Authentication Parameter (name=value)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthParam {
    pub name: String,
    pub value: String, // Consider storing raw bytes if unquoting is complex
}

impl fmt::Display for AuthParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}=\"{}\"", self.name, self.value)
    }
}

/// Parameters specific to Digest authentication (used in Challenge and Credentials)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigestParam {
    // Challenge & Credentials
    Realm(String),
    Nonce(String),
    Opaque(String), // Optional in challenge, MUST be returned if present
    Algorithm(Algorithm), // Optional in both
    // Challenge Only
    Domain(Vec<String>), // Optional, quoted list
    Stale(bool),       // Optional
    Qop(Vec<Qop>),     // Optional, quoted list
    // Credentials Only
    Username(String),
    Uri(Uri),
    Response(String), // response-digest (hex)
    Cnonce(String),   // Optional
    MsgQop(Qop),      // Optional, only one value
    NonceCount(u32),  // Optional, nc-value (hex, parsed to u32)
    // Generic fallback
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthenticationInfoParam {
    NextNonce(String),
    Qop(Qop), // Only one value allowed
    ResponseAuth(String), // rspauth (hex)
    Cnonce(String),
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

/// Represents a challenge (WWW-Authenticate, Proxy-Authenticate)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Challenge {
    Digest { params: Vec<DigestParam> },
    Basic { params: Vec<AuthParam> }, // Typically just realm
    Other { scheme: String, params: Vec<AuthParam> },
}

impl fmt::Display for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Challenge::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            },
            Challenge::Basic { params } => {
                 write!(f, "Basic ")?;
                 let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                 write!(f, "{}", params_str)
            },
            Challenge::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}

/// Represents credentials (Authorization, Proxy-Authorization)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Credentials {
    Digest { params: Vec<DigestParam> },
    Basic { token: String }, // Base64 encoded "userid:password"
    Other { scheme: String, params: Vec<AuthParam> },
}

impl fmt::Display for Credentials {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            },
             Credentials::Basic { token } => {
                 write!(f, "Basic {}", token)
            },
            Credentials::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}

/// Typed WWW-Authenticate header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WwwAuthenticate(pub Challenge); // Holds the Challenge enum directly

impl fmt::Display for WwwAuthenticate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Challenge's Display
    }
}

impl WwwAuthenticate {
    /// Creates a new WwwAuthenticate header with mandatory fields.
    pub fn new(scheme: Scheme, realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self(Challenge::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
        ] })
    }

    /// Sets the domain parameter.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Domain(vec![domain.into()]));
        }
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Opaque(opaque.into()));
        }
        self
    }

    /// Sets the stale parameter.
    pub fn with_stale(mut self, stale: bool) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Stale(stale));
        }
        self
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Algorithm(algorithm));
        }
        self
    }

    /// Adds a Qop value.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Qop(vec![qop]));
        }
        self
    }

    /// Sets multiple Qop values.
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        if let Challenge::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Qop(qops));
        }
        self
    }
}

impl FromStr for WwwAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
         crate::parser::headers::parse_www_authenticate(s.as_bytes())
             .map(|(_, challenge)| WwwAuthenticate(challenge))
             .map_err(Error::from) // Convert nom::Err to our Error type
    }
}

/// Typed Authorization header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authorization(pub Credentials); // Holds the Credentials enum directly

impl fmt::Display for Authorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Credentials' Display
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
        Self(Credentials::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
            DigestParam::Uri(uri),
            DigestParam::Response(response.into()),
        ] })
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Algorithm(algorithm));
        }
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Cnonce(cnonce.into()));
        }
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Opaque(opaque.into()));
        }
        self
    }

    /// Sets the message_qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::MsgQop(qop));
        }
        self
    }

    /// Sets the nonce_count parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::NonceCount(nc));
        }
        self
    }
}

impl FromStr for Authorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_authorization(s.as_bytes())
            .map(|(_, auth_header)| auth_header) // parser returns AuthorizationHeader directly
            .map_err(Error::from)
    }
}

/// Typed Proxy-Authenticate header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthenticate(pub Challenge); // Holds Challenge

impl fmt::Display for ProxyAuthenticate {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header.
    pub fn new(challenge: Challenge) -> Self { Self(challenge) }
}

impl FromStr for ProxyAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
         // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_proxy_authenticate(s.as_bytes())
            .map(|(_, challenge)| ProxyAuthenticate(challenge))
            .map_err(Error::from)
    }
}

/// Typed Proxy-Authorization header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthorization(pub Credentials); // Holds Credentials

impl fmt::Display for ProxyAuthorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthorization {
    /// Creates a new ProxyAuthorization header.
    pub fn new(creds: Credentials) -> Self { Self(creds) }
}

impl FromStr for ProxyAuthorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_proxy_authorization(s.as_bytes())
             .map(|(_, creds)| ProxyAuthorization(creds))
             .map_err(Error::from)
    }
}

/// Typed Authentication-Info header.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AuthenticationInfo(pub Vec<AuthenticationInfoParam>); // Holds a list of params

impl fmt::Display for AuthenticationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params_str = self.0.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        write!(f, "{}", params_str)
    }
}

impl AuthenticationInfo {
    /// Creates a new empty AuthenticationInfo header.
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the nextnonce parameter.
    pub fn with_nextnonce(mut self, nextnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::NextNonce(nextnonce.into()));
        self
    }

    /// Sets the qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.push(AuthenticationInfoParam::Qop(qop));
        self
    }

    /// Sets the rspauth parameter.
    pub fn with_rspauth(mut self, rspauth: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::ResponseAuth(rspauth.into()));
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::Cnonce(cnonce.into()));
        self
    }

    /// Sets the nc (nonce count) parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.0.push(AuthenticationInfoParam::NonceCount(nc));
        self
    }
}

impl FromStr for AuthenticationInfo {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
         // Call the actual parser and map nom::Err to crate::error::Error
         crate::parser::headers::parse_authentication_info(s.as_bytes())
            .map(|(_, params)| AuthenticationInfo(params))
            .map_err(Error::from)
    }
}

// TODO: Implement default values, helper methods, and parsing logic for each.
// TODO: Re-implement the `new` and `with_*` methods for the wrapper types
//       to correctly interact with the new enum/vec structures.
//       This requires careful handling of finding/replacing/adding params. 