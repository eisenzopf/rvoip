//! # SIP Proxy-Authenticate Header
//!
//! This module defines the Proxy-Authenticate header used in 407 Proxy Authentication Required responses.
//!
//! ## Overview
//!
//! The Proxy-Authenticate header is used by SIP proxy servers to challenge clients to authenticate 
//! themselves. It appears in 407 Proxy Authentication Required responses and contains one or more 
//! authentication challenges that the client must satisfy to gain access to the requested resource 
//! through the proxy.
//!
//! ## Authentication Process
//!
//! 1. Client sends a request to a proxy server
//! 2. Proxy server responds with a 407 Proxy Authentication Required response containing a 
//!    Proxy-Authenticate header
//! 3. Client uses the information in the Proxy-Authenticate header to construct a valid 
//!    Proxy-Authorization header
//! 4. Client resends the request with the Proxy-Authorization header
//! 5. Proxy server validates the credentials and processes the request if authentication succeeds
//!
//! ## Security Considerations
//!
//! - Proxy authentication is separate from endpoint authentication (which uses WWW-Authenticate)
//! - Multiple proxies in a chain can each require their own authentication
//! - Credentials are typically not encrypted unless TLS is used for the entire SIP connection
//! - Nonces should be carefully generated and validated to prevent replay attacks
//! - Using the qop (Quality of Protection) parameter with 'auth-int' provides message integrity
//!
//! ## Related Headers
//!
//! - [Proxy-Authorization](../authorization/struct.ProxyAuthorization.html): Used by clients to provide
//!   authentication credentials in response to a Proxy-Authenticate challenge
//! - [WWW-Authenticate](../www_authenticate/struct.WwwAuthenticate.html): Similar header used by 
//!   endpoints (not proxies) for user authentication
//! - [Authorization](../authorization/struct.Authorization.html): Used for endpoint authentication
//!
//! ## RFC References
//!
//! - [RFC 3261 Section 20.27](https://datatracker.ietf.org/doc/html/rfc3261#section-20.27)
//! - [RFC 2617](https://datatracker.ietf.org/doc/html/rfc2617) (HTTP Authentication)
//! - [RFC 7616](https://datatracker.ietf.org/doc/html/rfc7616) (HTTP Digest Authentication)

use std::fmt;
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::error::{Result, Error};
use crate::types::auth::challenge::Challenge;
use crate::types::auth::params::DigestParam;
use crate::types::auth::scheme::{Algorithm, Qop};
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Typed Proxy-Authenticate header.
///
/// The Proxy-Authenticate header is used by proxy servers in 407 Proxy Authentication Required
/// responses to challenge the client to authenticate itself to the proxy. It is similar to the
/// WWW-Authenticate header but scoped to proxy authentication.
///
/// A single Proxy-Authenticate header can contain multiple challenges using different authentication
/// schemes, allowing the client to choose the most appropriate one.
///
/// # Example
///
/// ```
/// use rvoip_sip_core::ProxyAuthenticate;
/// use rvoip_sip_core::{Algorithm, Qop};
///
/// // Create a basic digest authentication challenge
/// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
///     .with_algorithm(Algorithm::Md5)
///     .with_qop(Qop::Auth);
///
/// // Verify the challenge parameters
/// assert!(challenge.0.len() == 1);
/// if let rvoip_sip_core::Challenge::Digest { params } = &challenge.0[0] {
///     assert!(params.contains(&rvoip_sip_core::DigestParam::Realm("proxy.example.com".to_string())));
///     assert!(params.contains(&rvoip_sip_core::DigestParam::Nonce("abc123xyz789".to_string())));
///     assert!(params.contains(&rvoip_sip_core::DigestParam::Algorithm(Algorithm::Md5)));
///     assert!(params.contains(&rvoip_sip_core::DigestParam::Qop(vec![Qop::Auth])));
/// } else {
///     panic!("Expected Digest challenge");
/// }
///
/// // The header will be rendered as:
/// // Proxy-Authenticate: Digest realm="proxy.example.com", nonce="abc123xyz789", algorithm=MD5, qop="auth"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthenticate(pub Vec<Challenge>); // Holds multiple Challenge enums

