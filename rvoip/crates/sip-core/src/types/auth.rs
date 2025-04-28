//! # SIP Authentication
//! 
//! This module provides types for handling SIP authentication as defined in 
//! [RFC 3261 Section 22](https://datatracker.ietf.org/doc/html/rfc3261#section-22) and
//! [RFC 7616](https://datatracker.ietf.org/doc/html/rfc7616) (HTTP Digest Authentication).
//!
//! SIP authentication is primarily based on HTTP Digest Authentication, although Basic 
//! authentication can also be used. The authentication flow typically involves:
//!
//! 1. Client sends a request
//! 2. Server responds with a 401 (Unauthorized) or 407 (Proxy Authentication Required)
//!    containing a challenge in the WWW-Authenticate or Proxy-Authenticate header
//! 3. Client calculates a response digest and sends a new request with credentials
//!    in the Authorization or Proxy-Authorization header
//! 4. Server verifies the credentials and processes the request if valid
//!
//! ## Headers
//!
//! The following authentication-related headers are implemented:
//!
//! - **WWW-Authenticate**: Used by servers to issue authentication challenges
//! - **Authorization**: Used by clients to provide authentication credentials
//! - **Proxy-Authenticate**: Used by proxy servers to issue authentication challenges
//! - **Proxy-Authorization**: Used by clients to provide credentials to proxy servers
//! - **Authentication-Info**: Used by servers to provide authentication information after successful authentication
//!
//! ## Authentication Schemes
//!
//! Two authentication schemes are commonly used in SIP:
//!
//! - **Digest**: The recommended scheme, using challenge-response with various algorithms
//! - **Basic**: Simple Base64-encoded username:password (less secure, not recommended)
//!
//! ## Example
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Parse a WWW-Authenticate header
//! let www_auth = WwwAuthenticate::from_str(
//!     "Digest realm=\"example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", algorithm=MD5"
//! ).unwrap();
//!
//! // Create an Authorization header in response
//! let uri = Uri::from_str("sip:example.com").unwrap();
//! let auth = Authorization::new(
//!     AuthScheme::Digest,
//!     "user",
//!     "example.com",
//!     "dcd98b7102dd2f0e8b11d0f600bfb0c093",
//!     uri,
//!     "31d6cfe0d16ae931b73c59d7e0c089c0" // MD5 hash of credentials
//! ).with_algorithm(Algorithm::Md5);
//! ```

use crate::types::uri::Uri;
use std::collections::HashMap;
use std::fmt;
use crate::error::{Result, Error};
use std::str::FromStr;
use serde::{Deserialize, Serialize};
use crate::types::method::Method;

/// Authentication Scheme (Digest, Basic, etc.)
///
/// SIP authentication can use different schemes, with Digest being the recommended
/// approach in RFC 3261. Basic authentication is less secure and generally not recommended
/// for production use.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// let digest = AuthScheme::from_str("Digest").unwrap();
/// assert_eq!(digest, AuthScheme::Digest);
///
/// let basic = AuthScheme::from_str("Basic").unwrap();
/// assert_eq!(basic, AuthScheme::Basic);
///
/// let other = AuthScheme::from_str("NTLM").unwrap();
/// assert_eq!(other, AuthScheme::Other("NTLM".to_string()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthScheme {
    /// Digest authentication (RFC 3261 and RFC 7616)
    Digest,
    /// Basic authentication (username:password encoded in Base64)
    Basic, // Less common in SIP, but possible
    /// Other authentication schemes
    Other(String),
}

impl fmt::Display for AuthScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthScheme::Digest => write!(f, "Digest"),
            AuthScheme::Basic => write!(f, "Basic"),
            AuthScheme::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for AuthScheme {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "digest" => Ok(AuthScheme::Digest),
            "basic" => Ok(AuthScheme::Basic),
            _ if !s.is_empty() => Ok(AuthScheme::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty scheme name".to_string())),
        }
    }
}

