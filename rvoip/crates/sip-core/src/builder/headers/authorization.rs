//! Authorization header builder
//!
//! This module provides builder methods for adding Authorization headers to SIP requests,
//! used for providing authentication credentials as defined in RFC 3261 Section 22.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a request with a Digest Authorization
//! let request = RequestBuilder::new(Method::Register, "sip:example.com")
//!     .authorization_digest(
//!         "alice",                 // username
//!         "sip.example.com",       // realm
//!         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
//!         "5ccc069c403ebaf9f0171e9517f40e41",   // response
//!         Some("0a4f113b"),        // cnonce 
//!         Some("auth"),            // qop
//!         Some("00000001"),        // nc
//!         Some("REGISTER"),        // method
//!         Some("sip:example.com"), // uri
//!         Some("MD5"),             // algorithm
//!         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
//!     )
//!     .build();
//!
//! // Create a request with a Basic Authorization
//! let request = RequestBuilder::new(Method::Register, "sip:example.com")
//!     .authorization_basic("alice", "password123")
//!     .build();
//! ```

use base64::{Engine as _, engine::general_purpose};
use crate::error::Result;
use crate::types::{
    auth::{
        Authorization,
        Challenge,
        DigestParam,
        AuthParam,
        Algorithm,
        Qop,
        Credentials,
    },
    TypedHeader,
    header::{TypedHeaderTrait, Header, HeaderName},
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait for adding Authorization header building capabilities
pub trait AuthorizationExt {
    /// Add a Digest Authorization header to the request
    ///
    /// This method adds an Authorization header with a Digest authentication response
    /// to a SIP request. This is typically used in response to a 401 Unauthorized challenge.
    ///
    /// # Parameters
    ///
    /// * `username` - The authentication username (mandatory)
    /// * `realm` - The authentication realm (mandatory)
    /// * `nonce` - The server-provided nonce value (mandatory)
    /// * `response` - The computed response hash (mandatory)
    /// * `cnonce` - Optional client nonce (required for qop=auth/auth-int)
    /// * `qop` - Optional quality of protection (auth, auth-int)
    /// * `nc` - Optional nonce count (required for qop=auth/auth-int)
    /// * `method` - Optional method for proper response calculation
    /// * `uri` - Optional URI for proper response calculation
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None)
    /// * `opaque` - Optional opaque value to be returned unchanged
    ///
    /// # Returns
    ///
    /// The builder with the Authorization header added
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = RequestBuilder::new(Method::Register, "sip:example.com")
    ///     .authorization_digest(
    ///         "alice",
    ///         "sip.example.com",
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",
    ///         "5ccc069c403ebaf9f0171e9517f40e41",
    ///         Some("0a4f113b"),
    ///         Some("auth"),
    ///         Some("00000001"),
    ///         Some("REGISTER"),
    ///         Some("sip:example.com"),
    ///         Some("MD5"),
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"),
    ///     )
    ///     .build();
    /// ```
    fn authorization_digest(
        self,
        username: &str,
        realm: &str,
        nonce: &str,
        response: &str,
        cnonce: Option<&str>,
        qop: Option<&str>,
        nc: Option<&str>,
        method: Option<&str>,
        uri: Option<&str>,
        algorithm: Option<&str>,
        opaque: Option<&str>,
    ) -> Self;

    /// Add a Basic Authorization header to the request
    ///
    /// This method adds an Authorization header with a Basic authentication response
    /// to a SIP request. Basic authentication simply base64-encodes the username and password.
    ///
    /// # Parameters
    ///
    /// * `username` - The authentication username
    /// * `password` - The authentication password
    ///
    /// # Returns
    ///
    /// The builder with the Authorization header added
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = RequestBuilder::new(Method::Register, "sip:example.com")
    ///     .authorization_basic("alice", "password123")
    ///     .build();
    /// ```
    fn authorization_basic(self, username: &str, password: &str) -> Self;
}

