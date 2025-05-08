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

/// Authorization header builder
///
/// This module provides builder methods for adding Authorization headers to SIP requests,
/// used for providing authentication credentials as defined in RFC 3261 Section 22.
///
/// ## SIP Authentication Overview
///
/// SIP authentication is based on the HTTP authentication framework defined in
/// [RFC 2617](https://datatracker.ietf.org/doc/html/rfc2617) and later 
/// [RFC 7616](https://datatracker.ietf.org/doc/html/rfc7616), adapted for SIP in
/// [RFC 3261 Section 22](https://datatracker.ietf.org/doc/html/rfc3261#section-22).
/// Authorization occurs primarily when registering with a registrar, accessing resources
/// through a proxy, or when making requests to secured endpoints.
///
/// ## Authentication Flow
///
/// 1. Client sends a request without credentials
/// 2. Server responds with 401 Unauthorized (registrar/UAS) or 407 Proxy Authentication Required (proxy)
/// 3. Server includes a WWW-Authenticate or Proxy-Authenticate header with challenge parameters
/// 4. Client computes a response using the challenge, credentials, and request information
/// 5. Client resends request with Authorization or Proxy-Authorization header including the response
/// 6. Server validates the credentials and grants access if valid
///
/// ## Types of Authentication
///
/// - **Digest Authentication**: The primary authentication method in SIP, providing a challenge-response
///   mechanism that avoids sending passwords in plaintext
/// - **Basic Authentication**: A simple authentication method using Base64-encoded username:password,
///   which should only be used over secure transports like TLS
///
/// ## Common Digest Authentication Parameters
///
/// - **username**: The user being authenticated
/// - **realm**: Authentication domain
/// - **nonce**: Server-generated unique challenge string
/// - **response**: Hash computed from credentials and challenge
/// - **uri**: The request URI, used in hash computation
/// - **algorithm**: Hash algorithm (e.g., MD5, SHA-256)
/// - **qop** (Quality of Protection): Indicates authentication quality level (auth, auth-int)
/// - **cnonce**: Client-generated nonce for replay protection
/// - **nc** (Nonce Count): Counter for nonce reuse tracking
/// - **opaque**: Server data to be returned unchanged
///
/// ## Relationship with other headers
///
/// - **Authorization** vs **WWW-Authenticate**: WWW-Authenticate presents the challenge from a UAS,
///   Authorization provides the response to that challenge
/// - **Authorization** vs **Proxy-Authorization**: Authorization responds to endpoint challenges,
///   Proxy-Authorization responds to proxy challenges
/// - **Authorization** vs **Authentication-Info**: Authorization is in requests from clients,
///   Authentication-Info provides post-authentication data in responses
///
/// ## Security Considerations
///
/// - Digest authentication should use strong algorithms like SHA-256 when possible
/// - Basic authentication should only be used over TLS connections
/// - Clients should validate the realm to prevent man-in-the-middle attacks
/// - Mutual authentication (server authenticating to client) is possible with auth-int qop
/// - Replay protection requires proper handling of nonce/cnonce values and nonce counts
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
///
/// // Create a request with a Digest Authorization
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .authorization_digest(
///         "alice",                 // username
///         "sip.example.com",       // realm
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
///         "5ccc069c403ebaf9f0171e9517f40e41",   // response
///         Some("0a4f113b"),        // cnonce 
///         Some("auth"),            // qop
///         Some("00000001"),        // nc
///         Some("REGISTER"),        // method
///         Some("sip:example.com"), // uri
///         Some("MD5"),             // algorithm
///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
///     )
///     .build();
///
/// // Create a request with a Basic Authorization
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .authorization_basic("alice", "password123")
///     .build();
/// ```
///
/// # Important Note on Parameter Order
///
/// The parameter order in `authorization_digest` and `proxy_authorization_digest` (from ProxyAuthorizationExt) 
/// methods are *not* identical. Pay close attention to the parameter order when using these methods:
///
/// - In `authorization_digest`: the `response` parameter comes *before* the optional parameters like cnonce/qop
/// - In `proxy_authorization_digest`: the `uri` parameter comes *before* the `response` parameter
///
/// These differences exist for historical reasons in the implementation. Always refer to the method
/// signatures and examples to ensure correct parameter ordering.
///
/// # Examples
///
/// ## Responding to a 401 Unauthorized Challenge
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
/// use std::str::FromStr;
///
/// // First, we receive a 401 response with a WWW-Authenticate header
/// let www_auth_header = "Digest realm=\"sip.example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", \
///                         algorithm=MD5, qop=\"auth\"";
/// 
/// // Parse the response to extract the challenge parameters
/// let response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Unauthorized"))
///     .header(TypedHeader::WwwAuthenticate(WwwAuthenticate::from_str(www_auth_header).unwrap()))
///     .build();
///
/// // Now construct a new request with the proper authorization
/// // In a real scenario, we would compute the response hash using the challenge,
/// // credentials, and requested URI according to RFC 2617/7616
/// let computed_response = "5ccc069c403ebaf9f0171e9517f40e41"; // MD5(username:realm:password)
/// 
/// let authenticated_request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .authorization_digest(
///         "alice",                 // username 
///         "sip.example.com",       // realm (from challenge)
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce (from challenge)
///         computed_response,       // the computed response hash
///         Some("0a4f113b"),        // client nonce (required for qop=auth)
///         Some("auth"),            // qop (from challenge)
///         Some("00000001"),        // nonce count (starts at 1)
///         Some("REGISTER"),        // method (for proper hash computation)
///         Some("sip:example.com"), // uri (for proper hash computation)
///         Some("MD5"),             // algorithm (from challenge)
///         None,                    // opaque (would be from challenge if present)
///     )
///     .build();
/// ```
///
/// ## Full SIP Registration Flow with Authentication
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::CSeqBuilderExt;
/// use std::str::FromStr;
///
/// // Step 1: Send initial REGISTER request (no authorization)
/// let initial_register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2>;expires=3600", None)
///     .cseq_with_method(1, Method::Register)
///     .build();
///
/// // Step 2: Receive 401 Unauthorized with challenge
/// let challenge_response = SimpleResponseBuilder::new(StatusCode::Unauthorized, Some("Unauthorized"))
///     .header(TypedHeader::WwwAuthenticate(WwwAuthenticate::from_str(
///         "Digest realm=\"sip.example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", \
///          algorithm=MD5, qop=\"auth\"").unwrap()))
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Echo From header
///     .to("Alice", "sip:alice@example.com", None)                // Echo To header
///     .cseq_with_method(1, Method::Register)                     // Echo CSeq
///     .build();
///
/// // Step 3: Send authenticated REGISTER request
/// // In production code, compute the digest response using A1=username:realm:password and 
/// // A2=method:uri as defined in RFC 2617/7616
/// let computed_response = "5ccc069c403ebaf9f0171e9517f40e41"; 
///
/// let authenticated_register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Alice", "sip:alice@example.com", None)
///     .contact("<sip:alice@192.168.1.2>;expires=3600", None)
///     .cseq_with_method(2, Method::Register)  // Increment CSeq for new request
///     .authorization_digest(
///         "alice",                 // username
///         "sip.example.com",       // realm (from challenge)
///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce (from challenge)
///         computed_response,       // the computed response hash
///         Some("0a4f113b"),        // client nonce 
///         Some("auth"),            // qop (from challenge)
///         Some("00000001"),        // nonce count (starts at 1)
///         Some("REGISTER"),        // method
///         Some("sip:example.com"), // uri
///         Some("MD5"),             // algorithm (from challenge)
///         None,                    // opaque (not in challenge)
///     )
///     .build();
///
/// // Step 4: Receive 200 OK response (registration successful)
/// let success_response = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl")) // Echo From
///     .to("Alice", "sip:alice@example.com", Some("tag123"))     // Echo To, add tag
///     .cseq_with_method(2, Method::Register)                    // Echo CSeq
///     .contact("<sip:alice@192.168.1.2>;expires=3600", None)    // Confirm registration
///     .build();
/// ```
///
/// ## Using with Proxy Authentication
///
/// For proxy authentication (responding to 407 Proxy Authentication Required),
/// use the ProxyAuthorizationExt trait from the proxy_authorization module:
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::{ProxyAuthorizationExt, ContentTypeBuilderExt};
/// use std::str::FromStr;
///
/// // Scenario: Receive a 407 Proxy Authentication Required
/// let proxy_auth_response = SimpleResponseBuilder::new(StatusCode::ProxyAuthenticationRequired, Some("Proxy Authentication Required"))
///     .header(TypedHeader::ProxyAuthenticate(ProxyAuthenticate::from_str(
///         "Digest realm=\"proxy.example.com\", nonce=\"3ba1f67c4c2229b3a5fd\", algorithm=SHA-256, qop=\"auth\"").unwrap()))
///     .build();
///
/// // Compute proxy authentication response (in real code, this would be a proper hash)
/// let proxy_auth_hash = "7c1d357bec28ae9f4d800967legab276";
///
/// // Create INVITE request with proxy authentication
/// let invite_with_auth = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.net").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
///     .to("Bob", "sip:bob@example.net", None)
///     .contact("<sip:alice@192.168.1.2>", None)
///     // Use proxy_authorization_digest from ProxyAuthorizationExt trait
///     // NOTE: Parameter order is different from authorization_digest!
///     // Order: username, realm, nonce, uri, response, algorithm, cnonce, opaque, qop, nc
///     .proxy_authorization_digest(
///         "alice",                  // username
///         "proxy.example.com",      // realm
///         "3ba1f67c4c2229b3a5fd",   // nonce
///         "sip:bob@example.net",    // uri (NOTE: comes BEFORE response)
///         proxy_auth_hash,          // response
///         Some("SHA-256"),          // algorithm 
///         Some("8f5666ab"),         // cnonce
///         None,                     // opaque
///         Some("auth"),             // qop
///         Some("00000001")          // nc
///     )
///     .content_type_sdp()
///     .body(concat!(
///         "v=0\r\n",
///         "o=alice 2890844526 2890844526 IN IP4 192.168.1.2\r\n",
///         "s=Call with Alice\r\n",
///         "c=IN IP4 192.168.1.2\r\n",
///         "t=0 0\r\n",
///         "m=audio 49170 RTP/AVP 0 8\r\n",
///         "a=rtpmap:0 PCMU/8000\r\n",
///         "a=rtpmap:8 PCMA/8000\r\n"
///     ))
///     .build();
/// ```
///
/// ## Basic Authentication
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
///
/// // Create a request with Basic Authentication (less common in SIP)
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", None)
///     .to("Alice", "sip:alice@example.com", None)
///     .authorization_basic("alice", "password123")  // username:password base64 encoded
///     .build();
///
/// // Note: Basic authentication is generally not recommended for SIP as it 
/// // transmits credentials with minimal protection. Digest authentication
/// // is much more common and secure in SIP deployments.
/// ```
pub trait AuthorizationExt {
    /// Add a Digest Authorization header to the request
    ///
    /// This method adds an Authorization header with a Digest authentication response
    /// to a SIP request. This is typically used in response to a 401 Unauthorized challenge.
    ///
    /// ## Digest Authentication in SIP
    ///
    /// Digest authentication is the primary authentication mechanism in SIP, defined in 
    /// [RFC 3261 Section 22.4](https://datatracker.ietf.org/doc/html/rfc3261#section-22.4).
    /// It uses a challenge-response mechanism where:
    ///
    /// 1. The server provides a challenge in a WWW-Authenticate header
    /// 2. The client computes a response hash using the challenge, credentials, and request details
    /// 3. The client sends the response in an Authorization header
    /// 
    /// The digest response calculation depends on several parameters and the chosen hash algorithm.
    /// For MD5 with qop=auth (most common in legacy systems):
    ///
    /// - A1 = username:realm:password
    /// - A2 = method:uri
    /// - response = MD5(MD5(A1):nonce:nc:cnonce:qop:MD5(A2))
    ///
    /// For SHA-256 and newer systems, the calculation is similar but uses SHA-256 instead of MD5.
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
    /// # Important Note on Parameter Order
    ///
    /// The parameter order must be followed exactly as shown. This is especially important
    /// when comparing with `proxy_authorization_digest` which has different parameter ordering.
    /// If parameters are provided in the wrong order, authentication will fail or behave unexpectedly.
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// // Create a request with full Digest Authorization parameters
    /// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .authorization_digest(
    ///         "alice",                 // username
    ///         "sip.example.com",       // realm
    ///         "dcd98b7102dd2f0e8b11d0f600bfb0c093", // nonce
    ///         "5ccc069c403ebaf9f0171e9517f40e41",   // response
    ///         Some("0a4f113b"),        // cnonce (required for qop=auth)
    ///         Some("auth"),            // qop 
    ///         Some("00000001"),        // nonce count (required for qop=auth)
    ///         Some("REGISTER"),        // method used in hash calculation
    ///         Some("sip:example.com"), // uri used in hash calculation
    ///         Some("MD5"),             // algorithm
    ///         Some("5ccc069c403ebaf9f0171e9517f40e41"), // opaque
    ///     )
    ///     .build();
    /// ```
    ///
    /// # Common Algorithm Choices
    ///
    /// - **MD5**: The original algorithm from RFC 2617, widely supported but less secure
    /// - **MD5-sess**: MD5 with session variations for improved security
    /// - **SHA-256**: Modern secure hash algorithm, recommended for new implementations
    /// - **SHA-256-sess**: SHA-256 with session variations
    /// - **SHA-512-256**: Even stronger hash algorithm for high-security environments
    ///
    /// # Note on Nonce Count
    ///
    /// The nonce count (nc) is a hexadecimal counter that increases with each request using
    /// the same nonce. It starts at 00000001 and should be incremented for each request using
    /// the same nonce value. This prevents replay attacks.
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
    /// ## Basic Authentication in SIP
    ///
    /// Basic authentication is defined in [RFC 7617](https://datatracker.ietf.org/doc/html/rfc7617)
    /// for HTTP and adapted for SIP. It works by:
    ///
    /// 1. Concatenating username and password with a colon separator: `username:password`
    /// 2. Base64-encoding this string
    /// 3. Adding it to the Authorization header as `Authorization: Basic {encoded-string}`
    ///
    /// Because the password is sent with only Base64 encoding (which is trivially reversed),
    /// Basic authentication should only be used over secure transport like TLS.
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
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// // Create a REGISTER request with Basic Authentication over TLS
    /// let request = SimpleRequestBuilder::new(Method::Register, "sips:secure.example.com").unwrap()
    ///     .from("Alice", "sips:alice@secure.example.com", Some("reg-1"))
    ///     .to("Alice", "sips:alice@secure.example.com", None)
    ///     .contact("<sips:alice@192.0.2.1:5061>", None)
    ///     // Add a Basic authentication header
    ///     .authorization_basic("alice", "password123")
    ///     .build();
    ///
    /// // Note: This is sending a request to a SIPS URI (SIP over TLS),
    /// // which provides the transport security needed for Basic authentication
    /// ```
    ///
    /// # Security Considerations
    ///
    /// Basic authentication sends credentials with minimal protection (only base64 encoding,
    /// which is trivial to decode). It should only be used over secure connections (like TLS)
    /// and is generally not recommended for SIP authentication. Digest authentication
    /// provides much better security for insecure transports.
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
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
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
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
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