impl fmt::Display for ProxyAuthenticate {
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

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header with a single Digest challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (typically domain name of the proxy)
    /// - `nonce`: A server-generated unique nonce value
    ///
    /// # Returns
    ///
    /// A new ProxyAuthenticate header with a Digest challenge containing the 
    /// specified realm and nonce
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789");
    /// ```
    pub fn new(realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self(vec![Challenge::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
        ] }])
    }

    /// Creates a new ProxyAuthenticate header with a Basic challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (typically domain name of the proxy)
    ///
    /// # Returns
    ///
    /// A new ProxyAuthenticate header with a Basic challenge containing the
    /// specified realm
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    ///
    /// let challenge = ProxyAuthenticate::new_basic("proxy.example.com");
    /// ```
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
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    /// use rvoip_sip_core::Challenge;
    /// use rvoip_sip_core::DigestParam;
    ///
    /// let mut challenge = ProxyAuthenticate::new("proxy.example.com", "nonce1");
    /// 
    /// // Add a second challenge with a different nonce
    /// challenge.add_challenge(Challenge::Digest { params: vec![
    ///     DigestParam::Realm("proxy.example.com".to_string()),
    ///     DigestParam::Nonce("nonce2".to_string())
    /// ]});
    /// ```
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
    /// authentication information. This allows the client to reuse the 
    /// same credentials for multiple requests within the specified domain.
    ///
    /// # Parameters
    ///
    /// - `domain`: The domain URI to add
    ///
    /// # Returns
    ///
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
    ///     .with_domain("sip:*.example.com");
    /// ```
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
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
    ///     .with_opaque("8da1f33efc1b0d813006ef1a396ff276");
    /// ```
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
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    ///
    /// // Create a challenge indicating the nonce is stale
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "new_nonce_xyz789")
    ///     .with_stale(true);
    /// ```
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
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    /// use rvoip_sip_core::Algorithm;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
    ///     .with_algorithm(Algorithm::Sha256);
    /// ```
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
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    /// use rvoip_sip_core::Qop;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
    ///     .with_qop(Qop::Auth);
    /// ```
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
    /// The modified ProxyAuthenticate header
    ///
    /// # Example
    ///
    /// ```
    /// use rvoip_sip_core::ProxyAuthenticate;
    /// use rvoip_sip_core::Qop;
    ///
    /// let challenge = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
    ///     .with_qops(vec![Qop::Auth, Qop::AuthInt]);
    /// ```
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Qop(qops));
        }
        self
    }
}

impl FromStr for ProxyAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
         // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_proxy_authenticate(s.as_bytes())
            .map(|(_, challenges)| ProxyAuthenticate(challenges))
            .map_err(Error::from)
    }
}

