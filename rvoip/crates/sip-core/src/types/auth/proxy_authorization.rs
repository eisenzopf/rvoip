//! # SIP Proxy-Authorization Header
//!
//! This module defines the Proxy-Authorization header used by clients to authenticate to proxy servers.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::auth::credentials::Credentials;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed Proxy-Authorization header.
///
/// The Proxy-Authorization header is used by clients to provide authentication credentials
/// to a proxy server in response to a Proxy-Authenticate challenge. It is similar to the
/// Authorization header but scoped to proxy authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthorization(pub Credentials); // Holds Credentials

impl fmt::Display for ProxyAuthorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthorization {
    /// Creates a new ProxyAuthorization header.
    ///
    /// # Parameters
    ///
    /// - `creds`: The authentication credentials
    ///
    /// # Returns
    ///
    /// A new ProxyAuthorization header with the specified credentials
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

impl TypedHeaderTrait for ProxyAuthorization {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ProxyAuthorization
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::ProxyAuthorization(self.clone()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::ProxyAuthorization(proxy_auth) => {
                Ok(proxy_auth.clone())
            },
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    ProxyAuthorization::from_str(s.trim())
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
} 