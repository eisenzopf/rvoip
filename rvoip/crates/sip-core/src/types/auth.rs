use crate::uri::Uri;
use std::collections::HashMap;
use std::fmt;
use crate::parser::headers::{parse_www_authenticate, parse_authorization, parse_proxy_authenticate, parse_proxy_authorization, parse_authentication_info}; // Parsers
use crate::error::Result;
use std::str::FromStr;
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq)] // Eq might be tricky with floats/future extensions
pub struct WwwAuthenticate {
    pub scheme: Scheme,
    pub realm: String,
    pub domain: Option<String>,
    pub nonce: String,
    pub opaque: Option<String>,
    pub stale: Option<bool>, // Changed to bool based on RFC 2617
    pub algorithm: Option<Algorithm>,
    pub qop: Vec<Qop>, // Can be a list
    // Add other potential fields like charset, userhash
}

impl fmt::Display for WwwAuthenticate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} realm=\"\"{}\"\"", self.scheme, self.realm)?; // Realm is mandatory
        write!(f, ", nonce=\"\"{}\"\"", self.nonce)?; // Nonce is mandatory

        if let Some(domain) = &self.domain {
            write!(f, ", domain=\"\"{}\"\"", domain)?; // Domain should be quoted
        }
        if let Some(opaque) = &self.opaque {
            write!(f, ", opaque=\"\"{}\"\"", opaque)?; 
        }
        if let Some(stale) = self.stale {
            write!(f, ", stale={}", if stale { "true" } else { "false" })?;
        }
        if let Some(algo) = &self.algorithm {
            write!(f, ", algorithm={}", algo)?; // Algorithm might not need quotes per examples
        }
        if !self.qop.is_empty() {
            let qop_str = self.qop.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(",");
            write!(f, ", qop=\"\"{}\"\"", qop_str)?; // qop value MUST be quoted
        }
        // TODO: Add display for other fields (charset, userhash etc.)
        Ok(())
    }
}

impl WwwAuthenticate {
    /// Creates a new WwwAuthenticate header with mandatory fields.
    pub fn new(scheme: Scheme, realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self {
            scheme,
            realm: realm.into(),
            nonce: nonce.into(),
            domain: None,
            opaque: None,
            stale: None,
            algorithm: None,
            qop: Vec::new(),
        }
    }

    /// Sets the domain parameter.
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        self.opaque = Some(opaque.into());
        self
    }

    /// Sets the stale parameter.
    pub fn with_stale(mut self, stale: bool) -> Self {
        self.stale = Some(stale);
        self
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = Some(algorithm);
        self
    }

    /// Adds a Qop value.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if !self.qop.contains(&qop) { // Avoid duplicates
             self.qop.push(qop);
        }
        self
    }

    /// Sets multiple Qop values.
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        self.qop = qops;
        self
    }

    // TODO: Add with_charset, with_userhash if needed
}

impl FromStr for WwwAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_www_authenticate(s) }
}

/// Typed Authorization header.
#[derive(Debug, Clone, PartialEq)]
pub struct Authorization {
    pub scheme: Scheme,
    pub username: String,
    pub realm: String,
    pub nonce: String,
    pub uri: Uri,
    pub response: String,
    pub algorithm: Option<Algorithm>,
    pub cnonce: Option<String>,
    pub opaque: Option<String>,
    pub message_qop: Option<Qop>,
    pub nonce_count: Option<u32>,
    // Add other fields
}