/// Digest Algorithm (MD5, SHA-256, etc.)
///
/// Specifies the algorithm used for calculating digest hashes in SIP authentication.
/// MD5 is the original algorithm from RFC 2617, but newer algorithms like SHA-256
/// are recommended for better security as defined in RFC 7616.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// let md5 = Algorithm::from_str("MD5").unwrap();
/// assert_eq!(md5, Algorithm::Md5);
///
/// let sha256 = Algorithm::from_str("SHA-256").unwrap();
/// assert_eq!(sha256, Algorithm::Sha256);
///
/// let md5_sess = Algorithm::from_str("MD5-sess").unwrap();
/// assert_eq!(md5_sess, Algorithm::Md5Sess);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Algorithm {
    /// MD5 algorithm (RFC 2617)
    Md5,
    /// MD5 with session data (RFC 2617)
    Md5Sess,
    /// SHA-256 algorithm (RFC 7616)
    Sha256,
    /// SHA-256 with session data (RFC 7616)
    Sha256Sess,
    /// SHA-512 algorithm (RFC 5687)
    Sha512,
    /// SHA-512 with session data
    Sha512Sess,
    /// Other algorithms
    Other(String),
}

impl fmt::Display for Algorithm {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Algorithm::Md5 => write!(f, "MD5"),
            Algorithm::Md5Sess => write!(f, "MD5-sess"),
            Algorithm::Sha256 => write!(f, "SHA-256"),
            Algorithm::Sha256Sess => write!(f, "SHA-256-sess"),
            Algorithm::Sha512 => write!(f, "SHA-512-256"), // Note: RFC 7616 uses SHA-512-256
            Algorithm::Sha512Sess => write!(f, "SHA-512-256-sess"),
            Algorithm::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for Algorithm {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "md5" => Ok(Algorithm::Md5),
            "md5-sess" => Ok(Algorithm::Md5Sess),
            "sha-256" => Ok(Algorithm::Sha256),
            "sha-256-sess" => Ok(Algorithm::Sha256Sess),
            "sha-512-256" => Ok(Algorithm::Sha512),
            "sha-512-256-sess" => Ok(Algorithm::Sha512Sess),
            _ if !s.is_empty() => Ok(Algorithm::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty algorithm name".to_string())),
        }
    }
}

/// Quality of Protection (auth, auth-int)
///
/// Specifies the quality of protection for HTTP Digest Authentication as defined
/// in RFC 7616. The `auth` quality means authentication only, while `auth-int`
/// also provides integrity protection for the message body.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// let auth = Qop::from_str("auth").unwrap();
/// assert_eq!(auth, Qop::Auth);
///
/// let auth_int = Qop::from_str("auth-int").unwrap();
/// assert_eq!(auth_int, Qop::AuthInt);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Qop {
    /// Authentication only
    Auth,
    /// Authentication with message integrity protection
    AuthInt,
    /// Other QOP values
    Other(String),
}

impl fmt::Display for Qop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Qop::Auth => write!(f, "auth"),
            Qop::AuthInt => write!(f, "auth-int"),
            Qop::Other(s) => write!(f, "{}", s),
        }
    }
}

impl FromStr for Qop {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "auth" => Ok(Qop::Auth),
            "auth-int" => Ok(Qop::AuthInt),
            _ if !s.is_empty() => Ok(Qop::Other(s.to_string())),
            _ => Err(crate::error::Error::InvalidInput("Empty qop value".to_string())),
        }
    }
}

/// Generic Authentication Parameter (name=value)
///
/// Represents a generic name-value parameter used in SIP authentication headers.
/// Parameters are typically presented as `name="value"` pairs in header fields.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::types::auth::AuthParam;
///
/// let param = AuthParam {
///     name: "realm".to_string(),
///     value: "example.com".to_string()
/// };
///
/// assert_eq!(param.to_string(), "realm=\"example.com\"");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthParam {
    /// Parameter name
    pub name: String,
    /// Parameter value
    pub value: String, // Consider storing raw bytes if unquoting is complex
}

impl fmt::Display for AuthParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}=\"{}\"", self.name, self.value)
    }
}

