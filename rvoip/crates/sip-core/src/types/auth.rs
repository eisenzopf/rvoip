use crate::uri::Uri;
use std::collections::HashMap;
use std::fmt;
use crate::parser::headers::{parse_www_authenticate, parse_authorization, parse_proxy_authenticate, parse_proxy_authorization, parse_authentication_info}; // Parsers
use crate::error::Result;
use std::str::FromStr;

/// Authentication Scheme (Digest, Basic, etc.)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

/// Digest Algorithm (MD5, SHA-256, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// Quality of Protection (auth, auth-int)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

impl FromStr for ProxyAuthorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_proxy_authorization(s) }
}

/// Typed Authentication-Info header.
#[derive(Debug, Clone, PartialEq)]
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

impl FromStr for AuthenticationInfo {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> { parse_authentication_info(s) }
}

// TODO: Implement default values, helper methods, and parsing logic for each. 