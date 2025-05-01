//! # SIP WWW-Authenticate Header
//!
//! This module defines the WWW-Authenticate header used in 401 Unauthorized responses.

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::auth::challenge::Challenge;
use crate::types::auth::params::DigestParam;
use crate::types::auth::scheme::{Algorithm, Qop};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed WWW-Authenticate header.
///
/// The WWW-Authenticate header is used in 401 Unauthorized responses to challenge the client
/// to authenticate itself. It can contain multiple challenges using different authentication
/// schemes, allowing the client to choose the most appropriate one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WwwAuthenticate(pub Vec<Challenge>); // Holds multiple Challenge enums

impl fmt::Display for WwwAuthenticate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return Ok(());
        }
        
        let challenges_str = self.0.iter()
            .map(|challenge| challenge.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        
        write!(f, "{}", challenges_str)
    }
}

impl WwwAuthenticate {
    /// Creates a new WwwAuthenticate header with a single Digest challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (e.g., domain name)
    /// - `nonce`: A server-generated unique nonce value
    ///
    /// # Returns
    ///
    /// A new WWW-Authenticate header with a Digest challenge containing the 
    /// specified realm and nonce
    pub fn new(realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self(vec![Challenge::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
        ] }])
    }

    /// Creates a new WwwAuthenticate header with a Basic challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (e.g., domain name)
    ///
    /// # Returns
    ///
    /// A new WWW-Authenticate header with a Basic challenge containing the
    /// specified realm
    pub fn new_basic(realm: impl Into<String>) -> Self {
        Self(vec![Challenge::Basic { params: vec![
            crate::types::auth::params::AuthParam { name: "realm".to_string(), value: realm.into() }
        ] }])
    }

    /// Adds an additional challenge to this header.
    ///
    /// This allows presenting multiple authentication options to the client.
    ///
    /// # Parameters
    ///
    /// - `challenge`: The additional challenge to add
    pub fn add_challenge(&mut self, challenge: Challenge) {
        self.0.push(challenge);
    }

    /// Returns the first Digest challenge, if any.
    ///
    /// # Returns
    ///
    /// An Option containing a reference to the first Digest challenge,
    /// or None if no Digest challenge is present
    pub fn first_digest(&self) -> Option<&Challenge> {
        self.0.iter().find(|c| matches!(c, Challenge::Digest { .. }))
    }

    /// Returns the first Basic challenge, if any.
    ///
    /// # Returns
    ///
    /// An Option containing a reference to the first Basic challenge,
    /// or None if no Basic challenge is present
    pub fn first_basic(&self) -> Option<&Challenge> {
        self.0.iter().find(|c| matches!(c, Challenge::Basic { .. }))
    }

    /// Sets the domain parameter on the first Digest challenge.
    ///
    /// The domain parameter specifies a list of URIs that share the same
    /// authentication information.
    ///
    /// # Parameters
    ///
    /// - `domain`: The domain URI to add
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Domain(vec![domain.into()]));
        }
        self
    }

    /// Sets the opaque parameter on the first Digest challenge.
    ///
    /// The opaque parameter is used by the server to maintain state information,
    /// and clients must return it unchanged in their authorization response.
    ///
    /// # Parameters
    ///
    /// - `opaque`: The opaque string
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Opaque(opaque.into()));
        }
        self
    }

    /// Sets the stale parameter on the first Digest challenge.
    ///
    /// The stale parameter indicates that the nonce has expired but the credentials
    /// (username, password) are still valid. This allows the client to retry with
    /// a new nonce without prompting the user for credentials again.
    ///
    /// # Parameters
    ///
    /// - `stale`: Set to true if the nonce is stale
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_stale(mut self, stale: bool) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Stale(stale));
        }
        self
    }

    /// Sets the algorithm parameter on the first Digest challenge.
    ///
    /// The algorithm parameter specifies which hash algorithm to use for the digest.
    ///
    /// # Parameters
    ///
    /// - `algorithm`: The hash algorithm to use
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Algorithm(algorithm));
        }
        self
    }

    /// Adds a Qop value to the first Digest challenge.
    ///
    /// The qop (quality of protection) parameter specifies what type of
    /// protection is required for the authentication.
    ///
    /// # Parameters
    ///
    /// - `qop`: The quality of protection to add
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Qop(vec![qop]));
        }
        self
    }

    /// Sets multiple Qop values on the first Digest challenge.
    ///
    /// This allows the server to offer multiple quality of protection options
    /// to the client, which can choose the most appropriate one.
    ///
    /// # Parameters
    ///
    /// - `qops`: A vector of quality of protection options
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Qop(qops));
        }
        self
    }
}

impl FromStr for WwwAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
         crate::parser::headers::parse_www_authenticate(s.as_bytes())
             .map(|(_, challenges)| WwwAuthenticate(challenges))
             .map_err(Error::from) // Convert nom::Err to our Error type
    }
}

impl TypedHeaderTrait for WwwAuthenticate {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::WwwAuthenticate
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
                    WwwAuthenticate::from_str(s.trim())
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