impl fmt::Display for Authorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} username=\"\"{}\"\"", self.scheme, self.username)?;
        write!(f, ", realm=\"\"{}\"\"", self.realm)?; 
        write!(f, ", nonce=\"\"{}\"\"", self.nonce)?; 
        write!(f, ", uri=\"\"{}\"\"", self.uri)?; // URI should be quoted
        write!(f, ", response=\"\"{}\"\"", self.response)?; // Response must be quoted

        if let Some(algo) = &self.algorithm {
            write!(f, ", algorithm={}", algo)?;
        }
        if let Some(cnonce) = &self.cnonce {
            write!(f, ", cnonce=\"\"{}\"\"", cnonce)?; 
        }
        if let Some(opaque) = &self.opaque {
            write!(f, ", opaque=\"\"{}\"\"", opaque)?;
        }
        if let Some(qop) = &self.message_qop {
            write!(f, ", qop={}", qop)?; // qop value is not quoted here
        }
        if let Some(nc) = self.nonce_count {
            // Nonce count MUST be 8 hex digits
            write!(f, ", nc={:08x}", nc)?;
        }
        Ok(())
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
        Self {
            scheme,
            username: username.into(),
            realm: realm.into(),
            nonce: nonce.into(),
            uri,
            response: response.into(),
            algorithm: None,
            cnonce: None,
            opaque: None,
            message_qop: None,
            nonce_count: None,
        }
    }

    /// Sets the algorithm parameter.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.algorithm = Some(algorithm);
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.cnonce = Some(cnonce.into());
        self
    }

    /// Sets the opaque parameter.
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        self.opaque = Some(opaque.into());
        self
    }

    /// Sets the message_qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.message_qop = Some(qop);
        self
    }

    /// Sets the nonce_count parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.nonce_count = Some(nc);
        self
    }
}

impl FromStr for Authorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_authorization(s) }
}

/// Typed Proxy-Authenticate header.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyAuthenticate(pub WwwAuthenticate);

impl fmt::Display for ProxyAuthenticate {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to WwwAuthenticate display
    }
}

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header.
    pub fn new(auth: WwwAuthenticate) -> Self { Self(auth) }
}

impl FromStr for ProxyAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_proxy_authenticate(s) }
}

/// Typed Proxy-Authorization header.
#[derive(Debug, Clone, PartialEq)]
pub struct ProxyAuthorization(pub Authorization);

impl fmt::Display for ProxyAuthorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Authorization display
    }
}

impl ProxyAuthorization {
    /// Creates a new ProxyAuthorization header.
    pub fn new(auth: Authorization) -> Self { Self(auth) }
}

impl FromStr for ProxyAuthorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_proxy_authorization(s) }
}

/// Typed Authentication-Info header.
#[derive(Debug, Clone, PartialEq, Default)] // Add Default
pub struct AuthenticationInfo {
    pub nextnonce: Option<String>,
    pub qop: Option<Qop>,
    pub rspauth: Option<String>,
    pub cnonce: Option<String>,
    pub nc: Option<u32>,
}

impl fmt::Display for AuthenticationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if let Some(nextnonce) = &self.nextnonce {
            parts.push(format!("nextnonce=\"\"{}\"\"", nextnonce));
        }
        if let Some(qop) = &self.qop {
             parts.push(format!("qop={}", qop)); // qop value is not quoted here
        }
         if let Some(rspauth) = &self.rspauth {
            parts.push(format!("rspauth=\"\"{}\"\"", rspauth));
        }
        if let Some(cnonce) = &self.cnonce {
            parts.push(format!("cnonce=\"\"{}\"\"", cnonce));
        }
        if let Some(nc) = self.nc {
            // Nonce count MUST be 8 octal digits according to RFC 7615
            parts.push(format!("nc={:08o}", nc)); 
        }
        write!(f, "{}", parts.join(", "))
    }
}

impl AuthenticationInfo {
    /// Creates a new empty AuthenticationInfo header.
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the nextnonce parameter.
    pub fn with_nextnonce(mut self, nextnonce: impl Into<String>) -> Self {
        self.nextnonce = Some(nextnonce.into());
        self
    }

    /// Sets the qop parameter.
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.qop = Some(qop);
        self
    }

    /// Sets the rspauth parameter.
    pub fn with_rspauth(mut self, rspauth: impl Into<String>) -> Self {
        self.rspauth = Some(rspauth.into());
        self
    }

    /// Sets the cnonce parameter.
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.cnonce = Some(cnonce.into());
        self
    }

    /// Sets the nc (nonce count) parameter.
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.nc = Some(nc);
        self
    }
}

impl FromStr for AuthenticationInfo {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_authentication_info(s) }
}

// TODO: Implement default values, helper methods, and parsing logic for each. 