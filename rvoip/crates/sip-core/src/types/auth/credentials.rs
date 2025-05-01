//! # SIP Authentication Credentials
//!
//! This module defines the Credentials type used in Authorization and ProxyAuthorization headers.

use std::fmt;
use serde::{Deserialize, Serialize};
use crate::types::auth::params::{AuthParam, DigestParam};

/// Represents credentials (Authorization, Proxy-Authorization)
///
/// Credentials are sent by clients in response to authentication challenges. They
/// contain the information needed for the server to authenticate the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Credentials {
    /// Digest authentication credentials with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication credentials (Base64 encoded "username:password")
    Basic { token: String }, // Base64 encoded "userid:password"
    /// Other authentication scheme credentials
    Other { scheme: String, params: Vec<AuthParam> },
}

impl Credentials {
    /// Returns true if the credentials are of the Digest type
    ///
    /// # Returns
    ///
    /// `true` if these are Digest credentials, `false` otherwise
    pub fn is_digest(&self) -> bool {
        matches!(self, Credentials::Digest { .. })
    }
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