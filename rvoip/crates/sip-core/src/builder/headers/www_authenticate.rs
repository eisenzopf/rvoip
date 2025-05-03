//! WWW-Authenticate header builder
//!
//! This module provides builder methods for adding WWW-Authenticate headers to SIP responses,
//! used for authentication challenges as defined in RFC 3261 Section 22.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a response with a Digest WWW-Authenticate challenge
//! let response = ResponseBuilder::new(StatusCode::Unauthorized, None)
//!     .www_authenticate_digest(
//!         "sip.example.com",        // realm
//!         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
//!         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
//!         Some("MD5"),              // algorithm
//!         Some(vec!["auth"]),       // qop options
//!         None,                     // stale flag
//!         None,                     // domain
//!     )
//!     .build();
//!
//! // Create a response with a Basic WWW-Authenticate challenge
//! let response = ResponseBuilder::new(StatusCode::Unauthorized, None)
//!     .www_authenticate_basic("sip.example.com")
//!     .build();
//! ```

use crate::error::{Error, Result};
use std::convert::TryFrom;
use crate::types::{
    auth::{
        WwwAuthenticate,
        Challenge,
        DigestParam,
        AuthParam,
        Algorithm,
        Qop
    },
    TypedHeader,
    header::{TypedHeaderTrait, Header, HeaderName},
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait for adding WWW-Authenticate header building capabilities
pub trait WwwAuthenticateExt {
    /// Add a Digest WWW-Authenticate header to the response
    ///
    /// This method adds a WWW-Authenticate header with a Digest authentication challenge
    /// to a SIP response. This is typically used with 401 Unauthorized responses to
    /// challenge the client to authenticate.
    ///
    /// # Parameters
    ///
    /// * `realm` - The authentication realm (mandatory)
    /// * `nonce` - The server nonce value (mandatory)
    /// * `opaque` - Optional opaque value to be returned unchanged
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None)
    /// * `qop` - Optional quality of protection options (auth, auth-int)
    /// * `stale` - Optional stale flag (true if nonce is stale but credentials are valid)
    /// * `domain` - Optional authentication domain (list of URIs that share credentials)
    ///
    /// # Returns
    ///
    /// The builder with the WWW-Authenticate header added
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = ResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_digest(
    ///         "sip.example.com",
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"),
    ///         Some("MD5"),
    ///         Some(vec!["auth", "auth-int"]),
    ///         Some(false),
    ///         Some(vec!["sip:example.com"]),
    ///     )
    ///     .build();
    /// ```
    fn www_authenticate_digest(
        self,
        realm: &str,
        nonce: &str,
        opaque: Option<&str>,
        algorithm: Option<&str>,
        qop: Option<Vec<&str>>,
        stale: Option<bool>,
        domain: Option<Vec<&str>>,
    ) -> Self;

    /// Add a Basic WWW-Authenticate header to the response
    ///
    /// This method adds a WWW-Authenticate header with a Basic authentication challenge
    /// to a SIP response. While Basic authentication is less common in SIP than Digest,
    /// it may be used in simple scenarios.
    ///
    /// # Parameters
    ///
    /// * `realm` - The authentication realm
    ///
    /// # Returns
    ///
    /// The builder with the WWW-Authenticate header added
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = ResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_basic("sip.example.com")
    ///     .build();
    /// ```
    fn www_authenticate_basic(self, realm: &str) -> Self;
}

impl<T> WwwAuthenticateExt for T 
where 
    T: HeaderSetter,
{
    fn www_authenticate_digest(
        self,
        realm: &str,
        nonce: &str,
        opaque: Option<&str>,
        algorithm: Option<&str>,
        qop: Option<Vec<&str>>,
        stale: Option<bool>,
        domain: Option<Vec<&str>>,
    ) -> Self {
        // Create base parameters (required for Digest authentication)
        let mut params = vec![
            DigestParam::Realm(realm.to_string()),
            DigestParam::Nonce(nonce.to_string()),
        ];

        // Add optional parameters if provided
        if let Some(op) = opaque {
            params.push(DigestParam::Opaque(op.to_string()));
        }

        if let Some(alg) = algorithm {
            // Convert string to Algorithm enum
            let algorithm = match alg.to_lowercase().as_str() {
                "md5" => Algorithm::Md5,
                "md5-sess" => Algorithm::Md5Sess,
                "sha-256" | "sha256" => Algorithm::Sha256,
                "sha-256-sess" | "sha256-sess" => Algorithm::Sha256Sess,
                "sha-512-256" | "sha512-256" => Algorithm::Sha512,
                "sha-512-256-sess" | "sha512-256-sess" => Algorithm::Sha512Sess,
                _ => Algorithm::Other(alg.to_string()),
            };
            params.push(DigestParam::Algorithm(algorithm));
        }

        if let Some(q) = qop {
            if !q.is_empty() {
                // Create QoP values with proper conversion
                let mut qops = Vec::new();
                
                for q_str in q {
                    match q_str.to_lowercase().as_str() {
                        "auth" => qops.push(Qop::Auth),
                        "auth-int" => qops.push(Qop::AuthInt),
                        other => qops.push(Qop::Other(other.to_string())),
                    }
                }
                
                if !qops.is_empty() {
                    params.push(DigestParam::Qop(qops));
                }
            }
        }

        if let Some(s) = stale {
            params.push(DigestParam::Stale(s));
        }

        if let Some(d) = domain {
            if !d.is_empty() {
                let domains = d.into_iter().map(|d| d.to_string()).collect();
                params.push(DigestParam::Domain(domains));
            }
        }

        // Create the digest challenge with the parameters
        let digest_challenge = Challenge::Digest { params };
        let header_value = WwwAuthenticate(vec![digest_challenge]);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(header_value)
    }

    fn www_authenticate_basic(self, realm: &str) -> Self {
        // Create the params with just the realm
        let params = vec![
            AuthParam {
                name: "realm".to_string(),
                value: realm.to_string(),
            },
        ];

        // Create the WWW-Authenticate header with a Basic challenge
        let basic_challenge = Challenge::Basic { params };
        let header_value = WwwAuthenticate(vec![basic_challenge]);
        
        // Use the HeaderSetter trait method
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleResponseBuilder;
    use crate::types::StatusCode;
    
    #[test]
    fn test_www_authenticate_digest() {
        // Create a response with a WWW-Authenticate Digest challenge - simplified version first
        let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
            .www_authenticate_digest(
                "sip.example.com",
                "dcd98b7102dd2f0e8b11d0f600bfb0c093",
                Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
                Some("MD5"),                             // algorithm 
                Some(vec!["auth", "auth-int"]),          // Added back QoP
                Some(false),                             // stale
                None,                                    // no domain for now
            )
            .build();
        
        // Print all header names for debugging
        let header_names = response.header_names();
        println!("Response headers: {:?}", header_names);
        
        // Check if WWW-Authenticate header exists and has correct values
        let header = response.header(&HeaderName::WwwAuthenticate);
        println!("WWW-Authenticate header: {:?}", header);
        
        assert!(header.is_some(), "WWW-Authenticate header not found");
        
        if let Some(TypedHeader::WwwAuthenticate(WwwAuthenticate(challenges))) = header {
            assert_eq!(challenges.len(), 1, "Expected exactly one challenge");
            
            if let Challenge::Digest { params } = &challenges[0] {
                println!("Digest params: {:?}", params);
                
                // Check mandatory parameters
                assert!(params.contains(&DigestParam::Realm("sip.example.com".to_string())),
                      "Realm parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())),
                      "Nonce parameter not found or incorrect");
                
                // Check optional parameters
                assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())),
                      "Opaque parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)),
                      "Algorithm parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Stale(false)),
                      "Stale parameter not found or incorrect");
                
                // Check QOP - using a different approach for clearer error messages
                let qop_param = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
                assert!(qop_param.is_some(), "QoP parameter not found");
                
                if let Some(DigestParam::Qop(qops)) = qop_param {
                    println!("QoP values: {:?}", qops);
                    assert_eq!(qops.len(), 2, "Expected exactly 2 QoP values");
                    assert!(qops.contains(&Qop::Auth), "QoP 'auth' value not found");
                    assert!(qops.contains(&Qop::AuthInt), "QoP 'auth-int' value not found");
                }
            } else {
                panic!("Expected Digest challenge");
            }
        } else {
            panic!("Failed to get WWW-Authenticate header or wrong type");
        }
    }
    
    #[test]
    fn test_www_authenticate_basic() {
        // Create a response with a WWW-Authenticate Basic challenge
        let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
            .www_authenticate_basic("sip.example.com")
            .build();
            
        // Check if WWW-Authenticate header exists and has correct values
        let header = response.header(&HeaderName::WwwAuthenticate);
        assert!(header.is_some(), "WWW-Authenticate header not found");
        
        if let Some(TypedHeader::WwwAuthenticate(WwwAuthenticate(challenges))) = header {
            assert_eq!(challenges.len(), 1, "Expected exactly one challenge");
            
            if let Challenge::Basic { params } = &challenges[0] {
                assert_eq!(params.len(), 1, "Expected exactly one parameter in Basic auth");
                let realm_param = &params[0];
                assert_eq!(realm_param.name, "realm");
                assert_eq!(realm_param.value, "sip.example.com");
            } else {
                panic!("Expected Basic challenge");
            }
        } else {
            panic!("Failed to get WWW-Authenticate header or wrong type");
        }
    }
    
    #[test]
    fn test_www_authenticate_minimal() {
        // Test with only mandatory parameters
        let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
            .www_authenticate_digest(
                "sip.example.com",
                "some-nonce-value",
                None, // no opaque
                None, // no algorithm
                None, // no qop
                None, // no stale
                None, // no domain
            )
            .build();
            
        // Check if WWW-Authenticate header exists and has correct values
        let header = response.header(&HeaderName::WwwAuthenticate);
        assert!(header.is_some(), "WWW-Authenticate header not found");
        
        if let Some(TypedHeader::WwwAuthenticate(WwwAuthenticate(challenges))) = header {
            assert_eq!(challenges.len(), 1, "Expected exactly one challenge");
            
            if let Challenge::Digest { params } = &challenges[0] {
                // Should only have realm and nonce parameters
                assert_eq!(params.len(), 2, "Expected exactly two parameters");
                assert!(params.contains(&DigestParam::Realm("sip.example.com".to_string())));
                assert!(params.contains(&DigestParam::Nonce("some-nonce-value".to_string())));
            } else {
                panic!("Expected Digest challenge");
            }
        } else {
            panic!("Failed to get WWW-Authenticate header or wrong type");
        }
    }

    #[test]
    fn test_debug_minimal_auth() {
        // Create a bare-bones WWW-Authenticate header
        let challenge = Challenge::Digest { 
            params: vec![
                DigestParam::Realm("test-realm".to_string()),
                DigestParam::Nonce("test-nonce".to_string())
            ] 
        };
        let www_auth = WwwAuthenticate(vec![challenge]);
        
        // Convert directly to TypedHeader
        let header_val = www_auth.to_header();
        println!("Header value: {:?}", header_val);
        
        let typed_header = match TypedHeader::try_from(header_val) {
            Ok(th) => th,
            Err(e) => {
                println!("Error converting to TypedHeader: {:?}", e);
                panic!("Conversion failed");
            }
        };
        println!("TypedHeader: {:?}", typed_header);
        
        // Create a response with the header
        let mut response = crate::types::sip_response::Response::new(StatusCode::Unauthorized);
        response = response.with_header(typed_header);
        
        // Check if the header exists
        let header_names = response.header_names();
        println!("Response headers: {:?}", header_names);
        
        let header = response.header(&HeaderName::WwwAuthenticate);
        assert!(header.is_some(), "Header not found in response");
        
        println!("Found header: {:?}", header);
    }

    #[test]
    fn test_debug_builder_www_authenticate() {
        // Create a response with a simple WWW-Authenticate Digest challenge using the builder
        let builder = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
            .www_authenticate_digest(
                "test-realm",
                "test-nonce",
                None, // no opaque
                None, // no algorithm
                None, // no qop
                None, // no stale
                None, // no domain
            );
        
        // Build the response
        let response = builder.build();
        
        // Print all header names for debugging
        let header_names = response.header_names();
        println!("Response headers: {:?}", header_names);
        
        // Check if WWW-Authenticate header exists
        let header = response.header(&HeaderName::WwwAuthenticate);
        println!("WWW-Authenticate header: {:?}", header);
        
        assert!(header.is_some(), "WWW-Authenticate header not found");
    }
} 