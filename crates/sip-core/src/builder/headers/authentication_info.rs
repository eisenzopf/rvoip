use crate::error::{Error, Result};
use crate::types::{
    auth::{
        AuthenticationInfo,
        AuthenticationInfoParam,
        Qop
    },
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait for adding Authentication-Info header building capabilities
///
/// This trait provides methods for adding Authentication-Info headers to SIP responses.
/// Authentication-Info headers are defined in RFC 3261 and are sent by servers in 2xx responses
/// following successful authentication with either WWW-Authenticate/Authorization or
/// Proxy-Authenticate/Proxy-Authorization.
///
/// # Purpose of Authentication-Info
///
/// The Authentication-Info header serves several purposes in SIP authentication:
///
/// 1. **Session Continuation**: Provides the next nonce to use for subsequent requests,
///    allowing continued authentication without challenging every request
///
/// 2. **Mutual Authentication**: Proves the server's knowledge of shared credentials
///    through the rspauth parameter
///
/// 3. **State Synchronization**: Acknowledges the client's authentication parameters
///    and maintains authentication state
///
/// # When to Use Authentication-Info
///
/// The Authentication-Info header should be included in successful responses (2xx)
/// following authentication:
///
/// - After successful client authentication via Authorization
/// - As part of an ongoing digest authentication session
/// - When implementing mutual authentication (server proving its identity to client)
/// - When the server wants to update authentication parameters
///
/// # Relationship with Other Authentication Headers
///
/// Authentication-Info works together with other SIP authentication headers:
///
/// ```rust
/// // Authentication flow diagram:
/// //
/// // Client                   Server
/// //   |                         |
/// //   |------- REQUEST -------->|
/// //   |                         |
/// //   |<-- 401 + WWW-Auth. ----|  (Challenge)
/// //   |                         |
/// //   |-- REQ + Authorization ->|  (Response)
/// //   |                         |
/// //   |<-- 200 + Auth-Info ----|  (Acknowledgment + Next credentials)
/// //   |                         |
/// //   |-- REQ + Authorization ->|  (Using nextnonce)
/// //   |                         |
/// ```
///
/// # Security Considerations
///
/// The Authentication-Info header enhances security in SIP in several ways:
///
/// - Protects against replay attacks through nonce rotation
/// - Enables mutual authentication so clients can verify server identity
/// - Maintains authentication state for long-running sessions
///
/// # Examples
///
/// ## Complete Authentication Flow Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{WwwAuthenticateExt, AuthorizationExt, AuthenticationInfoExt};
///
/// // Step 1: Server challenges client with WWW-Authenticate
/// let challenge_response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "example.com",                              // realm
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093"       // nonce
///     )
///     .to("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .from("SIP Server", "sip:registrar@example.com", Some("1232412"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc")
///     .cseq(1, Method::Register)
///     .build();
///
/// // Step 2: Client calculates response and authenticates
/// // In practice, this would be calculated per RFC 2617/7616
/// let response_hash = "31e9ee75f699bd4b23a36577542c7dcc";
/// let client_nonce = "0a4f113b";
/// let uri = "sip:example.com";
///
/// let authenticated_request = SimpleRequestBuilder::register("sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc")
///     .cseq(2)
///     // Note: authorization_digest() parameter order:
///     // username, realm, nonce, response, uri, algorithm, cnonce, opaque, qop, nc, userhash
///     .authorization_digest(
///         "alice",                                    // username
///         "example.com",                              // realm
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",      // nonce
///         response_hash,                              // response
///         Some(uri),                                  // uri (needs to be wrapped in Some)
///         Some("SHA-256"),                            // algorithm
///         Some(client_nonce),                         // cnonce
///         None,                                       // opaque
///         Some("auth"),                               // qop
///         Some("00000001"),                           // nc
///         None                                        // userhash
///     )
///     .build();
///
/// // Step 3: Server validates auth and responds with Authentication-Info
/// // Also calculate server authentication response for mutual auth
/// let server_rspauth = "6629fae49393a05397450978507c4ef1";
/// let next_nonce = "fcb7bae57a15b83854575c0643bb9254"; // Provide new nonce for future requests
///
/// let success_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .to("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .from("SIP Server", "sip:registrar@example.com", Some("1232412"))
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@alice-pc")
///     .cseq(2, Method::Register)
///     .authentication_info(
///         Some(next_nonce),                          // nextnonce (for future requests)
///         Some("auth"),                              // qop (same as in client request)
///         Some(server_rspauth),                      // rspauth (for mutual authentication)
///         Some(client_nonce),                        // cnonce (echoed from client)
///         Some("00000001")                           // nc (echoed from client)
///     )
///     .build();
///
/// // Step 4: Client validates server's rspauth and stores nextnonce for future requests
/// ```
pub trait AuthenticationInfoExt {
    /// Add an Authentication-Info header to the response
    ///
    /// The Authentication-Info header is typically included in 2xx responses following
    /// successful authentication. It provides authentication session continuation
    /// information and can be used for mutual authentication (allowing clients to
    /// verify the server's identity).
    ///
    /// # Parameters
    ///
    /// * `nextnonce` - Optional new nonce value that should be used for the next client request, 
    ///                 helping prevent replay attacks and allowing for credential rotation
    /// * `qop` - Optional quality of protection that was applied to the message (usually "auth"
    ///           or "auth-int"), should match the qop from the client's request
    /// * `rspauth` - Optional server response digest value used for mutual authentication,
    ///              calculated similar to the client's response but using an empty method field
    /// * `cnonce` - Optional client nonce reflected back from the client's Authorization header
    /// * `nc` - Optional nonce count reflected back from the client's Authorization header
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Use cases
    ///
    /// There are several common use cases for the Authentication-Info header:
    ///
    /// 1. **Basic Acknowledgment**: Simply confirm successful authentication
    /// 2. **Nonce Rotation**: Provide a new nonce for continued authentication
    /// 3. **Mutual Authentication**: Prove server identity with rspauth
    /// 4. **Complete Session**: Maintain all authentication parameters
    ///
    /// # Examples
    ///
    /// ## Basic Example with New Nonce
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AuthenticationInfoExt};
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// 
    /// // Send a new nonce for the next request
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
    ///     .authentication_info(
    ///         Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"), // nextnonce
    ///         None,   // no qop needed for basic usage
    ///         None,   // no rspauth needed
    ///         None,   // no cnonce needed
    ///         None    // no nc needed
    ///     )
    ///     .build();
    /// ```
    ///
    /// ## Full Example with Mutual Authentication
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AuthenticationInfoExt};
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// 
    /// // Complete response with mutual authentication
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
    ///     .authentication_info(
    ///         Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"), // nextnonce
    ///         Some("auth"),                               // qop
    ///         Some("6629fae49393a05397450978507c4ef1"),   // rspauth for mutual auth
    ///         Some("8dd675a9"),                           // cnonce (echoed from client)
    ///         Some("00000001")                            // nc (echoed from client)
    ///     )
    ///     .build();
    /// ```
    ///
    /// ## Authentication-Info in a Proxy Response
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::{SimpleResponseBuilder, headers::AuthenticationInfoExt};
    /// use rvoip_sip_core::types::{Method, StatusCode};
    /// 
    /// // A proxy server responding after successful Proxy-Authorization
    /// let proxy_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     // Via headers would normally be here
    ///     .authentication_info(
    ///         Some("9de5b7ac8dc25c96b512dc458d393b35"), // nextnonce
    ///         Some("auth"),                             // qop
    ///         None,                                     // no rspauth (not using mutual auth)
    ///         Some("a71d2bef"),                         // cnonce (from client)
    ///         Some("00000001")                          // nc (from client)
    ///     )
    ///     .build();
    /// ```
    fn authentication_info(
        self,
        nextnonce: Option<&str>,
        qop: Option<&str>,
        rspauth: Option<&str>,
        cnonce: Option<&str>,
        nc: Option<&str>,
    ) -> Self;
}