/// Parameters specific to Digest authentication (used in Challenge and Credentials)
///
/// This enum represents the various parameters that can appear in Digest authentication
/// challenges and credentials as defined in RFC 3261 and RFC 7616.
///
/// Different parameters are used depending on whether they appear in:
/// - Server-issued challenges (WWW-Authenticate/Proxy-Authenticate headers)
/// - Client-provided credentials (Authorization/Proxy-Authorization headers)
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a realm parameter
/// let realm = DigestParam::Realm("example.com".to_string());
/// assert_eq!(realm.to_string(), "realm=\"example.com\"");
///
/// // Create a nonce parameter
/// let nonce = DigestParam::Nonce("1234abcd".to_string());
/// assert_eq!(nonce.to_string(), "nonce=\"1234abcd\"");
///
/// // Create an algorithm parameter
/// let algo = DigestParam::Algorithm(Algorithm::Md5);
/// assert_eq!(algo.to_string(), "algorithm=MD5");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigestParam {
    // Challenge & Credentials
    /// Authentication realm (mandatory in challenges and credentials)
    Realm(String),
    /// Server-generated nonce (mandatory in challenges and credentials)
    Nonce(String),
    /// Opaque data from server (Optional in challenge, MUST be returned if present)
    Opaque(String), 
    /// Hashing algorithm (Optional in both challenge and credentials)
    Algorithm(Algorithm), 
    // Challenge Only
    /// List of URIs that share credentials (Optional in challenges)
    Domain(Vec<String>), 
    /// Indicates if the nonce is stale (Optional in challenges)
    Stale(bool),       
    /// Quality of protection options (Optional in challenges)
    Qop(Vec<Qop>),     
    // Credentials Only
    /// User's username (Mandatory in credentials)
    Username(String),
    /// Request URI (Mandatory in credentials)
    Uri(Uri),
    /// Digest response hash (Mandatory in credentials)
    Response(String), 
    /// Client nonce (Mandatory if QOP is used)
    Cnonce(String),   
    /// Quality of protection used (Mandatory if QOP is offered)
    MsgQop(Qop),      
    /// Nonce count (Mandatory if QOP is used)
    NonceCount(u32),  
    // Generic fallback
    /// Generic parameter not specifically typed above
    Param(AuthParam),
}

impl fmt::Display for DigestParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DigestParam::Realm(v) => write!(f, "realm=\"{}\"", v),
            DigestParam::Nonce(v) => write!(f, "nonce=\"{}\"", v),
            DigestParam::Opaque(v) => write!(f, "opaque=\"{}\"", v),
            DigestParam::Algorithm(v) => write!(f, "algorithm={}", v),
            DigestParam::Domain(v) => write!(f, "domain=\"{}\"", v.join(", ")),
            DigestParam::Stale(v) => write!(f, "stale={}", v),
            DigestParam::Qop(v) => write!(f, "qop={}", v.iter().map(|q| q.to_string()).collect::<Vec<_>>().join(",")),
            DigestParam::Username(v) => write!(f, "username=\"{}\"", v),
            DigestParam::Uri(v) => write!(f, "uri=\"{}\"", v),
            DigestParam::Response(v) => write!(f, "response=\"{}\"", v),
            DigestParam::Cnonce(v) => write!(f, "cnonce=\"{}\"", v),
            DigestParam::MsgQop(v) => write!(f, "qop={}", v),
            DigestParam::NonceCount(v) => write!(f, "nc={:08x}", v),
            DigestParam::Param(p) => write!(f, "{}", p),
        }
    }
}

/// Parameters specific to Authentication-Info header
///
/// These parameters are used in the Authentication-Info header field, which is sent by
/// servers after successful authentication to provide additional information to clients.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a next-nonce parameter
/// let next_nonce = AuthenticationInfoParam::NextNonce("5678efgh".to_string());
/// assert_eq!(next_nonce.to_string(), "nextnonce=\"5678efgh\"");
///
/// // Create a response-auth parameter
/// let rspauth = AuthenticationInfoParam::ResponseAuth("abcdef1234567890".to_string());
/// assert_eq!(rspauth.to_string(), "rspauth=\"abcdef1234567890\"");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthenticationInfoParam {
    /// Next nonce to be used by the client
    NextNonce(String),
    /// Quality of protection used
    Qop(Qop), // Only one value allowed
    /// Server authentication response (mutual authentication)
    ResponseAuth(String), // rspauth (hex)
    /// Client nonce (echoed from the client's request)
    Cnonce(String),
    /// Nonce count (echoed from the client's request)
    NonceCount(u32), // nc-value (hex, parsed to u32)
}

