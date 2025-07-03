//! # SIP Authentication-Info Header
//!
//! This module defines the Authentication-Info header used in responses after successful authentication.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::auth::params::AuthenticationInfoParam;
use crate::types::auth::scheme::Qop;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed Authentication-Info header.
///
/// The Authentication-Info header is used in responses from a server after successful
/// authentication. It provides additional authentication information to the client, such
/// as a new nonce for subsequent requests or a server authentication response for mutual
/// authentication.
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
    ///
    /// # Returns
    ///
    /// A new empty AuthenticationInfo header
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the nextnonce parameter.
    ///
    /// The next nonce to be used for authentication, provided by the server to
    /// allow the client to authenticate in future requests without waiting for
    /// an authorization failure.
    ///
    /// # Parameters
    ///
    /// - `nextnonce`: The next nonce value to use
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    pub fn with_nextnonce(mut self, nextnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::NextNonce(nextnonce.into()));
        self
    }

    /// Sets the qop parameter.
    ///
    /// The quality of protection that was applied to the previous request.
    ///
    /// # Parameters
    ///
    /// - `qop`: The quality of protection used
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.push(AuthenticationInfoParam::Qop(qop));
        self
    }

    /// Sets the rspauth parameter.
    ///
    /// The rspauth (response authentication) parameter is used for mutual authentication,
    /// allowing the client to authenticate the server.
    ///
    /// # Parameters
    ///
    /// - `rspauth`: The server's authentication response
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    pub fn with_rspauth(mut self, rspauth: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::ResponseAuth(rspauth.into()));
        self
    }

    /// Sets the cnonce parameter.
    ///
    /// The cnonce (client nonce) echoed from the client's request.
    ///
    /// # Parameters
    ///
    /// - `cnonce`: The client nonce value
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::Cnonce(cnonce.into()));
        self
    }

    /// Sets the nc (nonce count) parameter.
    ///
    /// The nonce count echoed from the client's request.
    ///
    /// # Parameters
    ///
    /// - `nc`: The nonce count value
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
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

impl TypedHeaderTrait for AuthenticationInfo {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::AuthenticationInfo
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::AuthenticationInfo(self.clone()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::AuthenticationInfo(auth_info) => {
                Ok(auth_info.clone())
            },
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    AuthenticationInfo::from_str(s.trim())
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