impl<T> AuthenticationInfoExt for T 
where 
    T: HeaderSetter,
{
    fn authentication_info(
        self,
        nextnonce: Option<&str>,
        qop: Option<&str>,
        rspauth: Option<&str>,
        cnonce: Option<&str>,
        nc: Option<&str>,
    ) -> Self {
        let mut params = Vec::new();

        if let Some(nextnonce_val) = nextnonce {
            params.push(AuthenticationInfoParam::NextNonce(nextnonce_val.to_string()));
        }

        if let Some(qop_val) = qop {
            // Parse QOP type
            let qop_type = match qop_val.to_lowercase().as_str() {
                "auth" => Qop::Auth,
                "auth-int" => Qop::AuthInt,
                _ => Qop::Other(qop_val.to_string()),
            };
            params.push(AuthenticationInfoParam::Qop(qop_type));
        }

        if let Some(rspauth_val) = rspauth {
            params.push(AuthenticationInfoParam::ResponseAuth(rspauth_val.to_string()));
        }

        if let Some(cnonce_val) = cnonce {
            params.push(AuthenticationInfoParam::Cnonce(cnonce_val.to_string()));
        }

        if let Some(nc_val) = nc {
            if let Ok(nc_int) = u32::from_str_radix(nc_val.trim_start_matches("0x"), 16) {
                params.push(AuthenticationInfoParam::NonceCount(nc_int));
            }
        }

        // Only create and set header if at least one parameter is provided
        if !params.is_empty() {
            // Create the header using with_* methods
            let mut header = AuthenticationInfo::new();
            
            // Add all the parameters
            for param in params {
                match param {
                    AuthenticationInfoParam::NextNonce(val) => {
                        header = header.with_nextnonce(val);
                    },
                    AuthenticationInfoParam::Qop(val) => {
                        header = header.with_qop(val);
                    },
                    AuthenticationInfoParam::ResponseAuth(val) => {
                        header = header.with_rspauth(val);
                    },
                    AuthenticationInfoParam::Cnonce(val) => {
                        header = header.with_cnonce(val);
                    },
                    AuthenticationInfoParam::NonceCount(val) => {
                        header = header.with_nonce_count(val);
                    },
                }
            }
            
            self.set_header(header)
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleResponseBuilder;
    use crate::types::{Method, StatusCode};
    use crate::types::header::HeaderName;
    
    #[test]
    fn test_authentication_info() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .from("Alice", "sip:alice@example.com", Some("1928301774"))
            .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
            .authentication_info(
                Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"),
                Some("auth"),
                Some("6629fae49393a05397450978507c4ef1"),
                Some("8dd675a9"),
                Some("00000001")
            )
            .build();
            
        // Check if Authentication-Info header exists and has correct values
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_some(), "Authentication-Info header not found");
        
        if let Some(TypedHeader::AuthenticationInfo(AuthenticationInfo(params))) = header {
            // Look for specific parameters in the output
            let has_nextnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NextNonce(val) if val == "dcd98b7102dd2f0e8b11d0f600bfb0c099")
            });
            assert!(has_nextnonce, "NextNonce parameter not found or incorrect");
            
            let has_qop = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::Qop(Qop::Auth))
            });
            assert!(has_qop, "Qop parameter not found or incorrect");
            
            let has_rspauth = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::ResponseAuth(val) if val == "6629fae49393a05397450978507c4ef1")
            });
            assert!(has_rspauth, "ResponseAuth parameter not found or incorrect");
            
            let has_cnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::Cnonce(val) if val == "8dd675a9")
            });
            assert!(has_cnonce, "Cnonce parameter not found or incorrect");
            
            let has_nc = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NonceCount(val) if *val == 1)
            });
            assert!(has_nc, "NonceCount parameter not found or incorrect");
        } else {
            panic!("Expected Authentication-Info header");
        }
    }
    
    #[test]
    fn test_authentication_info_empty() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .authentication_info(None, None, None, None, None)
            .build();
            
        // Check that Authentication-Info header is NOT added when no parameters are provided
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_none(), "Authentication-Info header should not be present");
    }
    
    #[test]
    fn test_authentication_info_partial() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .authentication_info(
                Some("dcd98b7102dd2f0e8b11d0f600bfb0c099"),
                None,
                None,
                None,
                None
            )
            .build();
            
        // Check if Authentication-Info header exists with just the nextnonce parameter
        let header = response.header(&HeaderName::AuthenticationInfo);
        assert!(header.is_some(), "Authentication-Info header not found");
        
        if let Some(TypedHeader::AuthenticationInfo(AuthenticationInfo(params))) = header {
            assert_eq!(params.len(), 1, "Expected only one parameter");
            
            let has_nextnonce = params.iter().any(|p| {
                matches!(p, AuthenticationInfoParam::NextNonce(val) if val == "dcd98b7102dd2f0e8b11d0f600bfb0c099")
            });
            assert!(has_nextnonce, "NextNonce parameter not found or incorrect");
        } else {
            panic!("Expected Authentication-Info header");
        }
    }
} 