impl TypedHeaderTrait for ProxyAuthenticate {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ProxyAuthenticate
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::ProxyAuthenticate(self.clone()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::ProxyAuthenticate(proxy_auth) => {
                Ok(proxy_auth.clone())
            },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::auth::challenge::Challenge;
    use crate::types::auth::params::{DigestParam, AuthParam};
    use crate::types::auth::scheme::{Algorithm, Qop};
    use std::str::FromStr;

    #[test]
    fn test_new_proxy_authenticate() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789");
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("abc123xyz789".to_string())));
            assert_eq!(params.len(), 2);
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_new_basic_proxy_authenticate() {
        let proxy_auth = ProxyAuthenticate::new_basic("proxy.example.com");
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Basic { params } = &proxy_auth.0[0] {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "realm");
            assert_eq!(params[0].value, "proxy.example.com");
        } else {
            panic!("Expected Basic challenge");
        }
    }

    #[test]
    fn test_with_algorithm() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_algorithm(Algorithm::Md5);
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("abc123xyz789".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
            assert_eq!(params.len(), 3);
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_with_qop() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_qop(Qop::Auth);
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("abc123xyz789".to_string())));
            
            // Check the Qop parameter
            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
            assert!(qop.is_some());
            if let DigestParam::Qop(qops) = qop.unwrap() {
                assert_eq!(qops.len(), 1);
                assert!(qops.contains(&Qop::Auth));
            }
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_with_qops() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_qops(vec![Qop::Auth, Qop::AuthInt]);
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            // Check the Qop parameter
            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
            assert!(qop.is_some());
            if let DigestParam::Qop(qops) = qop.unwrap() {
                assert_eq!(qops.len(), 2);
                assert!(qops.contains(&Qop::Auth));
                assert!(qops.contains(&Qop::AuthInt));
            }
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_with_domain() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_domain("sip:*.example.com");
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            let domain = params.iter().find(|p| matches!(p, DigestParam::Domain(_)));
            assert!(domain.is_some());
            if let DigestParam::Domain(domains) = domain.unwrap() {
                assert_eq!(domains.len(), 1);
                assert_eq!(domains[0], "sip:*.example.com");
            }
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_with_opaque() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_opaque("opaque_token_123");
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Opaque("opaque_token_123".to_string())));
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_with_stale() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_stale(true);
        
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Stale(true)));
        } else {
            panic!("Expected Digest challenge");
        }
    }

    #[test]
    fn test_add_challenge() {
        let mut proxy_auth = ProxyAuthenticate::new("proxy.example.com", "nonce1");
        
        // Add a second challenge with a different nonce
        proxy_auth.add_challenge(Challenge::Digest { params: vec![
            DigestParam::Realm("proxy.example.com".to_string()),
            DigestParam::Nonce("nonce2".to_string())
        ]});
        
        assert_eq!(proxy_auth.0.len(), 2);
        
        // Check first challenge
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Nonce("nonce1".to_string())));
        } else {
            panic!("Expected first challenge to be Digest");
        }
        
        // Check second challenge
        if let Challenge::Digest { params } = &proxy_auth.0[1] {
            assert!(params.contains(&DigestParam::Nonce("nonce2".to_string())));
        } else {
            panic!("Expected second challenge to be Digest");
        }
    }

    #[test]
    fn test_multiple_challenge_types() {
        let mut proxy_auth = ProxyAuthenticate::new("proxy.example.com", "nonce1");
        
        // Add a Basic challenge
        proxy_auth.add_challenge(Challenge::Basic { params: vec![
            AuthParam { 
                name: "realm".to_string(), 
                value: "proxy.example.com".to_string() 
            }
        ]});
        
        assert_eq!(proxy_auth.0.len(), 2);
        
        // Check first challenge is Digest
        assert!(matches!(proxy_auth.0[0], Challenge::Digest { .. }));
        
        // Check second challenge is Basic
        assert!(matches!(proxy_auth.0[1], Challenge::Basic { .. }));
        
        // Test first_digest and first_basic helpers
        assert!(proxy_auth.first_digest().is_some());
        assert!(proxy_auth.first_basic().is_some());
    }

    #[test]
    fn test_display() {
        let proxy_auth = ProxyAuthenticate::new("proxy.example.com", "abc123xyz789")
            .with_algorithm(Algorithm::Md5)
            .with_qop(Qop::Auth);
        
        let display = format!("{}", proxy_auth);
        println!("Display output: {}", display);
        
        assert!(display.contains("realm=\"proxy.example.com\""));
        assert!(display.contains("nonce=\"abc123xyz789\""));
        assert!(display.contains("algorithm=MD5"));
        
        // The format could be either qop="auth" or qop=auth
        assert!(display.contains("qop="));
        assert!(display.contains("auth"));
    }

    #[test]
    fn test_parse_and_serialize() {
        let proxy_auth_str = r#"Digest realm="proxy.example.com", nonce="abc123xyz789", algorithm=MD5, qop="auth""#;
        
        // Parse from string
        let proxy_auth = ProxyAuthenticate::from_str(proxy_auth_str).unwrap();
        
        // Check parsed values
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("abc123xyz789".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
            
            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
            assert!(qop.is_some());
            if let DigestParam::Qop(qops) = qop.unwrap() {
                assert_eq!(qops.len(), 1);
                assert!(qops.contains(&Qop::Auth));
            }
        } else {
            panic!("Expected Digest challenge");
        }
        
        // Convert to header and back
        let header = proxy_auth.to_header();
        let proxy_auth2 = ProxyAuthenticate::from_header(&header).unwrap();
        
        // Should be the same after round-trip
        assert_eq!(format!("{}", proxy_auth), format!("{}", proxy_auth2));
    }

    #[test]
    fn test_parser_integration() {
        // Test with a complex header value including line folding and multiple parameters
        let header_value = "Digest realm=\"proxy.example.com\",\r\n nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",\r\n algorithm=MD5,\r\n qop=\"auth,auth-int\", opaque=\"5ccc069c403ebaf9f0171e9517f40e41\"";
        
        // Parse the header value
        let proxy_auth = ProxyAuthenticate::from_str(header_value).unwrap();
        
        // Verify the parsed values
        assert_eq!(proxy_auth.0.len(), 1);
        if let Challenge::Digest { params } = &proxy_auth.0[0] {
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())));
            assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)));
            assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())));
            
            // Check the Qop parameter
            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
            assert!(qop.is_some());
            if let DigestParam::Qop(qops) = qop.unwrap() {
                assert_eq!(qops.len(), 2);
                assert!(qops.contains(&Qop::Auth));
                assert!(qops.contains(&Qop::AuthInt));
            }
        } else {
            panic!("Expected Digest challenge");
        }
        
        // Add a Basic challenge
        let mut proxy_auth = proxy_auth;
        proxy_auth.add_challenge(Challenge::Basic { params: vec![
            AuthParam { 
                name: "realm".to_string(), 
                value: "proxy.example.com".to_string() 
            }
        ]});
        
        // Verify both challenges
        assert_eq!(proxy_auth.0.len(), 2);
        assert!(proxy_auth.first_digest().is_some());
        assert!(proxy_auth.first_basic().is_some());
        
        // Convert to string and verify format
        let display = format!("{}", proxy_auth);
        assert!(display.contains("Digest"));
        assert!(display.contains("Basic"));
    }
} 