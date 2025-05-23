//! # SIP Authorization Header
//!
//! This module defines the Authorization header used by clients to provide authentication credentials.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::uri::Uri;
use crate::types::auth::credentials::Credentials;
use crate::types::auth::params::DigestParam;
use crate::types::auth::scheme::{AuthScheme, Algorithm, Qop};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed Authorization header.
///
/// The Authorization header is used by clients to provide authentication credentials
/// in response to a WWW-Authenticate challenge. It typically contains the necessary
/// information for the server to verify the client's identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authorization(pub Credentials); // Holds the Credentials enum directly

impl fmt::Display for Authorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Credentials' Display
    }
}

impl Authorization {
    /// Creates a new Authorization header with mandatory fields.
    ///
    /// # Parameters
    ///
    /// - `scheme`: The authentication scheme to use
    /// - `username`: The username for authentication
    /// - `realm`: The authentication realm (must match the challenge)
    /// - `nonce`: The nonce from the challenge
    /// - `uri`: The request URI
    /// - `response`: The computed digest response
    ///
    /// # Returns
    ///
    /// A new Authorization header with the specified credentials
    pub fn new(
        scheme: AuthScheme,
        username: impl Into<String>,
        realm: impl Into<String>,
        nonce: impl Into<String>,
        uri: Uri,
        response: impl Into<String>
    ) -> Self {
        Self(Credentials::Digest { params: vec![
            DigestParam::Username(username.into()),
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
            DigestParam::Uri(uri),
            DigestParam::Response(response.into()),
        ] })
    }

    /// Sets the algorithm parameter.
    ///
    /// # Parameters
    ///
    /// - `algorithm`: The hash algorithm used
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Algorithm(algorithm));
        }
        self
    }

    /// Sets the cnonce parameter.
    ///
    /// The cnonce (client nonce) is a nonce generated by the client and is required
    /// when using quality of protection (qop).
    ///
    /// # Parameters
    ///
    /// - `cnonce`: The client-generated nonce
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Cnonce(cnonce.into()));
        }
        self
    }

    /// Sets the opaque parameter.
    ///
    /// The opaque parameter must be returned unchanged from the challenge.
    ///
    /// # Parameters
    ///
    /// - `opaque`: The opaque string from the challenge
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Opaque(opaque.into()));
        }
        self
    }

    /// Sets the message_qop parameter.
    ///
    /// This specifies which quality of protection the client has selected
    /// from those offered by the server.
    ///
    /// # Parameters
    ///
    /// - `qop`: The quality of protection used
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::MsgQop(qop));
        }
        self
    }

    /// Sets the nonce_count parameter.
    ///
    /// The nonce count is incremented by the client each time it reuses the same
    /// nonce in a new request, and is required when using quality of protection (qop).
    ///
    /// # Parameters
    ///
    /// - `nc`: The nonce count value
    ///
    /// # Returns
    ///
    /// The modified Authorization header
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

impl TypedHeaderTrait for Authorization {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Authorization
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Authorization(self.clone()))
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
                    Authorization::from_str(s.trim())
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