impl fmt::Display for AuthenticationInfoParam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthenticationInfoParam::NextNonce(v) => write!(f, "nextnonce=\"{}\"", v),
            AuthenticationInfoParam::Qop(v) => write!(f, "qop={}", v),
            AuthenticationInfoParam::ResponseAuth(v) => write!(f, "rspauth=\"{}\"", v),
            AuthenticationInfoParam::Cnonce(v) => write!(f, "cnonce=\"{}\"", v),
            AuthenticationInfoParam::NonceCount(v) => write!(f, "nc={:08x}", v),
        }
    }
}

/// Represents a challenge (WWW-Authenticate, Proxy-Authenticate)
///
/// A challenge is sent by a server in 401 Unauthorized or 407 Proxy Authentication Required
/// responses to request authentication from a client. Challenges can use different
/// authentication schemes, with Digest being the most common in SIP.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a Digest challenge
/// let challenge = Challenge::Digest {
///     params: vec![
///         DigestParam::Realm("example.com".to_string()),
///         DigestParam::Nonce("1234abcd".to_string()),
///         DigestParam::Algorithm(Algorithm::Md5)
///     ]
/// };
///
/// // Create a Basic challenge
/// let basic_challenge = Challenge::Basic {
///     params: vec![
///         AuthParam {
///             name: "realm".to_string(),
///             value: "example.com".to_string()
///         }
///     ]
/// };
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Challenge {
    /// Digest authentication challenge with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication challenge (typically just realm)
    Basic { params: Vec<AuthParam> }, // Typically just realm
    /// Other authentication scheme challenges
    Other { scheme: String, params: Vec<AuthParam> },
}

impl fmt::Display for Challenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Challenge::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            },
            Challenge::Basic { params } => {
                 write!(f, "Basic ")?;
                 let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                 write!(f, "{}", params_str)
            },
            Challenge::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}

/// Represents credentials (Authorization, Proxy-Authorization)
///
/// Credentials are sent by clients in response to authentication challenges. They
/// contain the information needed for the server to authenticate the client.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create Digest credentials
/// let uri = Uri::from_str("sip:example.com").unwrap();
/// let credentials = Credentials::Digest {
///     params: vec![
///         DigestParam::Username("bob".to_string()),
///         DigestParam::Realm("example.com".to_string()),
///         DigestParam::Nonce("1234abcd".to_string()),
///         DigestParam::Uri(uri),
///         DigestParam::Response("5678efgh".to_string()),
///         DigestParam::Algorithm(Algorithm::Md5)
///     ]
/// };
///
/// // Create Basic credentials
/// let basic = Credentials::Basic {
///     token: "Ym9iOnBhc3N3b3Jk".to_string()  // Base64 of "bob:password"
/// };
///
/// // Check credential type
/// assert!(credentials.is_digest());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Credentials {
    /// Digest authentication credentials with associated parameters
    Digest { params: Vec<DigestParam> },
    /// Basic authentication credentials (Base64 encoded "username:password")
    Basic { token: String }, // Base64 encoded "userid:password"
    /// Other authentication scheme credentials
    Other { scheme: String, params: Vec<AuthParam> },
}

impl Credentials {
    /// Returns true if the credentials are of the Digest type
    ///
    /// # Returns
    ///
    /// `true` if these are Digest credentials, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let digest = Credentials::Digest {
    ///     params: vec![
    ///         DigestParam::Username("bob".to_string()),
    ///         DigestParam::Realm("example.com".to_string()),
    ///         DigestParam::Nonce("1234abcd".to_string()),
    ///         DigestParam::Uri(uri),
    ///         DigestParam::Response("5678efgh".to_string())
    ///     ]
    /// };
    ///
    /// let basic = Credentials::Basic {
    ///     token: "Ym9iOnBhc3N3b3Jk".to_string()
    /// };
    ///
    /// assert!(digest.is_digest());
    /// assert!(!basic.is_digest());
    /// ```
    pub fn is_digest(&self) -> bool {
        matches!(self, Credentials::Digest { .. })
    }
}

