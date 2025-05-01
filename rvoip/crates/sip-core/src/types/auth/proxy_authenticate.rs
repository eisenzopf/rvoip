//! # SIP Proxy-Authenticate Header
//!
//! This module defines the Proxy-Authenticate header used in 407 Proxy Authentication Required responses.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::auth::challenge::Challenge;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed Proxy-Authenticate header.
///
/// The Proxy-Authenticate header is used by proxy servers in 407 Proxy Authentication Required
/// responses to challenge the client to authenticate itself to the proxy. It is similar to the
/// WWW-Authenticate header but scoped to proxy authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthenticate(pub Challenge); // Holds Challenge

impl fmt::Display for ProxyAuthenticate {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header.
    ///
    /// # Parameters
    ///
    /// - `challenge`: The authentication challenge
    ///
    /// # Returns
    ///
    /// A new ProxyAuthenticate header with the specified challenge
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

impl TypedHeaderTrait for ProxyAuthenticate {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ProxyAuthenticate
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    ProxyAuthenticate::from_str(s.trim())
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