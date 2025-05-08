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
///
/// This trait provides methods for adding Proxy-Authorization headers to SIP requests,
/// which are used to authenticate clients with proxy servers as specified in
/// [RFC 3261 Section 22.3](https://datatracker.ietf.org/doc/html/rfc3261#section-22.3).
///
/// The Proxy-Authorization header is sent by clients in response to a 407 (Proxy Authentication Required)
/// response containing a Proxy-Authenticate header. It provides authentication credentials that 
/// allow the client to authenticate with the proxy server.
///
/// # SIP Authentication Flow with Proxy Servers
///
/// A typical SIP proxy authentication flow works like this:
///
/// 1. Client sends a request to a destination through a proxy server
/// 2. Proxy server rejects the request with a 407 (Proxy Authentication Required) response 
///    containing a Proxy-Authenticate header with an authentication challenge
/// 3. Client creates a new request with the same Call-ID but incremented CSeq, and adds 
///    a Proxy-Authorization header containing credentials to meet the challenge
/// 4. If the credentials are valid, the proxy allows the request to continue
///
/// # Difference from Authorization Header
///
/// While the structure is similar to the Authorization header, the Proxy-Authorization header
/// serves a different purpose:
///
/// - **Authorization**: Used to authenticate with the final destination server (responds to WWW-Authenticate)
/// - **Proxy-Authorization**: Used to authenticate with proxy servers (responds to Proxy-Authenticate)
///
/// # Digest Authentication in SIP
///
/// Digest authentication is the recommended method for SIP authentication. The client must:
///
/// 1. Extract challenge parameters from the Proxy-Authenticate header
/// 2. Calculate a response value using the appropriate algorithm
/// 3. Include the response and all required parameters in the Proxy-Authorization header
///
/// # Warning: Parameter Order
///
/// **IMPORTANT**: Note that the parameter order for `proxy_authorization_digest()` differs from 
/// `authorization_digest()`. The parameters must be provided in exactly the order specified
/// in the method signature to ensure correct authentication.
///
/// # Examples
///
/// ## Complete Proxy Authentication Flow Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{ProxyAuthenticateExt, ProxyAuthorizationExt};
///
/// // Step 1: Client sends initial INVITE request
/// let initial_request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc")
///     .cseq(1)
///     .max_forwards(70)
///     .contact("<sip:alice@192.168.1.2>", None)
///     .build();
///
/// // Step 2: Proxy challenges with 407 Proxy Authentication Required
/// let nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
/// let challenge_response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_digest(
///         "sip.example.com",         // realm
///         nonce,                     // nonce
///         None,                      // opaque
///         Some("SHA-256"),           // algorithm
///         Some(vec!["auth"]),        // qop
///         None,                      // stale
///         None                       // domain
///     )
///     .to("Bob", "sip:bob@example.com", None)              // Echo To header
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Echo From header
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc")   // Echo Call-ID
///     .cseq(1, Method::Invite)                            // Echo CSeq
///     .build();
///
/// // Step 3: Calculate response (in a real application, this would be done per RFC 2617/7616)
/// // H(username:realm:password) = hash("alice:sip.example.com:secret")
/// let h1 = "e5eaae8fc1a368dc";
/// 
/// // H(method:digest-uri) = hash("INVITE:sip:bob@example.com")
/// let h2 = "849d1cfec93c3c2e";
/// 
/// // If qop is specified: response = hash(h1:nonce:nc:cnonce:qop:h2)
/// // If qop is not specified: response = hash(h1:nonce:h2)
/// let response_hash = "31e9ee75f699bd4b23a36577542c7dcc";
/// let client_nonce = "0a4f113b";
/// 
/// // Step 4: Client creates new request with Proxy-Authorization header
/// let authenticated_request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc") // Same Call-ID
///     .cseq(2)  // Incremented CSeq
///     .max_forwards(70)
///     .contact("<sip:alice@192.168.1.2>", None)
///     // Note the parameter order is different from authorization_digest!
///     .proxy_authorization_digest(
///         "alice",                  // username
///         "sip.example.com",        // realm
///         nonce,                    // nonce
///         "sip:bob@example.com",    // uri
///         response_hash,            // response
///         Some("SHA-256"),          // algorithm
///         Some(client_nonce),       // cnonce
///         None,                     // opaque
///         Some("auth"),             // qop
///         Some("00000001")          // nc (nonce count)
///     )
///     .build();
/// ```
pub trait ProxyAuthorizationExt {
    /// Add a Digest Proxy-Authorization header to the request
    ///
    /// This method adds credentials using the Digest authentication scheme to authenticate
    /// with a proxy server, typically in response to a 407 Proxy Authentication Required
    /// response containing a Proxy-Authenticate header with a Digest challenge.
    ///
    /// Digest authentication is the preferred method for SIP as specified in RFC 3261.
    ///
    /// # WARNING: Parameter Order
    ///
    /// The parameter order for this method differs from `authorization_digest()`. Make
    /// sure you provide the parameters in exactly the order specified below or authentication
    /// will fail.
    ///
    /// # Parameters
    ///
    /// * `username` - The username for authentication
    /// * `realm` - The authentication realm from the Proxy-Authenticate challenge
    /// * `nonce` - The server nonce value from the Proxy-Authenticate challenge
    /// * `uri_str` - The Request-URI string being accessed (typically the same as the request's URI)
    /// * `response` - The calculated response hash value
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None, but SHA-256 is recommended)
    /// * `cnonce` - Optional client nonce (required when qop is "auth" or "auth-int")
    /// * `opaque` - Optional opaque value to be returned unchanged to the server
    /// * `qop` - Optional quality of protection ("auth" or "auth-int")
    /// * `nc` - Optional nonce count (required when qop is specified)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ## Basic Authentication Response
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .proxy_authorization_digest(
    ///         "alice",                                    // username
    ///         "proxy.example.com",                        // realm 
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",      // nonce 
    ///         "sip:example.com",                          // uri 
    ///         "a2ea68c230e5fea1ca715740fb14db97",        // response
    ///         None,                                       // algorithm
    ///         None,                                       // cnonce
    ///         None,                                       // opaque
    ///         None,                                       // qop
    ///         None                                        // nc
    ///     )
    ///     .build();
    /// ```
    ///
    /// ## Complete Authentication with QoP
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .proxy_authorization_digest(
    ///         "alice",                                    // username
    ///         "proxy.example.com",                        // realm 
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",      // nonce 
    ///         "sip:example.com",                          // uri 
    ///         "31e9ee75f699bd4b23a36577542c7dcc",        // response
    ///         Some("SHA-256"),                            // algorithm (stronger than MD5)
    ///         Some("0a4f113b"),                           // cnonce (required with qop)
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"),  // opaque (echoed from challenge)
    ///         Some("auth"),                               // qop
    ///         Some("00000001")                            // nc (required with qop)
    ///     )
    ///     .build();
    /// ```
    ///
    /// ## Multiple Proxies in the Path
    ///
    /// When a request passes through multiple proxies that require authentication:
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// // First get credentials for both proxies (from prior 407 responses)
    /// let request = SimpleRequestBuilder::invite("sip:bob@example.net").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("dsjk32fks"))
    ///     .to("Bob", "sip:bob@example.net", None)
    ///     // Authenticate with first proxy
    ///     .proxy_authorization_digest(
    ///         "alice",
    ///         "proxy1.example.com", 
    ///         "aec91b0342fe144bda9d183a47528ba7", 
    ///         "sip:bob@example.net", 
    ///         "7c8307b0a98cd63fd8c2cf7ce7d9ad5a",
    ///         Some("MD5"),
    ///         Some("43f971bd"),
    ///         None,
    ///         Some("auth"),
    ///         Some("00000001")
    ///     )
    ///     // Authenticate with second proxy
    ///     // Note: The library automatically adds this as a separate header
    ///     .proxy_authorization_digest(
    ///         "alice", 
    ///         "proxy2.example.net",
    ///         "cb75f82f28e5b76a456a5f31c9daa59c",
    ///         "sip:bob@example.net",
    ///         "4fdac86aa1ae67e86edfa72836470ef1",
    ///         Some("SHA-256"),
    ///         Some("92fc06bf"),
    ///         None, 
    ///         Some("auth"),
    ///         Some("00000001")
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
    /// This method adds credentials using the Basic authentication scheme. Basic authentication
    /// simply base64-encodes the username and password, providing minimal security. It should
    /// only be used over secure connections (TLS/SIPS).
    ///
    /// # Security Considerations
    ///
    /// Basic authentication is not recommended for SIP as it transmits credentials with minimal
    /// protection. The password is only base64-encoded (not encrypted), which can be trivially 
    /// decoded. Always prefer Digest authentication unless working in a fully secured environment.
    ///
    /// # Parameters
    ///
    /// * `username` - The username for authentication
    /// * `password` - The password for authentication
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ## Basic Usage
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", None)
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .proxy_authorization_basic("alice", "secret-password")
    ///     .build();
    /// ```
    ///
    /// ## When Basic Authentication Might Be Appropriate
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ProxyAuthorizationExt};
    /// 
    /// // 1. In private networks with TLS
    /// let secure_request = SimpleRequestBuilder::new(Method::Register, "sips:example.com").unwrap()
    ///     .from("Alice", "sips:alice@example.com", None)
    ///     .to("Alice", "sips:alice@example.com", None)
    ///     .proxy_authorization_basic("alice", "secure-password")
    ///     .build();
    ///     
    /// // 2. For testing or development environments
    /// let test_request = SimpleRequestBuilder::new(Method::Register, "sip:test.local").unwrap()
    ///     .from("Test", "sip:test@test.local", None)
    ///     .to("Test", "sip:test@test.local", None)
    ///     .proxy_authorization_basic("test", "test")
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