impl fmt::Display for Credentials {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credentials::Digest { params } => {
                write!(f, "Digest ")?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            },
             Credentials::Basic { token } => {
                 write!(f, "Basic {}", token)
            },
            Credentials::Other { scheme, params } => {
                write!(f, "{} ", scheme)?;
                let params_str = params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
                write!(f, "{}", params_str)
            }
        }
    }
}

/// Typed WWW-Authenticate header.
///
/// The WWW-Authenticate header is used in 401 Unauthorized responses to challenge the client
/// to authenticate itself. It can contain multiple challenges using different authentication
/// schemes, allowing the client to choose the most appropriate one.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a WWW-Authenticate header with a Digest challenge
/// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
///     .with_algorithm(Algorithm::Md5)
///     .with_qop(Qop::Auth);
///
/// // Parse from a string
/// let www_auth = WwwAuthenticate::from_str(
///     "Digest realm=\"example.com\", nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\", algorithm=MD5"
/// ).unwrap();
///
/// // Get the first Digest challenge
/// let digest_challenge = www_auth.first_digest();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WwwAuthenticate(pub Vec<Challenge>); // Holds multiple Challenge enums

impl fmt::Display for WwwAuthenticate {
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

impl WwwAuthenticate {
    /// Creates a new WwwAuthenticate header with a single Digest challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (e.g., domain name)
    /// - `nonce`: A server-generated unique nonce value
    ///
    /// # Returns
    ///
    /// A new WWW-Authenticate header with a Digest challenge containing the 
    /// specified realm and nonce
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd");
    /// ```
    pub fn new(realm: impl Into<String>, nonce: impl Into<String>) -> Self {
        Self(vec![Challenge::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
        ] }])
    }

    /// Creates a new WwwAuthenticate header with a Basic challenge.
    ///
    /// # Parameters
    ///
    /// - `realm`: The authentication realm (e.g., domain name)
    ///
    /// # Returns
    ///
    /// A new WWW-Authenticate header with a Basic challenge containing the
    /// specified realm
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new_basic("example.com");
    /// ```
    pub fn new_basic(realm: impl Into<String>) -> Self {
        Self(vec![Challenge::Basic { params: vec![
            AuthParam { name: "realm".to_string(), value: realm.into() }
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
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut www_auth = WwwAuthenticate::new("example.com", "1234abcd");
    ///
    /// // Add a Basic challenge as an alternative
    /// www_auth.add_challenge(Challenge::Basic {
    ///     params: vec![
    ///         AuthParam { name: "realm".to_string(), value: "example.com".to_string() }
    ///     ]
    /// });
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd");
    ///
    /// if let Some(digest) = www_auth.first_digest() {
    ///     // Handle Digest challenge
    /// }
    /// ```
    pub fn first_digest(&self) -> Option<&Challenge> {
        self.0.iter().find(|c| matches!(c, Challenge::Digest { .. }))
    }

    /// Returns the first Basic challenge, if any.
    ///
    /// # Returns
    ///
    /// An Option containing a reference to the first Basic challenge,
    /// or None if no Basic challenge is present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new_basic("example.com");
    ///
    /// if let Some(basic) = www_auth.first_basic() {
    ///     // Handle Basic challenge
    /// }
    /// ```
    pub fn first_basic(&self) -> Option<&Challenge> {
        self.0.iter().find(|c| matches!(c, Challenge::Basic { .. }))
    }

    /// Sets the domain parameter on the first Digest challenge.
    ///
    /// The domain parameter specifies a list of URIs that share the same
    /// authentication information.
    ///
    /// # Parameters
    ///
    /// - `domain`: The domain URI to add
    ///
    /// # Returns
    ///
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
    ///     .with_domain("sip:example.com");
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
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
    ///     .with_opaque("5678efgh");
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
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
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
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
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
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
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
    /// The modified WWW-Authenticate header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let www_auth = WwwAuthenticate::new("example.com", "1234abcd")
    ///     .with_qops(vec![Qop::Auth, Qop::AuthInt]);
    /// ```
    pub fn with_qops(mut self, qops: Vec<Qop>) -> Self {
        if let Some(Challenge::Digest { ref mut params }) = self.0.first_mut().filter(|c| matches!(c, Challenge::Digest { .. })) {
            params.push(DigestParam::Qop(qops));
        }
        self
    }
}

impl FromStr for WwwAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
         crate::parser::headers::parse_www_authenticate(s.as_bytes())
             .map(|(_, challenges)| WwwAuthenticate(challenges))
             .map_err(Error::from) // Convert nom::Err to our Error type
    }
}

/// Typed Authorization header.
///
/// The Authorization header is used by clients to provide authentication credentials
/// in response to a WWW-Authenticate challenge. It typically contains the necessary
/// information for the server to verify the client's identity.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create an Authorization header with Digest credentials
/// let uri = Uri::from_str("sip:example.com").unwrap();
/// let auth = Authorization::new(
///     AuthScheme::Digest,
///     "bob",
///     "example.com",
///     "1234abcd",
///     uri,
///     "5678efgh"
/// ).with_algorithm(Algorithm::Md5)
///  .with_qop(Qop::Auth)
///  .with_cnonce("87654321");
///
/// // Parse from a string
/// let auth = Authorization::from_str(
///     "Digest username=\"bob\", realm=\"example.com\", nonce=\"1234abcd\", \
///      uri=\"sip:example.com\", response=\"5678efgh\", algorithm=MD5"
/// ).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authorization(pub Credentials); // Holds the Credentials enum directly

impl fmt::Display for Authorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to Credentials' Display
    }
}

