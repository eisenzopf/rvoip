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

/// WWW-Authenticate header builder
///
/// This module provides builder methods for adding WWW-Authenticate headers to SIP responses,
/// used for authentication challenges as defined in RFC 3261 Section 22.
///
/// ## SIP Authentication Challenge Overview
///
/// The WWW-Authenticate header is a key component of SIP's security framework defined in
/// [RFC 3261 Section 22](https://datatracker.ietf.org/doc/html/rfc3261#section-22). It's sent
/// by a server in 401 (Unauthorized) responses to challenge the client to provide valid credentials
/// before accessing the requested resource.
///
/// ## Authentication Challenge Process
///
/// 1. Client sends an initial request (e.g., REGISTER, INVITE)
/// 2. Server responds with 401 Unauthorized containing a WWW-Authenticate header with challenge parameters
/// 3. Client computes a response using the challenge, user credentials, and request information
/// 4. Client resends the request with an Authorization header including the computed response
/// 5. Server validates the credentials and processes the request if valid
///
/// ## Challenge Types in SIP
///
/// - **Digest Authentication**: The primary challenge mechanism in SIP, providing a secure way
///   to authenticate without transmitting passwords in plaintext
/// - **Basic Authentication**: A simple challenge method requiring username:password in Base64 encoding,
///   which should only be used over secure transports like TLS
/// 
/// ## Common Digest Challenge Parameters
///
/// - **realm**: Authentication domain, indicates the protection space
/// - **nonce**: Server-generated unique challenge string
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
/// ## Relationship with other headers
///
/// - **WWW-Authenticate** vs **Authorization**: WWW-Authenticate presents the challenge from the server,
///   Authorization provides the client's response to that challenge
/// - **WWW-Authenticate** vs **Proxy-Authenticate**: WWW-Authenticate is used by UAS/registrars for
///   end-to-end authentication, Proxy-Authenticate is used by proxies for hop-by-hop authentication
/// - **WWW-Authenticate** vs **Authentication-Info**: WWW-Authenticate initiates the challenge,
///   Authentication-Info provides additional authentication data in successful responses
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
///
/// // Create a response with a Digest WWW-Authenticate challenge
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "sip.example.com",        // realm
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
///         Some("MD5"),              // algorithm
///         Some(vec!["auth"]),       // qop options
///         None,                     // stale flag
///         None,                     // domain
///     )
///     .build();
///
/// // Create a response with a Basic WWW-Authenticate challenge
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_basic("sip.example.com")
///     .build();
/// ```
///
/// The WWW-Authenticate header field consists of at least one challenge that indicates
/// the authentication scheme and parameters applicable to a specific realm.
///
/// # Common Use Cases
///
/// - Adding a Digest authentication challenge to 401 Unauthorized responses
/// - Supporting multiple authentication schemes in a single response
/// - Implementing security for sensitive SIP operations like registration
///
/// # More Examples
///
/// ## Complete Authentication Flow Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
/// use rvoip_sip_core::builder::headers::AuthorizationExt;
/// use std::str::FromStr;
///
/// // Step 1: Client sends initial REGISTER request
/// let initial_request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2>", None)
///     .build();
///
/// // Step 2: Server challenges client with WWW-Authenticate
/// // Generate a nonce value (in production, this would be securely generated)
/// let nonce = "dcd98b7102dd2f0e8b11d0f600bfb0c093";
/// 
/// let challenge_response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "sip.example.com",          // realm 
///         nonce,                      // nonce
///         None,                       // opaque
///         Some("MD5"),                // algorithm
///         Some(vec!["auth"]),         // qop options
///         None,                       // stale flag
///         None,                       // domain
///     )
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Echo From header
///     .to("Alice", "sip:alice@example.com", None)                // Echo To header
///     .build();
///
/// // Step 3: Client calculates response and sends authenticated request
/// // In a real implementation, the response would be calculated according to RFC 2617
/// // For example: MD5(username:realm:password) etc.
/// let auth_response = "5ccc069c403ebaf9f0171e9517f40e41";
///
/// let authenticated_request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2>", None)
///     .authorization_digest(
///         "alice",                 // username
///         "sip.example.com",       // realm (from challenge)
///         nonce,                   // nonce (from challenge)
///         auth_response,           // calculated response
///         Some("0a4f113b"),        // cnonce (client nonce)
///         Some("auth"),            // qop (from challenge)
///         Some("00000001"),        // nonce count
///         Some("REGISTER"),        // method
///         Some("sip:example.com"), // uri
///         Some("MD5"),             // algorithm
///         None                     // opaque
///     )
///     .build();
///
/// // Step 4: Server validates credentials and sends 200 OK
/// let success_response = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", Some("tag123"))
///     .contact("<sip:alice@192.168.1.2>", None)
///     .build();
/// ```
///
/// ## Advanced Digest Authentication Options
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
///
/// // Challenge with SHA-256 algorithm and both auth and auth-int QoP options
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "sip.example.com",
///         "9FxHwSyJClx391jQKoMl3Z1",
///         Some("secureOpaque8734"), 
///         Some("SHA-256"),            // SHA-256 for improved security
///         Some(vec!["auth", "auth-int"]), // Support both auth types
///         None,
///         Some(vec!["sip:example.com", "sip:voip.example.com"]) // Domain restriction
///     )
///     .build();
///
/// // Challenge with stale=true (indicates nonce expired but credentials may be valid)
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "sip.example.com",
///         "newNonce5349058kdjfd",
///         None,
///         Some("MD5"),
///         Some(vec!["auth"]),
///         Some(true),                 // stale=true - indicates client should retry with new nonce
///         None
///     )
///     .build();
/// ```
///
/// ## SIP Registration Service with Strong Authentication
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::WwwAuthenticateExt;
///
/// // Scenario: SIP Registrar implementation with strong authentication
/// // This example shows how to create a secure challenge for a SIP registration service
///
/// // Function to generate a 401 challenge with best security practices
/// fn create_secure_registration_challenge(request_from_tag: &str) -> SimpleResponseBuilder {
///     // In production, use a cryptographically secure random generator for nonce
///     let nonce = "d5e5ff37c381c489bc858ac968a7c246a729ef76";
///     
///     SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Authentication Required"))
///         .www_authenticate_digest(
///             "registrar.example.com",         // Consistent realm for your domain
///             nonce,                           // Strong random nonce
///             Some("9e14fdca8fe47d13b5"), // Opaque server state value
///             Some("SHA-256"),                 // Modern secure algorithm
///             Some(vec!["auth"]),              // Quality of protection with nonce counting
///             None,                            // Not stale yet
///             Some(vec!["sip:registrar.example.com", "sip:sip.example.com"]) // Domain scope
///         )
///         .from("Alice", "sip:alice@example.com", Some(request_from_tag)) 
///         .to("Alice", "sip:alice@example.com", None)
/// }
///
/// // When a REGISTER request is received, create a challenge response
/// let challenge = create_secure_registration_challenge("a73kszlfl").build();
/// 
/// // The challenge response follows best practices:
/// // 1. Uses SHA-256 instead of MD5
/// // 2. Employs qop="auth" requiring cnonce and nonce counting
/// // 3. Includes domain parameter to limit scope
/// // 4. Provides opaque server state
/// ```
///
/// ## Multiple Authentication Challenges
///
/// While not directly supported by this builder, you can add multiple WWW-Authenticate
/// headers to support different authentication schemes:
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::TypedHeader;
///
/// // Create a response with both Digest and Basic authentication challenges
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
///     .www_authenticate_digest(
///         "sip.example.com",
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093",
///         None,
///         Some("MD5"),
///         Some(vec!["auth"]),
///         None,
///         None
///     )
///     // Add Basic authentication as a fallback
///     .www_authenticate_basic("sip.example.com")
///     .build();
/// ```
pub trait WwwAuthenticateExt {
    /// Add a Digest WWW-Authenticate header to the response
    ///
    /// This method adds a WWW-Authenticate header with a Digest authentication challenge
    /// to a SIP response. This is typically used with 401 Unauthorized responses to
    /// challenge the client to authenticate.
    ///
    /// ## Digest Challenge in SIP
    ///
    /// Digest authentication is the preferred authentication method for SIP as defined in
    /// [RFC 3261 Section 22.4](https://datatracker.ietf.org/doc/html/rfc3261#section-22.4),
    /// which builds upon the HTTP Digest Authentication in [RFC 2617](https://datatracker.ietf.org/doc/html/rfc2617).
    ///
    /// The challenge contains parameters that the client will use to compute a response hash.
    /// This enables secure authentication without transmitting the password over the network.
    ///
    /// ## Parameters
    ///
    /// * `realm` - The authentication realm (mandatory) - identifies the protection domain
    /// * `nonce` - The server nonce value (mandatory) - a server-specified data string
    /// * `opaque` - Optional opaque value that must be returned unchanged in the Authorization header
    /// * `algorithm` - Optional algorithm (defaults to MD5 if None, but SHA-256 is recommended for security)
    /// * `qop` - Optional quality of protection options (auth, auth-int)
    /// * `stale` - Optional stale flag (true if nonce is stale but credentials are valid)
    /// * `domain` - Optional authentication domain (list of URIs that share credentials)
    ///
    /// ## Returns
    ///
    /// The builder with the WWW-Authenticate header added
    ///
    /// ## Examples
    ///
    /// ### Basic Challenge
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // Create a minimal digest challenge
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_digest(
    ///         "sip.example.com",
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
    /// ### Secure Production Challenge
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // Create a more secure challenge for production use
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_digest(
    ///         "secure.example.com",
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // In production, use a cryptographically random value
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // Opaque data for server state
    ///         Some("SHA-256"),                        // More secure than MD5
    ///         Some(vec!["auth"]),                     // Quality of protection
    ///         None,
    ///         Some(vec!["sip:secure.example.com"])    // Limit to specific domain
    ///     )
    ///     .build();
    /// ```
    ///
    /// ### INVITE Challenge with Domain Restriction
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // Challenge for an INVITE request to protect sensitive calling features
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Authentication Required"))
    ///     .www_authenticate_digest(
    ///         "pbx.example.com",
    ///         "a2f3ab7c8d9e0f1a2b3c4d5e", 
    ///         None,
    ///         Some("SHA-256"),
    ///         Some(vec!["auth"]),
    ///         None,
    ///         Some(vec!["sip:pbx.example.com", "sip:voicemail.example.com"]) // Only these services
    ///     )
    ///     .from("Bob", "sip:bob@example.com", Some("invite-1"))  // Echo From 
    ///     .to("Service", "sip:service@pbx.example.com", None)   // Echo To
    ///     .build();
    /// ```
    ///
    /// ### Handling Nonce Expiration
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // After nonce timeout, send a new challenge with stale=true
    /// // This tells the client their credentials might be valid but nonce has expired
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Nonce Expired"))
    ///     .www_authenticate_digest(
    ///         "sip.example.com",
    ///         "fresh45nonce89value12", // New nonce value
    ///         None,
    ///         Some("MD5"),
    ///         Some(vec!["auth"]),
    ///         Some(true),             // stale=true indicates valid credentials but expired nonce
    ///         None
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
    /// it may be used in simple scenarios or for legacy compatibility.
    ///
    /// ## Basic Authentication in SIP
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
    /// and is generally not recommended for SIP authentication. Digest authentication
    /// provides much better security.
    ///
    /// ## Parameters
    ///
    /// * `realm` - The authentication realm (protection domain)
    ///
    /// ## Returns
    ///
    /// The builder with the WWW-Authenticate header added
    ///
    /// ## Examples
    ///
    /// ### Simple Basic Challenge
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // Create a basic authentication challenge
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_basic("sip.example.com")
    ///     .build();
    /// ```
    ///
    /// ### Using with TLS for Better Security
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // When used over TLS, Basic authentication is somewhat more secure
    /// // This should only be used when Digest authentication is not an option
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, None)
    ///     .www_authenticate_basic("sips.example.com") // Note: using SIPS realm to indicate TLS usage
    ///     .build();
    /// 
    /// // The response should be sent over a TLS connection
    /// ```
    ///
    /// ### In a Simple SIPS Environment
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// // Basic authentication for a small office with TLS
    /// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Authentication Required"))
    ///     .from("PBX", "sips:pbx@example.com", Some("pbx-challenge"))
    ///     .to("User", "sips:user@example.com", None)
    ///     .www_authenticate_basic("secure-office-pbx")
    ///     .build();
    ///
    /// // This is only appropriate for small deployments with TLS
    /// // where simplicity is preferred over security strength
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