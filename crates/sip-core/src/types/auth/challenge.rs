//! # SIP Authentication Challenge
//!
//! This module defines the Challenge type used in WWW-Authenticate and Proxy-Authenticate headers.

use std::fmt;
use serde::{Deserialize, Serialize};
use crate::types::auth::params::{AuthParam, DigestParam};

/// Represents a challenge (WWW-Authenticate, Proxy-Authenticate)
///
/// A challenge is sent by a server in 401 Unauthorized or 407 Proxy Authentication Required
/// responses to request authentication from a client. Challenges can use different
/// authentication schemes, with Digest being the most common in SIP.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Challenge {
    /// Digest authentication challenge with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication challenge (typically just realm)
    Basic { params: Vec<AuthParam> }, // Typically just realm
    /// Bearer authentication challenge (RFC 8898)
    Bearer { 
        /// The authentication realm
        realm: String,
        /// Optional scope requirement
        scope: Option<String>,
        /// Optional error code
        error: Option<String>,
        /// Optional error description
        error_description: Option<String>,
    },
    /// Other authentication scheme challenges
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
            Challenge::Bearer { realm, scope, error, error_description } => {
                write!(f, "Bearer realm=\"{}\"", realm)?;
                if let Some(scope) = scope {
                    write!(f, ", scope=\"{}\"", scope)?;
                }
                if let Some(error) = error {
                    write!(f, ", error=\"{}\"", error)?;
                }
                if let Some(error_desc) = error_description {
                    write!(f, ", error_description=\"{}\"", error_desc)?;
                }
                Ok(())
            },
            Challenge::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
} 