impl Authorization {
    /// Creates a new Authorization header with mandatory fields.
    ///
    /// # Parameters
    ///
    /// - `scheme`: The authentication scheme to use
    /// - `username`: The username for authentication
    /// - `realm`: The authentication realm (must match the challenge)
    /// - `nonce`: The nonce from the challenge
    /// - `uri`: The request URI
    /// - `response`: The computed digest response
    ///
    /// # Returns
    ///
    /// A new Authorization header with the specified credentials
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// );
    /// ```
    pub fn new(
        scheme: AuthScheme,
        username: impl Into<String>,
        realm: impl Into<String>,
        nonce: impl Into<String>,
        uri: Uri,
        response: impl Into<String>
    ) -> Self {
        Self(Credentials::Digest { params: vec![
            DigestParam::Realm(realm.into()),
            DigestParam::Nonce(nonce.into()),
            DigestParam::Uri(uri),
            DigestParam::Response(response.into()),
        ] })
    }

    /// Sets the algorithm parameter.
    ///
    /// # Parameters
    ///
    /// - `algorithm`: The hash algorithm used
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// ).with_algorithm(Algorithm::Md5);
    /// ```
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Algorithm(algorithm));
        }
        self
    }

    /// Sets the cnonce parameter.
    ///
    /// The cnonce (client nonce) is a nonce generated by the client and is required
    /// when using quality of protection (qop).
    ///
    /// # Parameters
    ///
    /// - `cnonce`: The client-generated nonce
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// ).with_cnonce("87654321");
    /// ```
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Cnonce(cnonce.into()));
        }
        self
    }

    /// Sets the opaque parameter.
    ///
    /// The opaque parameter must be returned unchanged from the challenge.
    ///
    /// # Parameters
    ///
    /// - `opaque`: The opaque string from the challenge
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// ).with_opaque("opaque-data");
    /// ```
    pub fn with_opaque(mut self, opaque: impl Into<String>) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::Opaque(opaque.into()));
        }
        self
    }

    /// Sets the message_qop parameter.
    ///
    /// This specifies which quality of protection the client has selected
    /// from those offered by the server.
    ///
    /// # Parameters
    ///
    /// - `qop`: The quality of protection used
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// ).with_qop(Qop::Auth);
    /// ```
    pub fn with_qop(mut self, qop: Qop) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::MsgQop(qop));
        }
        self
    }

    /// Sets the nonce_count parameter.
    ///
    /// The nonce count is incremented by the client each time it reuses the same
    /// nonce in a new request, and is required when using quality of protection (qop).
    ///
    /// # Parameters
    ///
    /// - `nc`: The nonce count value
    ///
    /// # Returns
    ///
    /// The modified Authorization header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "bob",
    ///     "example.com",
    ///     "1234abcd",
    ///     uri,
    ///     "5678efgh"
    /// ).with_nonce_count(1);
    /// ```
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        if let Credentials::Digest { ref mut params } = self.0 {
            params.push(DigestParam::NonceCount(nc));
        }
        self
    }
}

