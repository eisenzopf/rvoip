//! # SIP Authentication Schemes
//!
//! This module defines the authentication schemes, algorithms, and quality of protection
//! options used in SIP authentication.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};

/// Authentication Scheme (Digest, Basic, etc.)
///
/// SIP authentication can use different schemes, with Digest being the recommended
/// approach in RFC 3261. Basic authentication is less secure and generally not recommended
/// for production use.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthScheme {
    /// Digest authentication (RFC 3261 and RFC 7616)
    Digest,
    /// Basic authentication (username:password encoded in Base64)
    Basic, // Less common in SIP, but possible
    /// Other authentication schemes
    Other(String),
}

impl fmt::Display for AuthScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthScheme::Digest => write!(f, "Digest"),
            AuthScheme::Basic => write!(f, "Basic"),
            AuthScheme::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for AuthScheme {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "digest" => Ok(AuthScheme::Digest),
            "basic" => Ok(AuthScheme::Basic),
            _ if !s.is_empty() => Ok(AuthScheme::Other(s.to_string())),
            _ => Err(Error::InvalidInput("Empty scheme name".to_string())),
        }
    }
}

/// Digest Algorithm (MD5, SHA-256, etc.)
///
/// Specifies the algorithm used for calculating digest hashes in SIP authentication.
/// MD5 is the original algorithm from RFC 2617, but newer algorithms like SHA-256
/// are recommended for better security as defined in RFC 7616.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Algorithm {
    /// MD5 algorithm (RFC 2617)
    Md5,
    /// MD5 with session data (RFC 2617)
    Md5Sess,
    /// SHA-256 algorithm (RFC 7616)
    Sha256,
    /// SHA-256 with session data (RFC 7616)
    Sha256Sess,
    /// SHA-512 algorithm (RFC 5687)
    Sha512,
    /// SHA-512 with session data
    Sha512Sess,
    /// Other algorithms
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
            _ => Err(Error::InvalidInput("Empty algorithm name".to_string())),
        }
    }
}

/// Quality of Protection (auth, auth-int)
///
/// Specifies the quality of protection for HTTP Digest Authentication as defined
/// in RFC 7616. The `auth` quality means authentication only, while `auth-int`
/// also provides integrity protection for the message body.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Qop {
    /// Authentication only
    Auth,
    /// Authentication with message integrity protection
    AuthInt,
    /// Other QOP values
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
            _ => Err(Error::InvalidInput("Empty qop value".to_string())),
        }
    }
} 