impl<T> AuthorizationExt for T
where
    T: HeaderSetter,
{
    fn authorization_digest(
        self,
        username: &str,
        realm: &str,
        nonce: &str,
        response: &str,
        cnonce: Option<&str>,
        qop: Option<&str>,
        nc: Option<&str>,
        method: Option<&str>,
        uri: Option<&str>,
        algorithm: Option<&str>,
        opaque: Option<&str>,
    ) -> Self {
        // Create the params starting with mandatory fields
        let mut params = vec![
            DigestParam::Username(username.to_string()),
            DigestParam::Realm(realm.to_string()),
            DigestParam::Nonce(nonce.to_string()),
            DigestParam::Response(response.to_string()),
        ];

        // Add the URI if provided
        if let Some(uri_value) = uri {
            // Try to parse the URI, but fallback to a string value if needed
            match crate::types::uri::Uri::try_from(uri_value) {
                Ok(parsed_uri) => params.push(DigestParam::Uri(parsed_uri)),
                Err(_) => {
                    // If we can't parse it as a URI, use a generic parameter
                    params.push(DigestParam::Param(AuthParam {
                        name: "uri".to_string(),
                        value: uri_value.to_string(),
                    }));
                }
            }
        }

        // Add algorithm if provided
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

        // Add qop if provided
        if let Some(qop_value) = qop {
            let qop_type = match qop_value.to_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                _ => Qop::Other(qop_value.to_string()),
            };
            params.push(DigestParam::MsgQop(qop_type));
        }

        // Add cnonce if provided
        if let Some(cnonce_value) = cnonce {
            params.push(DigestParam::Cnonce(cnonce_value.to_string()));
        }

        // Add nonce count if provided
        if let Some(nc_value) = nc {
            // Try to parse the nonce count as a hexadecimal number
            if let Ok(nc_val) = u32::from_str_radix(nc_value.trim_start_matches("0x"), 16) {
                params.push(DigestParam::NonceCount(nc_val));
            }
        }

        // Add opaque if provided
        if let Some(opaque_value) = opaque {
            params.push(DigestParam::Opaque(opaque_value.to_string()));
        }

        // Create the digest challenge response
        let credentials = Credentials::Digest { params };
        let header_value = Authorization(credentials);
        
        // Use the HeaderSetter trait to set the header
        self.set_header(header_value)
    }

    fn authorization_basic(self, username: &str, password: &str) -> Self {
        // Create the credentials string and base64 encode it
        let credentials = format!("{}:{}", username, password);
        let encoded = general_purpose::STANDARD.encode(credentials);
        
        // Create a Basic authorization with the encoded credentials
        let auth = Authorization(Credentials::Basic { token: encoded });
        
        // Use the HeaderSetter trait to set the header
        self.set_header(auth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::Method;
    
    #[test]
    fn test_authorization_digest() {
        // Create a request with a Digest Authorization
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .expect("Failed to create request builder")
            .authorization_digest(
                "alice",
                "sip.example.com",
                "dcd98b7102dd2f0e8b11d0f600bfb0c093",
                "5ccc069c403ebaf9f0171e9517f40e41",
                Some("0a4f113b"),
                Some("auth"),
                Some("00000001"),
                Some("REGISTER"),
                Some("sip:example.com"),
                Some("MD5"),
                Some("5ccc069c403ebaf9f0171e9517f40e41"),
            )
            .build();
            
        // Print all header names for debugging
        let header_names = request.header_names();
        println!("Request headers: {:?}", header_names);
        
        // Check if Authorization header exists and has correct values
        let header = request.header(&HeaderName::Authorization);
        println!("Authorization header: {:?}", header);
        
        assert!(header.is_some(), "Authorization header not found");
        
        if let Some(TypedHeader::Authorization(Authorization(credentials))) = header {
            if let Credentials::Digest { params } = credentials {
                println!("Digest params: {:?}", params);
                
                // Check mandatory parameters
                assert!(params.contains(&DigestParam::Username("alice".to_string())),
                      "Username parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Realm("sip.example.com".to_string())),
                      "Realm parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())),
                      "Nonce parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Response("5ccc069c403ebaf9f0171e9517f40e41".to_string())),
                      "Response parameter not found or incorrect");
                
                // Check optional parameters
                // URI could be either a parsed URI or a generic Param
                let has_uri = params.iter().any(|p| match p {
                    DigestParam::Uri(uri) => uri.to_string().contains("sip:example.com"),
                    DigestParam::Param(param) => param.name == "uri" && param.value == "sip:example.com",
                    _ => false,
                });
                assert!(has_uri, "URI parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Algorithm(Algorithm::Md5)),
                      "Algorithm parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())),
                      "Opaque parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::MsgQop(Qop::Auth)),
                      "QoP parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::Cnonce("0a4f113b".to_string())),
                      "Cnonce parameter not found or incorrect");
                
                assert!(params.contains(&DigestParam::NonceCount(1)),
                      "Nonce count parameter not found or incorrect");
            } else {
                panic!("Expected Digest challenge response");
            }
        } else {
            panic!("Failed to get Authorization header or wrong type");
        }
    }
    
    #[test]
    fn test_authorization_basic() {
        // Create a request with a Basic Authorization
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .expect("Failed to create request builder")
            .authorization_basic("alice", "password123")
            .build();
            
        // Check if Authorization header exists and has correct values
        let header = request.header(&HeaderName::Authorization);
        assert!(header.is_some(), "Authorization header not found");
        
        if let Some(TypedHeader::Authorization(Authorization(credentials))) = header {
            if let Credentials::Basic { token } = credentials {
                // Verify that the token is properly base64 encoded
                let decoded = general_purpose::STANDARD.decode(token).unwrap();
                let decoded_str = String::from_utf8(decoded).unwrap();
                assert_eq!(decoded_str, "alice:password123");
            } else {
                panic!("Expected Basic challenge response");
            }
        } else {
            panic!("Failed to get Authorization header or wrong type");
        }
    }
} 