impl FromStr for Authorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_authorization(s.as_bytes())
            .map(|(_, auth_header)| auth_header) // parser returns AuthorizationHeader directly
            .map_err(Error::from)
    }
}

/// Typed Proxy-Authenticate header.
///
/// The Proxy-Authenticate header is used by proxy servers in 407 Proxy Authentication Required
/// responses to challenge the client to authenticate itself to the proxy. It is similar to the
/// WWW-Authenticate header but scoped to proxy authentication.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Proxy-Authenticate header with a Digest challenge
/// let proxy_auth = ProxyAuthenticate::new(
///     Challenge::Digest {
///         params: vec![
///             DigestParam::Realm("proxy.example.com".to_string()),
///             DigestParam::Nonce("proxy-nonce-1234".to_string()),
///             DigestParam::Algorithm(Algorithm::Md5)
///         ]
///     }
/// );
///
/// // Parse from a string
/// let proxy_auth = ProxyAuthenticate::from_str(
///     "Digest realm=\"proxy.example.com\", nonce=\"proxy-nonce-1234\", algorithm=MD5"
/// ).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthenticate(pub Challenge); // Holds Challenge

impl fmt::Display for ProxyAuthenticate {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthenticate {
    /// Creates a new ProxyAuthenticate header.
    ///
    /// # Parameters
    ///
    /// - `challenge`: The authentication challenge
    ///
    /// # Returns
    ///
    /// A new ProxyAuthenticate header with the specified challenge
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let proxy_auth = ProxyAuthenticate::new(
    ///     Challenge::Digest {
    ///         params: vec![
    ///             DigestParam::Realm("proxy.example.com".to_string()),
    ///             DigestParam::Nonce("proxy-nonce-1234".to_string())
    ///         ]
    ///     }
    /// );
    /// ```
    pub fn new(challenge: Challenge) -> Self { Self(challenge) }
}

impl FromStr for ProxyAuthenticate {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
         // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_proxy_authenticate(s.as_bytes())
            .map(|(_, challenge)| ProxyAuthenticate(challenge))
            .map_err(Error::from)
    }
}

/// Typed Proxy-Authorization header.
///
/// The Proxy-Authorization header is used by clients to provide authentication credentials
/// to a proxy server in response to a Proxy-Authenticate challenge. It is similar to the
/// Authorization header but scoped to proxy authentication.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Proxy-Authorization header with Digest credentials
/// let uri = Uri::from_str("sip:example.com").unwrap();
/// let creds = Credentials::Digest {
///     params: vec![
///         DigestParam::Username("bob".to_string()),
///         DigestParam::Realm("proxy.example.com".to_string()),
///         DigestParam::Nonce("proxy-nonce-1234".to_string()),
///         DigestParam::Uri(uri),
///         DigestParam::Response("proxy-response-5678".to_string())
///     ]
/// };
/// let proxy_auth = ProxyAuthorization::new(creds);
///
/// // Parse from a string
/// let proxy_auth = ProxyAuthorization::from_str(
///     "Digest username=\"bob\", realm=\"proxy.example.com\", \
///      nonce=\"proxy-nonce-1234\", uri=\"sip:example.com\", \
///      response=\"proxy-response-5678\""
/// ).unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProxyAuthorization(pub Credentials); // Holds Credentials

impl fmt::Display for ProxyAuthorization {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl ProxyAuthorization {
    /// Creates a new ProxyAuthorization header.
    ///
    /// # Parameters
    ///
    /// - `creds`: The authentication credentials
    ///
    /// # Returns
    ///
    /// A new ProxyAuthorization header with the specified credentials
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:example.com").unwrap();
    /// let creds = Credentials::Digest {
    ///     params: vec![
    ///         DigestParam::Username("bob".to_string()),
    ///         DigestParam::Realm("proxy.example.com".to_string()),
    ///         DigestParam::Nonce("proxy-nonce-1234".to_string()),
    ///         DigestParam::Uri(uri),
    ///         DigestParam::Response("proxy-response-5678".to_string())
    ///     ]
    /// };
    /// let proxy_auth = ProxyAuthorization::new(creds);
    /// ```
    pub fn new(creds: Credentials) -> Self { Self(creds) }
}

impl FromStr for ProxyAuthorization {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
        // Call the actual parser and map nom::Err to crate::error::Error
        crate::parser::headers::parse_proxy_authorization(s.as_bytes())
             .map(|(_, creds)| ProxyAuthorization(creds))
             .map_err(Error::from)
    }
}

/// Typed Authentication-Info header.
///
/// The Authentication-Info header is used in responses from a server after successful
/// authentication. It provides additional authentication information to the client, such
/// as a new nonce for subsequent requests or a server authentication response for mutual
/// authentication.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create an Authentication-Info header
/// let auth_info = AuthenticationInfo::new()
///     .with_nextnonce("5678efgh")
///     .with_qop(Qop::Auth)
///     .with_rspauth("server-response-hash")
///     .with_cnonce("client-nonce")
///     .with_nonce_count(1);
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AuthenticationInfo(pub Vec<AuthenticationInfoParam>); // Holds a list of params

impl fmt::Display for AuthenticationInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let params_str = self.0.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ");
        write!(f, "{}", params_str)
    }
}

