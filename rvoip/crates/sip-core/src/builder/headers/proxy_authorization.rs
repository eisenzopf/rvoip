use crate::error::{Error, Result};
use crate::types::{
    auth::{
        ProxyAuthorization, 
        Credentials,
        DigestParam,
        AuthScheme, 
        Algorithm, 
        Qop
    },
    TypedHeader,
    header::TypedHeaderTrait,
    Uri,
    headers::header_access::HeaderAccess,
};
use base64::engine::{general_purpose, Engine};
use super::HeaderSetter;

/// Extension trait for adding Proxy-Authorization header building capabilities
pub trait ProxyAuthorizationExt {
    /// Add a Digest Proxy-Authorization header to the request
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
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .proxy_authorization_digest(
    ///         "alice",
    ///         "proxy.example.com", 
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
    fn proxy_authorization_digest(
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
    ) -> Self;
    
    /// Add a Basic Proxy-Authorization header to the request
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
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .proxy_authorization_basic("alice", "secret-password")
    ///     .build();
    /// ```
    fn proxy_authorization_basic(self, username: &str, password: &str) -> Self;
}

impl<T> ProxyAuthorizationExt for T 
where 
    T: HeaderSetter,
{
    fn proxy_authorization_digest(
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
            // Parse QOP type
            let qop_type = match q.to_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                _ => Qop::Other(q.to_string()),
            };
            params.push(DigestParam::MsgQop(qop_type));
            
            // If QOP is specified, nonce count is required
            if let Some(count) = nc {
                // Try to parse the nonce count as a hexadecimal number
                if let Ok(nc_val) = u32::from_str_radix(count.trim_start_matches("0x"), 16) {
                    params.push(DigestParam::NonceCount(nc_val));
                }
            }
        }
        
        // Create the Proxy-Authorization header
        let auth = ProxyAuthorization(Credentials::Digest { params });
        self.set_header(auth)
    }
    
    fn proxy_authorization_basic(self, username: &str, password: &str) -> Self {
        let credentials = format!("{}:{}", username, password);
        let encoded = general_purpose::STANDARD.encode(credentials.as_bytes());
        
        // Create the Proxy-Authorization header with Basic scheme
        let auth = ProxyAuthorization(Credentials::Basic { token: encoded });
        self.set_header(auth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::Method;
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_proxy_authorization_digest() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .proxy_authorization_digest(
                "alice",
                "proxy.example.com", 
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
            
        // Check if Proxy-Authorization header exists and has correct values
        let header = request.header(&HeaderName::ProxyAuthorization);
        assert!(header.is_some(), "Proxy-Authorization header not found");
        
        if let Some(TypedHeader::ProxyAuthorization(ProxyAuthorization(Credentials::Digest { params }))) = header {
            assert!(params.contains(&DigestParam::Username("alice".to_string())));
            assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
            assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())));
            assert!(params.contains(&DigestParam::Response("a2ea68c230e5fea1ca715740fb14db97".to_string())));
            assert!(params.iter().any(|p| matches!(p, DigestParam::Uri(_))));
        } else {
            panic!("Expected Digest credentials");
        }
    }
    
    #[test]
    fn test_proxy_authorization_basic() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .proxy_authorization_basic("alice", "secret-password")
            .build();
            
        // Check if Proxy-Authorization header exists and has correct values
        let header = request.header(&HeaderName::ProxyAuthorization);
        assert!(header.is_some(), "Proxy-Authorization header not found");
        
        if let Some(TypedHeader::ProxyAuthorization(ProxyAuthorization(Credentials::Basic { token }))) = header {
            // For Basic auth, token should contain base64 encoded username:password
            let expected_encoded = general_purpose::STANDARD.encode("alice:secret-password".as_bytes());
            assert_eq!(token, &expected_encoded);
        } else {
            panic!("Expected Basic credentials");
        }
    }
} 