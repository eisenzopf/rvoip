//! # SIP Authentication Credentials
//!
//! This module defines the Credentials type used in Authorization and ProxyAuthorization headers.

use crate::types::auth::params::{AuthParam, DigestParam};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Represents credentials (Authorization, Proxy-Authorization)
///
/// Credentials are sent by clients in response to authentication challenges. They
/// contain the information needed for the server to authenticate the client.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub enum Credentials {
    /// Digest authentication credentials with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication credentials (Base64 encoded "username:password")
    Basic { token: String }, // Base64 encoded "userid:password"
    /// Bearer token authentication (RFC 8898)
    Bearer { token: String },
    /// Other authentication scheme credentials
    Other {
        scheme: String,
        params: Vec<AuthParam>,
    },
}

impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Digest { params } => formatter
                .debug_struct("Credentials::Digest")
                .field("param_count", &params.len())
                .finish(),
            Self::Basic { .. } => formatter.write_str("Credentials::Basic([redacted])"),
            Self::Bearer { .. } => formatter.write_str("Credentials::Bearer([redacted])"),
            Self::Other { params, .. } => formatter
                .debug_struct("Credentials::Other")
                .field("scheme", &"[redacted]")
                .field("param_count", &params.len())
                .finish(),
        }
    }
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

    /// Returns true if the credentials are of the Bearer type
    ///
    /// # Returns
    ///
    /// `true` if these are Bearer credentials, `false` otherwise
    pub fn is_bearer(&self) -> bool {
        matches!(self, Credentials::Bearer { .. })
    }

    /// Creates new Bearer credentials with the given token
    ///
    /// # Parameters
    ///
    /// - `token`: The Bearer token string
    ///
    /// # Returns
    ///
    /// Bearer credentials with the specified token
    pub fn bearer(token: impl Into<String>) -> Self {
        Credentials::Bearer {
            token: token.into(),
        }
    }
}

impl fmt::Display for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}", params_str)
            }
            Credentials::Basic { token } => {
                write!(f, "Basic {}", token)
            }
            Credentials::Bearer { token } => {
                write!(f, "Bearer {}", token)
            }
            Credentials::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Credentials;
    use crate::types::auth::{Authorization, ProxyAuthorization};

    #[test]
    fn auth_debug_is_redacted_while_display_remains_the_wire_value() {
        let credentials = Credentials::bearer("direct-wire-secret");
        assert_eq!(credentials.to_string(), "Bearer direct-wire-secret");
        assert!(!format!("{credentials:?}").contains("direct-wire-secret"));

        let authorization = Authorization(credentials.clone());
        assert_eq!(authorization.to_string(), "Bearer direct-wire-secret");
        assert!(!format!("{authorization:?}").contains("direct-wire-secret"));

        let proxy_authorization = ProxyAuthorization(credentials);
        assert_eq!(proxy_authorization.to_string(), "Bearer direct-wire-secret");
        assert!(!format!("{proxy_authorization:?}").contains("direct-wire-secret"));
    }
}