impl AuthenticationInfo {
    /// Creates a new empty AuthenticationInfo header.
    ///
    /// # Returns
    ///
    /// A new empty AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new();
    /// ```
    pub fn new() -> Self {
        Default::default()
    }

    /// Sets the nextnonce parameter.
    ///
    /// The next nonce to be used for authentication, provided by the server to
    /// allow the client to authenticate in future requests without waiting for
    /// an authorization failure.
    ///
    /// # Parameters
    ///
    /// - `nextnonce`: The next nonce value to use
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new()
    ///     .with_nextnonce("5678efgh");
    /// ```
    pub fn with_nextnonce(mut self, nextnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::NextNonce(nextnonce.into()));
        self
    }

    /// Sets the qop parameter.
    ///
    /// The quality of protection that was applied to the previous request.
    ///
    /// # Parameters
    ///
    /// - `qop`: The quality of protection used
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new()
    ///     .with_qop(Qop::Auth);
    /// ```
    pub fn with_qop(mut self, qop: Qop) -> Self {
        self.0.push(AuthenticationInfoParam::Qop(qop));
        self
    }

    /// Sets the rspauth parameter.
    ///
    /// The rspauth (response authentication) parameter is used for mutual authentication,
    /// allowing the client to authenticate the server.
    ///
    /// # Parameters
    ///
    /// - `rspauth`: The server's authentication response
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new()
    ///     .with_rspauth("server-response-hash");
    /// ```
    pub fn with_rspauth(mut self, rspauth: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::ResponseAuth(rspauth.into()));
        self
    }

    /// Sets the cnonce parameter.
    ///
    /// The cnonce (client nonce) echoed from the client's request.
    ///
    /// # Parameters
    ///
    /// - `cnonce`: The client nonce value
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new()
    ///     .with_cnonce("client-nonce");
    /// ```
    pub fn with_cnonce(mut self, cnonce: impl Into<String>) -> Self {
        self.0.push(AuthenticationInfoParam::Cnonce(cnonce.into()));
        self
    }

    /// Sets the nc (nonce count) parameter.
    ///
    /// The nonce count echoed from the client's request.
    ///
    /// # Parameters
    ///
    /// - `nc`: The nonce count value
    ///
    /// # Returns
    ///
    /// The modified AuthenticationInfo header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let auth_info = AuthenticationInfo::new()
    ///     .with_nonce_count(1);
    /// ```
    pub fn with_nonce_count(mut self, nc: u32) -> Self {
        self.0.push(AuthenticationInfoParam::NonceCount(nc));
        self
    }
}

impl FromStr for AuthenticationInfo {
    type Err = crate::error::Error;
    fn from_str(s: &str) -> Result<Self> {
         // Call the actual parser and map nom::Err to crate::error::Error
         crate::parser::headers::parse_authentication_info(s.as_bytes())
            .map(|(_, params)| AuthenticationInfo(params))
            .map_err(Error::from)
    }
}

// TODO: Implement default values, helper methods, and parsing logic for each.
// TODO: Re-implement the `new` and `with_*` methods for the wrapper types
//       to correctly interact with the new enum/vec structures.
//       This requires careful handling of finding/replacing/adding params. 