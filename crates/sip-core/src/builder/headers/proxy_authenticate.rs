use crate::error::{Error, Result};
use std::convert::TryFrom;
use crate::types::{
    auth::{
        ProxyAuthenticate,
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

/// Proxy-Authenticate header builder
///
/// This module provides builder methods for adding Proxy-Authenticate headers to SIP responses,
/// used for proxy authentication challenges as defined in RFC 3261 Section 22.
///
/// ## SIP Proxy Authentication Overview
///
/// The Proxy-Authenticate header is a critical component of SIP's hop-by-hop security framework
/// defined in [RFC 3261 Section 22.3](https://datatracker.ietf.org/doc/html/rfc3261#section-22.3).
/// It's sent by proxy servers in 407 (Proxy Authentication Required) responses to challenge
/// clients before forwarding their requests to the destination.
///
/// ## Proxy Authentication Challenge Process
///
/// 1. Client sends a request that passes through a proxy server
/// 2. Proxy responds with 407 Proxy Authentication Required containing a Proxy-Authenticate header
/// 3. Client computes a response using the challenge, user credentials, and request information
/// 4. Client resends the request with a Proxy-Authorization header including the computed response
/// 5. Proxy validates the credentials and forwards the request to its destination if valid
///
/// ## Challenge Types in Proxy Authentication
///
/// - **Digest Authentication**: The primary challenge mechanism for proxies, providing a secure way
///   to authenticate without transmitting passwords in plaintext
/// - **Basic Authentication**: A simple challenge method requiring username:password in Base64 encoding,
///   which should only be used over secure transports like TLS
/// 
/// ## Common Digest Challenge Parameters
///
/// - **realm**: Authentication domain, indicates the protection space (e.g., "sip.example.com")
/// - **nonce**: Proxy-generated unique challenge string (should be cryptographically random)
/// - **opaque**: Data string that should be returned unchanged by the client
/// - **algorithm**: Hash algorithm (e.g., MD5, SHA-256)
/// - **qop** (Quality of Protection): Indicates authentication quality level (auth, auth-int)
/// - **stale**: Indicates if the nonce is stale but credentials might still be valid
/// - **domain**: List of URIs that share the same authentication information
///
/// ## Security Recommendations
///
/// - Use cryptographically random values for nonce generation
/// - Prefer SHA-256 over MD5 for modern deployments
/// - Include client nonce and nonce count support (qop="auth")
/// - Set short nonce lifetimes and use the stale parameter for expired nonces
/// - For high-security environments, use TLS along with Digest authentication
///
/// ## Difference from WWW-Authenticate
///
/// While the structure and parameters are nearly identical to WWW-Authenticate, 
/// the Proxy-Authenticate header is used for different purposes:
///
/// - **WWW-Authenticate**: Used by the final destination server (UAS) for end-to-end authentication
/// - **Proxy-Authenticate**: Used by intermediate proxy servers for hop-by-hop authentication 
///   before forwarding requests
///
/// ## Benefits of Proxy Authentication
///
/// - Controls access to outbound services
/// - Prevents unauthorized use of proxy resources
/// - Enables accounting and billing functionality
/// - Can enforce call routing policies
/// - Allows for traffic management and prioritization
///
/// ## Relationship with other headers
///
/// - **Proxy-Authenticate** vs **Proxy-Authorization**: Proxy-Authenticate presents the challenge,
///   Proxy-Authorization provides the client's response
/// - **Proxy-Authenticate** vs **WWW-Authenticate**: Proxy-Authenticate is for proxy authentication,
///   WWW-Authenticate is for end-server authentication
/// - **Proxy-Authenticate** vs **Authentication-Info**: Proxy-Authenticate initiates the challenge,
///   Authentication-Info provides additional data in successful responses
///
/// # More Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
///
/// // Create a response with a Digest Proxy-Authenticate challenge
/// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_digest(
///         "proxy.example.com",       // realm
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
///         Some("MD5"),               // algorithm
///         Some(vec!["auth"]),        // qop options
///         None,                      // stale flag
///         None,                      // domain
///     )
///     .build();
///
/// // Create a response with a Basic Proxy-Authenticate challenge
/// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_basic("proxy.example.com")
///     .build();
/// ```
///
/// The Proxy-Authenticate header field consists of at least one challenge that indicates
/// the authentication scheme and parameters applicable to a specific realm.
///
/// # Common Use Cases
///
/// - Adding authentication challenges to proxy servers
/// - Implementing security for SIP traffic traversing a proxy
/// - Creating multi-tier authentication systems (both proxy and endpoint authentication)
///
/// # Examples
///
/// ## Complete Proxy Authentication Flow Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::{ProxyAuthenticateExt, ProxyAuthorizationExt};
/// use std::str::FromStr;
///
/// // Step 1: Client sends initial INVITE request through a proxy
/// let initial_request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.net").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.net", None)
///     .contact("<sip:alice@192.168.1.2>", None)
///     .build();
///
/// // Step 2: Proxy challenges client with Proxy-Authenticate
/// // Generate a nonce value (in production, this would be securely generated)
/// let nonce = "3ba1f67c4c2229b3a5fd";
/// 
/// let challenge_response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_digest(
///         "proxy.example.com",       // realm 
///         nonce,                     // nonce
///         None,                      // opaque
///         Some("SHA-256"),           // algorithm
///         Some(vec!["auth"]),        // qop options
///         None,                      // stale flag
///         None,                      // domain
///     )
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Echo From header
///     .to("Bob", "sip:bob@example.net", None)                    // Echo To header
///     .build();
///
/// // Step 3: Client calculates response and sends authenticated request
/// // In a real implementation, the response would be calculated according to RFC 2617
/// let proxy_auth_hash = "7c1d357bec28ae9f4d800967legab276";
///
/// let authenticated_request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.net").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.net", None)
///     .contact("<sip:alice@192.168.1.2>", None)
///     // Note: parameter order differs from authorization_digest!
///     // Order: username, realm, nonce, uri, response, algorithm, cnonce, opaque, qop, nc
///     .proxy_authorization_digest(
///         "alice",                  // username
///         "proxy.example.com",      // realm
///         nonce,                    // nonce (from challenge)
///         "sip:bob@example.net",    // uri
///         proxy_auth_hash,          // response
///         Some("SHA-256"),          // algorithm 
///         Some("8f5666ab"),         // cnonce
///         None,                     // opaque
///         Some("auth"),             // qop
///         Some("00000001")          // nc
///     )
///     .build();
///
/// // Step 4: Proxy forwards the request (not shown in this example)
/// ```
///
/// ## Enterprise SIP Proxy with Domain-Based Authentication
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
///
/// // Scenario: Enterprise SIP proxy that handles multiple domains
/// // and requires different authentication for different services
///
/// // Create a challenge response for VoIP services
/// let voip_challenge = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, 
///                                                Some("Authentication Required"))
///     .proxy_authenticate_digest(
///         "voip.example.com",           // VoIP services realm
///         "e34f8b672ac93ed7f759960da19",
///         Some("a6b7c8d9e0f1"),         // Opaque data
///         Some("SHA-256"),              // Modern algorithm
///         Some(vec!["auth"]),           // QoP with nonce counting
///         None,
///         // Domain list for all VoIP-related services
///         Some(vec![
///             "sip:pbx.example.com", 
///             "sip:voicemail.example.com",
///             "sip:conference.example.com"
///         ])
///     )
///     .build();
///     
/// // Create a challenge response for multimedia services
/// let media_challenge = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, 
///                                                Some("Authentication Required"))
///     .proxy_authenticate_digest(
///         "media.example.com",          // Media services realm
///         "67d89af356c21e94db702c5478", 
///         Some("1f2e3d4c5b6a"),         // Opaque data
///         Some("SHA-256"),              // Modern algorithm
///         Some(vec!["auth"]),           // QoP with nonce counting
///         None,
///         // Domain list for all media-related services
///         Some(vec![
///             "sip:video.example.com",
///             "sip:streaming.example.com", 
///             "sip:recording.example.com"
///         ])
///     )
///     .build();
/// ```
///
/// ## SIP Gateway with Multiple Authentication Mechanisms
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
/// use rvoip_sip_core::types::header::{HeaderName, HeaderValue};
/// use rvoip_sip_core::types::TypedHeader;
/// 
/// // Scenario: SIP Gateway that provides both Digest and TLS-based authentication
/// 
/// // Function to create a challenge response based on client capabilities
/// fn create_gateway_challenge(secure_transport: bool) -> SimpleResponseBuilder {
///     let builder = SimpleResponseBuilder::new(
///         StatusCode::ProxyAuthenticationRequired, 
///         Some("Gateway Authentication Required")
///     );
///     
///     if secure_transport {
///         // For clients on TLS, offer both authentication options
///         builder
///             .proxy_authenticate_digest(
///                 "gateway.example.com",
///                 "891a2b3c4d5e6f7g8h9i0j",
///                 Some("secure-session-data"),
///                 Some("SHA-256"),
///                 Some(vec!["auth"]),
///                 None,
///                 None
///             )
///             // Also offer Basic auth as an option for simpler clients on TLS
///             .proxy_authenticate_basic("gateway.example.com")
///             // Add a custom header with security info
///             .header(TypedHeader::Other(
///                 HeaderName::Other("Security-Scheme".to_string()),
///                 HeaderValue::text("TLS-Required")
///             ))
///     } else {
///         // For non-TLS clients, only offer Digest with mandatory domain restriction
///         builder.proxy_authenticate_digest(
///             "gateway.example.com",
///             "891a2b3c4d5e6f7g8h9i0j", 
///             None,
///             Some("SHA-256"),
///             Some(vec!["auth"]),
///             None,
///             // Restrict to specific domains for non-TLS connections
///             Some(vec!["sip:internal.example.com"])
///         )
///     }
/// }
/// 
/// // Create challenges for different connection types
/// let tls_challenge = create_gateway_challenge(true).build();
/// let standard_challenge = create_gateway_challenge(false).build();
/// ```
///
/// ## Advanced Authentication Options
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
///
/// // Secure challenge with domain restriction
/// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_digest(
///         "proxy.example.com",
///         "9FxHwSyJClx391jQKoMl3Z1",
///         Some("secureOpaque8734"), 
///         Some("SHA-256"),            // SHA-256 for improved security
///         Some(vec!["auth", "auth-int"]), // Support both auth types
///         None,
///         // Restrict authentication to specific domains handled by this proxy
///         Some(vec!["sip:example.com", "sip:voice.example.com", "sip:video.example.com"])
///     )
///     .build();
///
/// // Challenge with stale=true for nonce refresh without requiring new credentials
/// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
///     .proxy_authenticate_digest(
///         "proxy.example.com",
///         "newNonce5349058kdjfd",
///         None,
///         Some("MD5"),
///         Some(vec!["auth"]),
///         Some(true),                 // stale=true - indicates client should retry with new nonce
///         None
///     )
///     .build();
/// ```
pub trait ProxyAuthenticateExt {
    /// Add a Digest Proxy-Authenticate header to the response
    ///
    /// This method adds a Proxy-Authenticate header with a Digest authentication challenge
    /// to a SIP response. This is typically used with 407 Proxy Authentication Required 
    /// responses to challenge the client to authenticate with the proxy.
    ///
    /// ## Digest Challenge in SIP Proxies
    ///
    /// Digest authentication is the preferred authentication method for SIP as defined in
    /// [RFC 3261 Section 22.4](https://datatracker.ietf.org/doc/html/rfc3261#section-22.4),
    /// which builds upon the HTTP Digest Authentication in [RFC 2617](https://datatracker.ietf.org/doc/html/rfc2617).
    ///
    /// For proxies, Digest authentication provides secure hop-by-hop authentication that
    /// prevents unauthorized access to proxy resources and services.
    ///
    /// ## Parameters
    ///
    /// * `realm` - The authentication realm (mandatory) - identifies the protection domain
    /// * `nonce` - The server nonce value (mandatory) - a server-specified data string that should change periodically
    /// * `opaque` - Optional opaque value that must be returned unchanged in the Proxy-Authorization header
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None, but SHA-256 is recommended for security)
    /// * `qop` - Optional quality of protection options (auth, auth-int)
    /// * `stale` - Optional stale flag (true if nonce is stale but credentials are valid)
    /// * `domain` - Optional authentication domain (list of URIs that share credentials)
    ///
    /// ## Security Considerations
    ///
    /// - Use cryptographically random values for nonce generation
    /// - Prefer SHA-256 over MD5 for modern deployments
    /// - Include client nonce and nonce count support (qop="auth")
    /// - Set short nonce lifetimes and use the stale parameter for expired nonces
    /// - For high-security environments, use TLS along with Digest authentication
    ///
    /// ## Returns
    ///
    /// The builder with the Proxy-Authenticate header added
    ///
    /// ## Examples
    ///
    /// ### Basic Challenge
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // Create a minimal digest challenge
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    ///     .proxy_authenticate_digest(
    ///         "proxy.example.com",
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",
    ///         None, // no opaque
    ///         None, // default algorithm (MD5)
    ///         None, // no QoP
    ///         None, // no stale flag
    ///         None, // no domain
    ///     )
    ///     .build();
    /// ```
    ///
    /// ### Secure Challenge with QoP
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // Create a secure challenge with quality of protection options
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    ///     .proxy_authenticate_digest(
    ///         "proxy.example.com",
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // Opaque value
    ///         Some("SHA-256"),                       // Modern algorithm
    ///         Some(vec!["auth"]),                    // Auth QoP
    ///         None,
    ///         Some(vec!["sip:example.com"])          // Domain
    ///     )
    ///     .build();
    /// ```
    ///
    /// ### Load Balancer Challenge with Multiple Domains
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // Load balancer proxy that handles multiple domains
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, 
    ///                                         Some("Authentication Required for Proxy Access"))
    ///     .proxy_authenticate_digest(
    ///         "lb.example.com",                   // Load balancer realm
    ///         "68fd93c0a5b71e94284d7f293e57",    // Random nonce
    ///         Some("load-balancer-state-data"),   // State tracking
    ///         Some("SHA-256"),                    // Secure algorithm
    ///         Some(vec!["auth"]),                 // QoP
    ///         None,
    ///         // Multiple domains handled by this load balancer
    ///         Some(vec![
    ///             "sip:east.example.com", 
    ///             "sip:west.example.com",
    ///             "sip:north.example.com", 
    ///             "sip:south.example.com"
    ///         ])
    ///     )
    ///     .build();
    /// ```
    ///
    /// ### Handling Nonce Expiration
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // When a client uses an expired nonce, send a new challenge with stale=true
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, 
    ///                                          Some("Nonce Expired - Please Refresh"))
    ///     .proxy_authenticate_digest(
    ///         "proxy.example.com",
    ///         "new-nonce-34f8a6c2d9b1", // Fresh nonce value
    ///         None,
    ///         Some("SHA-256"),
    ///         Some(vec!["auth"]),
    ///         Some(true),  // stale=true indicates valid credentials but expired nonce
    ///         None
    ///     )
    ///     .from("Bob", "sip:bob@example.net", Some("proxy-tag"))
    ///     .to("Alice", "sip:alice@example.com", Some("alice-tag"))
    ///     .build();
    /// ```
    fn proxy_authenticate_digest(
        self,
        realm: &str,
        nonce: &str,
        opaque: Option<&str>,
        algorithm: Option<&str>,
        qop: Option<Vec<&str>>,
        stale: Option<bool>,
        domain: Option<Vec<&str>>,
    ) -> Self;

    /// Add a Basic Proxy-Authenticate header to the response
    ///
    /// This method adds a Proxy-Authenticate header with a Basic authentication challenge
    /// to a SIP response. While Basic authentication is less common in SIP than Digest,
    /// it may be used in simple scenarios or for legacy compatibility.
    ///
    /// ## Basic Authentication in SIP Proxies
    ///
    /// Basic authentication simply requires the client to provide Base64-encoded 
    /// "username:password" credentials. It is defined in [RFC 7617](https://datatracker.ietf.org/doc/html/rfc7617)
    /// for HTTP and adapted for SIP.
    ///
    /// Because this authentication method transmits the password with minimal protection,
    /// it should only be used over secure transports like TLS (SIPS).
    ///
    /// ## Security Considerations
    ///
    /// Basic authentication transmits credentials with minimal protection (only base64 encoding,
    /// which is trivial to decode). It should only be used over secure connections (like TLS)
    /// and is generally not recommended for SIP proxy authentication. Digest authentication
    /// provides much better security.
    ///
    /// ## Parameters
    ///
    /// * `realm` - The authentication realm (protection domain)
    ///
    /// ## Returns
    ///
    /// The builder with the Proxy-Authenticate header added
    ///
    /// ## Examples
    ///
    /// ### Simple Basic Challenge
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // Create a basic authentication challenge
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    ///     .proxy_authenticate_basic("proxy.example.com")
    ///     .build();
    /// ```
    ///
    /// ### When to Use Basic Authentication
    ///
    /// Basic authentication might be appropriate in these limited scenarios:
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    ///
    /// // 1. Within private networks with TLS
    /// let private_response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    ///     .proxy_authenticate_basic("internal.proxy.corp.example.com")
    ///     .build();
    ///     
    /// // 2. As a fallback when a client doesn't support digest
    /// let fallback_response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
    ///     // Primary authentication method (preferred)
    ///     .proxy_authenticate_digest(
    ///         "proxy.example.com",
    ///         "nonce123456",
    ///         None, None, None, None, None
    ///     )
    ///     // Fallback authentication method (less secure)
    ///     .proxy_authenticate_basic("proxy.example.com")
    ///     .build();
    /// ```
    ///
    /// ### TLS-Based Basic Authentication
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::builder::headers::ProxyAuthenticateExt;
    /// use rvoip_sip_core::types::header::{HeaderName, HeaderValue};
    /// use rvoip_sip_core::types::TypedHeader;
    ///
    /// // When using Basic auth, always ensure TLS is used
    /// let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, 
    ///                                          Some("TLS Authentication Required"))
    ///     .proxy_authenticate_basic("secure.proxy.example.com")
    ///     // Add headers indicating security requirements
    ///     .header(TypedHeader::Other(
    ///         HeaderName::Other("Security-Scheme".to_string()),
    ///         HeaderValue::text("TLS-Required")
    ///     ))
    ///     .build();
    /// ```
    fn proxy_authenticate_basic(self, realm: &str) -> Self;
}

impl<T> ProxyAuthenticateExt for T 
where
    T: HeaderSetter,
{
    fn proxy_authenticate_digest(
        self,
        realm: &str,
        nonce: &str,
        opaque: Option<&str>,
        algorithm: Option<&str>,
        qop: Option<Vec<&str>>,
        stale: Option<bool>,
        domain: Option<Vec<&str>>,
    ) -> Self {
        // Create the parameter collection for the Digest challenge
        let mut params = vec![
            DigestParam::Realm(realm.to_string()),
            DigestParam::Nonce(nonce.to_string()),
        ];
        
        // Add optional parameters if provided
        if let Some(opaque_str) = opaque {
            params.push(DigestParam::Opaque(opaque_str.to_string()));
        }
        
        if let Some(algorithm_str) = algorithm {
            // Convert string to Algorithm enum
            let algorithm = match algorithm_str.to_lowercase().as_str() {
                "md5" => Algorithm::Md5,
                "md5-sess" => Algorithm::Md5Sess,
                "sha-256" | "sha256" => Algorithm::Sha256,
                "sha-256-sess" | "sha256-sess" => Algorithm::Sha256Sess,
                "sha-512-256" | "sha512-256" => Algorithm::Sha512,
                "sha-512-256-sess" | "sha512-256-sess" => Algorithm::Sha512Sess,
                _ => Algorithm::Other(algorithm_str.to_string()),
            };
            params.push(DigestParam::Algorithm(algorithm));
        }
        
        if let Some(qop_values) = qop {
            if !qop_values.is_empty() {
                let mut qops = Vec::new();
                for qop_str in qop_values {
                    // Convert string to Qop enum
                    let qop_val = match qop_str.to_lowercase().as_str() {
                        "auth" => Qop::Auth,
                        "auth-int" => Qop::AuthInt,
                        _ => Qop::Other(qop_str.to_string()),
                    };
                    qops.push(qop_val);
                }
                if !qops.is_empty() {
                    params.push(DigestParam::Qop(qops));
                }
            }
        }
        
        if let Some(stale_flag) = stale {
            params.push(DigestParam::Stale(stale_flag));
        }
        
        if let Some(domain_values) = domain {
            if !domain_values.is_empty() {
                let domains = domain_values.iter().map(|s| s.to_string()).collect();
                params.push(DigestParam::Domain(domains));
            }
        }
        
        // Create the ProxyAuthenticate header with the Digest challenge
        let header_value = ProxyAuthenticate(vec![Challenge::Digest { params }]);
        
        // Add the header to the builder
        self.set_header(header_value)
    }
    
    fn proxy_authenticate_basic(self, realm: &str) -> Self {
        // Create a Basic challenge with just a realm parameter
        let basic_challenge = Challenge::Basic { 
            params: vec![
                AuthParam { 
                    name: "realm".to_string(), 
                    value: realm.to_string() 
                }
            ] 
        };
        
        // Create the ProxyAuthenticate header with the Basic challenge
        let header_value = ProxyAuthenticate(vec![basic_challenge]);
        
        // Add the header to the builder
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleResponseBuilder;
    use crate::types::StatusCode;
    use crate::types::header::HeaderName;
    use crate::types::auth::{DigestParam, Qop, Algorithm};
    
    #[test]
    fn test_proxy_authenticate_digest() {
        // Test basic parameters
        let digest_params = vec![
            ("realm", "proxy.example.com"),
            ("nonce", "dcd98b7102dd2f0e8b11d0f600bfb0c093")
        ];
        
        // Create a response with just the required parameters
        let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
            .proxy_authenticate_digest(
                digest_params[0].1, // realm
                digest_params[1].1, // nonce
                None,               // opaque
                None,               // algorithm
                None,               // qop
                None,               // stale
                None,               // domain
            )
            .build();
            
        // Verify the Proxy-Authenticate header was added correctly
        let proxy_auth = response.header(&HeaderName::ProxyAuthenticate);
        assert!(proxy_auth.is_some());
        
        // Check the parameters of the challenge
        if let Some(header) = proxy_auth {
            match header {
                crate::types::TypedHeader::ProxyAuthenticate(auth) => {
                    assert_eq!(auth.0.len(), 1);
                    if let Challenge::Digest { params } = &auth.0[0] {
                        assert_eq!(params.len(), 2); // There should be 2 parameters (realm and nonce)
                        assert!(params.contains(&DigestParam::Realm(digest_params[0].1.to_string())));
                        assert!(params.contains(&DigestParam::Nonce(digest_params[1].1.to_string())));
                    } else {
                        panic!("Expected Digest challenge");
                    }
                },
                _ => panic!("Expected ProxyAuthenticate header"),
            }
        }
        
        // Test complete challenge with all optional parameters
        let response2 = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
            .proxy_authenticate_digest(
                "proxy.example.com",                      // realm
                "dcd98b7102dd2f0e8b11d0f600bfb0c093",    // nonce
                Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
                Some("SHA-256"),                         // algorithm
                Some(vec!["auth", "auth-int"]),          // qop
                Some(false),                             // stale
                Some(vec!["sip:example.com"]),           // domain
            )
            .build();
            
        // Verify the Proxy-Authenticate header
        let proxy_auth2 = response2.header(&HeaderName::ProxyAuthenticate);
        assert!(proxy_auth2.is_some());
        
        // Verify all parameters
        if let Some(header) = proxy_auth2 {
            match header {
                crate::types::TypedHeader::ProxyAuthenticate(auth) => {
                    assert_eq!(auth.0.len(), 1);
                    if let Challenge::Digest { params } = &auth.0[0] {
                        // We expect 6 distinct parameter types
                        assert!(params.len() >= 6, "Expected at least 6 parameters, found {}", params.len());
                        
                        // Check required parameters
                        assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
                        assert!(params.contains(&DigestParam::Nonce("dcd98b7102dd2f0e8b11d0f600bfb0c093".to_string())));
                        
                        // Check optional parameters
                        assert!(params.contains(&DigestParam::Opaque("5ccc069c403ebaf9f0171e9517f40e41".to_string())));
                        assert!(params.contains(&DigestParam::Algorithm(Algorithm::Sha256)));
                        assert!(params.contains(&DigestParam::Stale(false)));
                        
                        // Check that Qop has both auth and auth-int
                        let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
                        assert!(qop.is_some());
                        if let DigestParam::Qop(qops) = qop.unwrap() {
                            assert_eq!(qops.len(), 2);
                            assert!(qops.contains(&Qop::Auth));
                            assert!(qops.contains(&Qop::AuthInt));
                        }
                        
                        // Check domain parameter
                        let domain = params.iter().find(|p| matches!(p, DigestParam::Domain(_)));
                        assert!(domain.is_some());
                        if let DigestParam::Domain(domains) = domain.unwrap() {
                            assert_eq!(domains.len(), 1);
                            assert_eq!(domains[0], "sip:example.com");
                        }
                    } else {
                        panic!("Expected Digest challenge");
                    }
                },
                _ => panic!("Expected ProxyAuthenticate header"),
            }
        }
    }
    
    #[test]
    fn test_proxy_authenticate_basic() {
        // Test basic authentication
        let realm = "proxy.example.com";
        
        // Create a response with a Basic challenge
        let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
            .proxy_authenticate_basic(realm)
            .build();
            
        // Verify the Proxy-Authenticate header was added
        let proxy_auth = response.header(&HeaderName::ProxyAuthenticate);
        assert!(proxy_auth.is_some());
        
        // Verify it's a Basic challenge with the correct realm
        if let Some(header) = proxy_auth {
            match header {
                crate::types::TypedHeader::ProxyAuthenticate(auth) => {
                    assert_eq!(auth.0.len(), 1);
                    if let Challenge::Basic { params } = &auth.0[0] {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "realm");
                        assert_eq!(params[0].value, realm);
                    } else {
                        panic!("Expected Basic challenge");
                    }
                },
                _ => panic!("Expected ProxyAuthenticate header"),
            }
        }
    }

    #[test]
    fn test_proxy_authenticate_multiple_challenges() {
        // Test adding multiple challenges
        let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
            .proxy_authenticate_digest(
                "proxy.example.com",
                "nonce123",
                None, None, None, None, None
            )
            .proxy_authenticate_basic("proxy.example.com")
            .build();

        // Get all Proxy-Authenticate headers
        let headers = response.headers(&HeaderName::ProxyAuthenticate);
        assert_eq!(headers.len(), 2, "Should have two Proxy-Authenticate headers");

        // Verify Digest challenge in first header
        if let crate::types::TypedHeader::ProxyAuthenticate(auth) = &headers[0] {
            assert_eq!(auth.0.len(), 1);
            if let Challenge::Digest { params } = &auth.0[0] {
                assert!(params.contains(&DigestParam::Realm("proxy.example.com".to_string())));
                assert!(params.contains(&DigestParam::Nonce("nonce123".to_string())));
            } else {
                panic!("Expected first challenge to be Digest");
            }
        } else {
            panic!("Expected ProxyAuthenticate header");
        }
        
        // Verify Basic challenge in second header
        if let crate::types::TypedHeader::ProxyAuthenticate(auth) = &headers[1] {
            assert_eq!(auth.0.len(), 1);
            if let Challenge::Basic { params } = &auth.0[0] {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].name, "realm");
                assert_eq!(params[0].value, "proxy.example.com");
            } else {
                panic!("Expected second challenge to be Basic");
            }
        } else {
            panic!("Expected ProxyAuthenticate header");
        }
    }

    #[test]
    fn test_proxy_authenticate_algorithm_variants() {
        // Test different algorithm variants
        let algorithms = vec![
            ("MD5", Algorithm::Md5),
            ("SHA-256", Algorithm::Sha256),
            ("SHA-512-256", Algorithm::Sha512),
            ("CUSTOM", Algorithm::Other("CUSTOM".to_string())),
        ];

        for (algo_str, expected_algo) in algorithms {
            let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
                .proxy_authenticate_digest(
                    "proxy.example.com",
                    "nonce123",
                    None,
                    Some(algo_str),
                    None,
                    None,
                    None
                )
                .build();

            let proxy_auth = response.header(&HeaderName::ProxyAuthenticate);
            assert!(proxy_auth.is_some());

            if let Some(header) = proxy_auth {
                match header {
                    crate::types::TypedHeader::ProxyAuthenticate(auth) => {
                        if let Challenge::Digest { params } = &auth.0[0] {
                            assert!(params.contains(&DigestParam::Algorithm(expected_algo.clone())));
                        } else {
                            panic!("Expected Digest challenge");
                        }
                    },
                    _ => panic!("Expected ProxyAuthenticate header"),
                }
            }
        }
    }

    #[test]
    fn test_proxy_authenticate_qop_variants() {
        // Test different QoP variants
        let qop_combinations = vec![
            vec!["auth"],
            vec!["auth-int"],
            vec!["auth", "auth-int"],
            vec!["custom"],
        ];

        for qops in qop_combinations {
            let response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, None)
                .proxy_authenticate_digest(
                    "proxy.example.com",
                    "nonce123",
                    None,
                    None,
                    Some(qops.clone()),
                    None,
                    None
                )
                .build();

            let proxy_auth = response.header(&HeaderName::ProxyAuthenticate);
            assert!(proxy_auth.is_some());

            if let Some(header) = proxy_auth {
                match header {
                    crate::types::TypedHeader::ProxyAuthenticate(auth) => {
                        if let Challenge::Digest { params } = &auth.0[0] {
                            let qop = params.iter().find(|p| matches!(p, DigestParam::Qop(_)));
                            assert!(qop.is_some());
                            if let DigestParam::Qop(parsed_qops) = qop.unwrap() {
                                assert_eq!(parsed_qops.len(), qops.len());
                                for qop_str in qops {
                                    match qop_str {
                                        "auth" => assert!(parsed_qops.contains(&Qop::Auth)),
                                        "auth-int" => assert!(parsed_qops.contains(&Qop::AuthInt)),
                                        _ => assert!(parsed_qops.contains(&Qop::Other(qop_str.to_string()))),
                                    }
                                }
                            }
                        } else {
                            panic!("Expected Digest challenge");
                        }
                    },
                    _ => panic!("Expected ProxyAuthenticate header"),
                }
            }
        }
    }
} 