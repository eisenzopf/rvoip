use crate::error::{Error, Result};
use crate::types::{
    auth::{
        Authorization, 
        Credentials,
        DigestParam,
        AuthScheme, 
        Algorithm, 
        Qop
    },
    TypedHeader,
    Uri,
    headers::header_access::HeaderAccess,
};

/// Extension trait for adding Authorization header building capabilities
pub trait AuthorizationExt {
    /// Add a Digest Authorization header to the request
    ///
    /// # Arguments
    ///
    /// * `username` - The username for authentication
    /// * `realm` - The authentication realm
    /// * `nonce` - The server nonce value
    /// * `uri` - The URI being accessed
    /// * `response` - The hashed response value
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None)
    /// * `cnonce` - Optional client nonce for auth-int and auth-qop
    /// * `opaque` - Optional opaque value to be returned unchanged to the server
    /// * `qop` - Optional quality of protection
    /// * `nc` - Optional nonce count for auth-int and auth-qop
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .authorization_digest(
    ///         "alice",
    ///         "example.com", 
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", 
    ///         "sip:example.com", 
    ///         "a2ea68c230e5fea1ca715740fb14db97",
    ///         None,
    ///         None,
    ///         None,
    ///         None,
    ///         None
    ///     )
    ///     .build();
    /// ```
    fn authorization_digest(
        self,
        username: &str,
        realm: &str,
        nonce: &str,
        uri: &str,
        response: &str,
        algorithm: Option<&str>,
        cnonce: Option<&str>,
        opaque: Option<&str>,
        qop: Option<&str>,
        nc: Option<&str>,
    ) -> Self;
    
    /// Add a simple Basic Authorization header to the request
    ///
    /// # Arguments
    ///
    /// * `username` - The username for authentication
    /// * `password` - The password for authentication
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::AuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .authorization_basic("alice", "secret-password")
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
        uri_str: &str,
        response: &str,
        algorithm: Option<&str>,
        cnonce: Option<&str>,
        opaque: Option<&str>,
        qop: Option<&str>,
        nc: Option<&str>,
    ) -> Self {
        // Try to parse the URI
        let uri = match Uri::try_from(uri_str) {
            Ok(u) => u,
            Err(_) => return self,
        };

        // Create digest params
        let mut params = vec![
            DigestParam::Username(username.to_string()),
            DigestParam::Realm(realm.to_string()),
            DigestParam::Nonce(nonce.to_string()),
            DigestParam::Uri(uri),
            DigestParam::Response(response.to_string()),
        ];
        
        // Add optional parameters
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
        
        if let Some(cn) = cnonce {
            params.push(DigestParam::Cnonce(cn.to_string()));
        }
        
        if let Some(op) = opaque {
            params.push(DigestParam::Opaque(op.to_string()));
        }
        
        if let Some(q) = qop {
            // Convert string to Qop enum
            let qop_value = match q.to_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                _ => Qop::Other(q.to_string()),
            };
            params.push(DigestParam::MsgQop(qop_value));
        }
        
        if let Some(n) = nc {
            // Parse nonce count as hex
            if let Ok(nc_value) = u32::from_str_radix(n, 16) {
                params.push(DigestParam::NonceCount(nc_value));
            }
        }
        
        // Create the Authorization header with the digest credentials
        let auth = Authorization(Credentials::Digest { params });
        
        self.set_header(TypedHeader::Authorization(auth))
    }
    
    fn authorization_basic(self, username: &str, password: &str) -> Self {
        use base64::engine::{general_purpose, Engine};
        
        let credentials = format!("{}:{}", username, password);
        let encoded = general_purpose::STANDARD.encode(credentials.as_bytes());
        
        // Create the Authorization header with the Basic credentials
        let auth = Authorization(Credentials::Basic { token: encoded });
        
        self.set_header(TypedHeader::Authorization(auth))
    }
}

/// Internal trait for types that can set headers
/// This should be implemented by request and response builders
pub trait HeaderSetter {
    /// Set a header in the builder
    fn set_header(self, header: TypedHeader) -> Self;
}

// Implementations for the builder types
impl HeaderSetter for crate::builder::SimpleRequestBuilder {
    fn set_header(self, header: TypedHeader) -> Self {
        self.header(header)
    }
}

impl HeaderSetter for crate::builder::SimpleResponseBuilder {
    fn set_header(self, header: TypedHeader) -> Self {
        self.header(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::Method;
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_authorization_digest() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .authorization_digest(
                "alice",
                "example.com", 
                "dcd98b7102dd2f0e8b11d0f600bfb0c093", 
                "sip:example.com", 
                "a2ea68c230e5fea1ca715740fb14db97",
                None,
                None,
                None,
                None,
                None
            )
            .build();
            
        // Check if Authorization header exists and has correct values
        let header = request.header(&HeaderName::Authorization);
        assert!(header.is_some(), "Authorization header not found");
        
        if let Some(TypedHeader::Authorization(Authorization(Credentials::Digest { params }))) = header {
            assert!(params.contains(&DigestParam::Username("alice".to_string())));
            assert!(params.contains(&DigestParam::Realm("example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())));
            assert!(params.contains(&DigestParam::Response("a2ea68c230e5fea1ca715740fb14db97".to_string())));
            assert!(params.iter().any(|p| matches!(p, DigestParam::Uri(_))));
        } else {
            panic!("Expected Digest credentials");
        }
    }
    
    #[test]
    fn test_authorization_basic() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .authorization_basic("alice", "secret-password")
            .build();
            
        // Check if Authorization header exists and has correct values
        let header = request.header(&HeaderName::Authorization);
        assert!(header.is_some(), "Authorization header not found");
        
        if let Some(TypedHeader::Authorization(Authorization(Credentials::Basic { token }))) = header {
            // For Basic auth, token should contain base64 encoded username:password
            use base64::engine::{general_purpose, Engine};
            let expected_encoded = general_purpose::STANDARD.encode("alice:secret-password".as_bytes());
            assert_eq!(token, &expected_encoded);
        } else {
            panic!("Expected Basic credentials");
        }
    }
} 