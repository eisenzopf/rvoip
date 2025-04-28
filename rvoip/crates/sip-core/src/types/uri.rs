//! # SIP URI Implementation
//!
//! This module provides a comprehensive implementation of SIP Uniform Resource Identifiers (URIs) 
//! as defined in [RFC 3261](https://tools.ietf.org/html/rfc3261).
//!
//! SIP URIs are used to identify users, servers, and services in a SIP network.
//! They have a similar structure to email addresses, with additional parameters 
//! and headers for SIP-specific functionality.
//!
//! ## URI Structure
//!
//! A SIP URI has the following general form:
//!
//! ```text
//! sip:user:password@host:port;uri-parameters?headers
//! ```
//!
//! Where:
//! - `sip:` is the scheme (can also be `sips:` for secure SIP)
//! - `user:password` is the optional userinfo component (password is deprecated)
//! - `host` is the required domain name or IP address
//! - `port` is the optional port number
//! - `uri-parameters` are optional parameters (`;key=value` or `;key`)
//! - `headers` are optional headers (`?key=value&key=value`)
//!
//! ## Usage Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Parse a URI from a string
//! let uri = Uri::from_str("sip:alice@example.com:5060;transport=udp?subject=meeting").unwrap();
//!
//! // Access URI components
//! assert_eq!(uri.scheme, Scheme::Sip);
//! assert_eq!(uri.username(), Some("alice"));
//! assert_eq!(uri.host.to_string(), "example.com");
//! assert_eq!(uri.port, Some(5060));
//! assert_eq!(uri.transport(), Some("udp"));
//!
//! // Create a URI programmatically
//! let uri = Uri::sip("example.com")
//!     .with_user("bob")
//!     .with_port(5060)
//!     .with_parameter(Param::transport("tcp"));
//!
//! assert_eq!(uri.to_string(), "sip:bob@example.com:5060;transport=tcp");
//! ```

// URI implementation moved from root directory
// Implements URI types according to RFC 3261

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_till, take_until, take_while, take_while1},
    character::complete::{char, digit1, hex_digit1},
    combinator::{map, map_res, opt, recognize, verify, all_consuming},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, separated_pair, terminated, tuple},
    IResult,
};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::param::Param; // Updated import path
use crate::parser::uri::parse_uri; // Import the nom parser

/// SIP URI scheme types
///
/// Represents the scheme component of a URI, which indicates the protocol
/// or addressing scheme being used.
///
/// The most common schemes in SIP are:
/// - `sip`: Standard SIP (typically over UDP or TCP)
/// - `sips`: Secure SIP (typically over TLS)
/// - `tel`: Telephone number
///
/// The implementation also supports `http`, `https`, and custom schemes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scheme {
    /// SIP URI (non-secure)
    Sip,
    /// SIPS URI (secure SIP)
    Sips,
    /// TEL URI (telephone number)
    Tel,
    /// HTTP URI
    Http,
    /// HTTPS URI
    Https,
    /// Custom scheme (any other scheme)
    Custom(String),
}

impl Scheme {
    /// Returns the string representation of the scheme
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::uri::Scheme;
    ///
    /// assert_eq!(Scheme::Sip.as_str(), "sip");
    /// assert_eq!(Scheme::Sips.as_str(), "sips");
    /// assert_eq!(Scheme::Custom("xmpp".to_string()).as_str(), "xmpp");
    /// ```
    pub fn as_str(&self) -> &str {
        match self {
            Scheme::Sip => "sip",
            Scheme::Sips => "sips",
            Scheme::Tel => "tel",
            Scheme::Http => "http",
            Scheme::Https => "https",
            Scheme::Custom(scheme) => scheme,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scheme {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Check specifically for schemes followed by ':' implicitly
        // The nom parser `terminated(scheme_parser, char(':'))` ensures this, 
        // so direct FromStr should handle the base scheme string correctly.
        match s.to_lowercase().as_str() {
            "sip" => Ok(Scheme::Sip),
            "sips" => Ok(Scheme::Sips),
            "tel" => Ok(Scheme::Tel),
            "http" => Ok(Scheme::Http),
            "https" => Ok(Scheme::Https),
            _ => Ok(Scheme::Custom(s.to_string())), // Support arbitrary schemes
        }
    }
}

/// Represents the host part of a URI.
///
/// The host can be either a domain name or an IP address (v4 or v6).
/// In SIP URIs, the host is the mandatory component that identifies the target
/// server or endpoint.
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Domain name
/// let host = Host::domain("example.com");
/// assert_eq!(host.to_string(), "example.com");
///
/// // IPv4 address
/// let host = Host::from_str("192.168.1.1").unwrap();
/// assert!(matches!(host, Host::Address(_)));
///
/// // IPv6 address
/// let host = Host::from_str("[2001:db8::1]").unwrap();
/// assert!(matches!(host, Host::Address(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Host {
    /// A domain name (e.g., "example.com").
    Domain(String),
    /// An IP address (v4 or v6).
    Address(IpAddr),
}

impl Host {
    /// Create a new host from a domain name
    ///
    /// # Parameters
    /// - `domain`: The domain name string
    ///
    /// # Returns
    /// A new Host instance with the Domain variant
    pub fn domain(domain: impl Into<String>) -> Self {
        Host::Domain(domain.into())
    }

    /// Create a new host from an IPv4 address
    ///
    /// # Parameters
    /// - `ip`: The IPv4 address as a string
    ///
    /// # Returns
    /// A new Host instance with the Address variant
    ///
    /// # Panics
    /// Panics if the input string is not a valid IPv4 address
    pub fn ipv4(ip: impl Into<String>) -> Self {
        Host::Address(IpAddr::V4(Ipv4Addr::from_str(ip.into().as_str()).unwrap()))
    }

    /// Create a new host from an IPv6 address
    ///
    /// # Parameters
    /// - `ip`: The IPv6 address as a string
    ///
    /// # Returns
    /// A new Host instance with the Address variant
    ///
    /// # Panics
    /// Panics if the input string is not a valid IPv6 address
    pub fn ipv6(ip: impl Into<String>) -> Self {
        Host::Address(IpAddr::V6(Ipv6Addr::from_str(ip.into().as_str()).unwrap()))
    }

    /// Get the host as a string slice (only works for domain names).
    /// For addresses, it converts to String.
    ///
    /// # Returns
    /// The host as a string
    pub fn as_str(&self) -> String {
        match self {
            Host::Domain(domain) => domain.clone(),
            Host::Address(addr) => addr.to_string(),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(domain) => write!(f, "{}", domain),
            Host::Address(addr) => write!(f, "{}", addr),
        }
    }
}

impl FromStr for Host {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Attempt to parse as IP address first
        if let Ok(addr) = IpAddr::from_str(s) {
            return Ok(Host::Address(addr));
        } else if s.starts_with('[') {
            if s.ends_with(']') {
                // Properly formatted IPv6 with brackets
                if let Ok(addr) = Ipv6Addr::from_str(&s[1..s.len()-1]) {
                    return Ok(Host::Address(IpAddr::V6(addr)));
                }
                // If it looked like IPv6 but failed, treat as domain
                Ok(Host::Domain(s.to_string()))
            } else {
                // String starts with '[' but doesn't end with ']' - malformed IPv6
                Err(Error::InvalidUri(format!("Malformed IPv6 address (unclosed bracket): {}", s)))
            }
        } else {
            // Assume domain name if not a valid IP
            // TODO: Add stricter domain name validation?
             if s.is_empty() {
                 Err(Error::ParseError("Host cannot be empty".to_string()))
             } else {
                Ok(Host::Domain(s.to_string()))
             }
        }
    }
}

impl From<IpAddr> for Host {
    fn from(addr: IpAddr) -> Self {
        Host::Address(addr)
    }
}

impl From<Ipv4Addr> for Host {
    fn from(addr: Ipv4Addr) -> Self {
        Host::Address(IpAddr::V4(addr))
    }
}

impl From<Ipv6Addr> for Host {
    fn from(addr: Ipv6Addr) -> Self {
        Host::Address(IpAddr::V6(addr))
    }
}

/// SIP URI components as defined in RFC 3261
///
/// Represents a complete SIP URI with all its components. URIs are used throughout
/// the SIP protocol to identify endpoints, proxy servers, redirect servers, and
/// other network elements.
///
/// # Structure
///
/// A complete SIP URI has the following format:
/// `sip:user:password@host:port;uri-parameters?headers`
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Parse a URI from a string
/// let uri = Uri::from_str("sip:alice@example.com").unwrap();
///
/// // Create a URI programmatically
/// let uri = Uri::sip("example.com")
///     .with_user("bob")
///     .with_port(5060)
///     .with_parameter(Param::transport("tcp"));
///
/// // Get components
/// assert_eq!(uri.scheme.as_str(), "sip");
/// assert_eq!(uri.username(), Some("bob"));
/// assert_eq!(uri.port, Some(5060));
/// assert_eq!(uri.transport(), Some("tcp"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Uri {
    /// URI scheme (sip, sips, tel)
    pub scheme: Scheme,
    /// User part (optional)
    pub user: Option<String>,
    /// Password (optional, deprecated)
    pub password: Option<String>,
    /// Host (required)
    pub host: Host,
    /// Port (optional)
    pub port: Option<u16>,
    /// URI parameters (;key=value or ;key)
    pub parameters: Vec<Param>,
    /// URI headers (?key=value)
    pub headers: HashMap<String, String>,
    /// Raw URI string for custom schemes
    pub raw_uri: Option<String>,
}

impl Uri {
    /// Create a new URI with the minimum required fields
    ///
    /// # Parameters
    /// - `scheme`: The URI scheme
    /// - `host`: The host part
    ///
    /// # Returns
    /// A new URI instance with the given scheme and host
    pub fn new(scheme: Scheme, host: Host) -> Self {
        Uri {
            scheme,
            user: None,
            password: None,
            host,
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: None,
        }
    }

    /// Returns the scheme of this URI
    ///
    /// # Returns
    /// The scheme (e.g., Sip, Sips, Tel)
    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }
    
    /// Returns the host and port (if present) formatted as a string
    ///
    /// # Returns
    /// The host and port as a string (e.g., "example.com:5060")
    pub fn host_port(&self) -> String {
        match self.port {
            Some(port) if port > 0 => format!("{}:{}", self.host, port),
            _ => self.host.to_string()
        }
    }

    /// Create a new SIP URI with a domain host
    ///
    /// # Parameters
    /// - `host`: The domain name
    ///
    /// # Returns
    /// A new URI with SIP scheme and the given domain host
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip("example.com");
    /// assert_eq!(uri.to_string(), "sip:example.com");
    /// ```
    pub fn sip(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::domain(host))
    }

    /// Create a new SIP URI with an IPv4 host
    ///
    /// # Parameters
    /// - `host`: The IPv4 address as a string
    ///
    /// # Returns
    /// A new URI with SIP scheme and the given IPv4 host
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip_ipv4("192.168.1.1");
    /// assert_eq!(uri.to_string(), "sip:192.168.1.1");
    /// ```
    pub fn sip_ipv4(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::ipv4(host))
    }

    /// Create a new SIP URI with an IPv6 host
    ///
    /// # Parameters
    /// - `host`: The IPv6 address as a string
    ///
    /// # Returns
    /// A new URI with SIP scheme and the given IPv6 host
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip_ipv6("2001:db8::1");
    /// assert_eq!(uri.to_string(), "sip:[2001:db8::1]");
    /// ```
    pub fn sip_ipv6(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sip, Host::ipv6(host))
    }

    /// Create a new SIPS URI
    ///
    /// # Parameters
    /// - `host`: The domain name
    ///
    /// # Returns
    /// A new URI with SIPS scheme and the given domain host
    pub fn sips(host: impl Into<String>) -> Self {
        Self::new(Scheme::Sips, Host::domain(host))
    }

    /// Create a new TEL URI
    ///
    /// # Parameters
    /// - `number`: The telephone number
    ///
    /// # Returns
    /// A new URI with TEL scheme and the given number as host
    pub fn tel(number: impl Into<String>) -> Self {
        Self::new(Scheme::Tel, Host::domain(number))
    }

    /// Create a new HTTP URI
    ///
    /// # Parameters
    /// - `host`: The domain name
    ///
    /// # Returns
    /// A new URI with HTTP scheme and the given domain host
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::http("example.com");
    /// assert_eq!(uri.to_string(), "http:example.com");
    /// ```
    pub fn http(host: impl Into<String>) -> Self {
        Self::new(Scheme::Http, Host::domain(host))
    }

    /// Create a new HTTPS URI
    ///
    /// # Parameters
    /// - `host`: The domain name
    ///
    /// # Returns
    /// A new URI with HTTPS scheme and the given domain host
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::https("example.com");
    /// assert_eq!(uri.to_string(), "https:example.com");
    /// ```
    pub fn https(host: impl Into<String>) -> Self {
        Self::new(Scheme::Https, Host::domain(host))
    }

    /// Create a new URI with a custom scheme by storing the entire URI string
    /// This is used for schemes that are not explicitly supported (like http, https)
    /// but need to be preserved in the Call-Info header
    ///
    /// # Parameters
    /// - `uri_string`: The full URI string
    ///
    /// # Returns
    /// A new URI with the appropriate scheme and preserved raw string
    pub fn custom(uri_string: impl Into<String>) -> Self {
        let uri_string = uri_string.into();
        
        // Try to extract the scheme if possible
        let scheme = if let Some(colon_pos) = uri_string.find(':') {
            let scheme_str = &uri_string[0..colon_pos];
            match scheme_str.to_lowercase().as_str() {
                "http" => Scheme::Http,
                "https" => Scheme::Https,
                "tel" => Scheme::Tel,
                "sip" => Scheme::Sip,
                "sips" => Scheme::Sips,
                _ => Scheme::Custom(scheme_str.to_string()), // Preserve custom schemes
            }
        } else {
            // If no scheme found, create a custom scheme from the whole string
            Scheme::Custom(uri_string.clone())
        };
        
        Uri {
            scheme,
            user: None,
            password: None,
            host: Host::domain("unknown.host"), // Placeholder host
            port: None,
            parameters: Vec::new(),
            headers: HashMap::new(),
            raw_uri: Some(uri_string),
        }
    }

    /// Check if this URI has a custom scheme (non-SIP)
    ///
    /// # Returns
    /// `true` if this is a custom URI, `false` otherwise
    pub fn is_custom(&self) -> bool {
        self.raw_uri.is_some()
    }

    /// Get the raw URI string if this is a custom URI
    ///
    /// # Returns
    /// The raw URI string if this is a custom URI, `None` otherwise
    pub fn as_raw_uri(&self) -> Option<&str> {
        self.raw_uri.as_deref()
    }

    /// Get the username part of the URI, if present
    ///
    /// # Returns
    /// The username as a string slice, or `None` if not set
    pub fn username(&self) -> Option<&str> {
        self.user.as_deref()
    }

    /// Set the user part of the URI
    ///
    /// # Parameters
    /// - `user`: The user part to set
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// Set the password part of the URI (deprecated in SIP)
    ///
    /// # Parameters
    /// - `password`: The password to set
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Note
    /// Passwords in SIP URIs are deprecated for security reasons,
    /// but supported for compatibility.
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Set the port part of the URI
    ///
    /// # Parameters
    /// - `port`: The port number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Add a parameter to the URI
    ///
    /// # Parameters
    /// - `param`: The parameter to add
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip("example.com")
    ///     .with_parameter(Param::transport("tcp"))
    ///     .with_parameter(Param::ttl(60));
    ///
    /// assert_eq!(uri.to_string(), "sip:example.com;transport=tcp;ttl=60");
    /// ```
    pub fn with_parameter(mut self, param: Param) -> Self {
        self.parameters.push(param);
        self
    }

    /// Add a header to the URI
    ///
    /// # Parameters
    /// - `key`: The header name
    /// - `value`: The header value
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip("example.com")
    ///     .with_header("subject", "Meeting")
    ///     .with_header("priority", "urgent");
    ///
    /// // Headers are added to the URI string
    /// let uri_str = uri.to_string();
    /// assert!(uri_str.contains("subject=Meeting"));
    /// assert!(uri_str.contains("priority=urgent"));
    /// ```
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Returns the transport parameter if present
    ///
    /// # Returns
    /// The transport value as a string slice, or `None` if not set
    ///
    /// # Examples
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri = Uri::sip("example.com")
    ///     .with_parameter(Param::transport("tcp"));
    ///
    /// assert_eq!(uri.transport(), Some("tcp"));
    /// ```
    pub fn transport(&self) -> Option<&str> {
        self.parameters.iter().find_map(|p| match p {
            Param::Transport(val) => Some(val.as_str()),
            _ => None,
        })
    }

    /// Returns the user=phone parameter if present
    ///
    /// # Returns
    /// `true` if the URI has the user=phone parameter, `false` otherwise
    ///
    /// # Examples
    /// ```
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:+12125551212@example.com;user=phone").unwrap();
    /// assert!(uri.is_phone_number());
    ///
    /// let uri = Uri::sip("example.com").with_user("alice");
    /// assert!(!uri.is_phone_number());
    /// ```
    pub fn is_phone_number(&self) -> bool {
        self.parameters.iter().any(|p| match p {
            Param::User(val) if val == "phone" => true,
            _ => false,
        })
    }
}

impl fmt::Display for Uri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // If this is a custom URI, just output the raw string
        if let Some(raw_uri) = &self.raw_uri {
            return f.write_str(raw_uri);
        }

        // Normal URI formatting
        write!(f, "{}:", self.scheme)?;
        
        // User info (username and optional password)
        if let Some(ref user) = self.user {
            let escaped_user = escape_user_info(user);
            write!(f, "{}", escaped_user)?;
            
            if let Some(ref password) = self.password {
                let escaped_password = escape_user_info(password);
                write!(f, ":{}", escaped_password)?;
            }
            
            write!(f, "@")?;
        }
        
        // Host (domain or IP address)
        match &self.host {
            Host::Domain(domain) => write!(f, "{}", domain)?,
            Host::Address(IpAddr::V4(addr)) => write!(f, "{}", addr)?,
            Host::Address(IpAddr::V6(addr)) => write!(f, "[{}]", addr)?,
        }
        
        // Optional port (only if not 0)
        if let Some(port) = self.port {
            // Don't show port 0
            if port > 0 {
                write!(f, ":{}", port)?;
            }
        }
        
        // Parameters (;key=value or ;key)
        for param in &self.parameters {
            write!(f, ";{}", param)?;
        }
        
        // Headers (?key=value&key=value)
        if !self.headers.is_empty() {
            let mut first = true;
            for (key, value) in &self.headers {
                if first {
                    write!(f, "?")?;
                    first = false;
                } else {
                    write!(f, "&")?;
                }
                
                // URL-encode key and value
                write!(f, "{}={}", escape_param(key), escape_param(value))?;
            }
        }
        
        Ok(())
    }
}

// Use the internal nom parser for FromStr
impl FromStr for Uri {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        // Use the nom parser from the parser module
        match all_consuming(parse_uri)(s.as_bytes()) {
            Ok((_rem, uri)) => Ok(uri),
            Err(e) => {
                // For HTTP and HTTPS URIs, create a special case
                if s.starts_with("http://") || s.starts_with("https://") {
                    let scheme = if s.starts_with("https://") { 
                        Scheme::Https 
                    } else { 
                        Scheme::Http 
                    };
                    
                    // Extract host from the URL (simple implementation)
                    let without_scheme = if s.starts_with("https://") {
                        &s[8..]
                    } else {
                        &s[7..]
                    };
                    
                    let host_part = without_scheme
                        .split('/')
                        .next()
                        .unwrap_or(without_scheme);
                    
                    return Ok(Uri {
                        scheme,
                        user: None,
                        password: None,
                        host: Host::domain(host_part),
                        port: None,
                        parameters: Vec::new(),
                        headers: HashMap::new(),
                        raw_uri: Some(s.to_string()),
                    });
                }
                
                // Otherwise return the original error
                Err(Error::from(e.to_owned())) // Convert nom::Err to crate::error::Error
            }
        }
    }
}

// Use the internal nom parser for From<&str>
impl<'a> From<&'a str> for Uri {
    fn from(s: &'a str) -> Self {
        // Attempt to parse using the internal nom parser.
        // If it fails, fall back to a custom URI with the raw string.
        Uri::from_str(s).unwrap_or_else(|_| {
            // If parsing fails, create a custom URI to preserve the string
            Uri::custom(s)
        })
    }
}

// Parser-related functions need to be reimplemented or imported from parser module
// For now we'll just include the basic utility functions used by the URI implementation

// --- Helper functions (escape/unescape, validation) ---

/// Escape URI user info component according to RFC 3261
///
/// This escapes characters in the user info component (username/password)
/// using percent-encoding as specified in the RFC.
///
/// # Parameters
/// - `s`: The string to escape
///
/// # Returns
/// The escaped string
fn escape_user_info(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3); // Worst case: all chars need escaping (×3)

    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' => {
                result.push(c);
            },
            _ => {
                // Escape all other characters
                for byte in c.to_string().bytes() {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
    }

    result
}

/// Unescape URI user info component
fn unescape_user_info(s: &str) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            let mut hex = String::with_capacity(2);

            if let Some(h1) = chars.next() {
                hex.push(h1);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }

            if let Some(h2) = chars.next() {
                hex.push(h2);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: "Incomplete percent encoding".to_string()
                });
            }

            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                return Err(Error::MalformedUriComponent {
                    component: "user info".to_string(),
                    message: format!("Invalid percent encoding: %{}", hex)
                });
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Escape URI parameters and headers
fn escape_param(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);

    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' |
            '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' | '+' => {
                result.push(c);
            },
            _ => {
                // Escape all other characters
                for byte in c.to_string().bytes() {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
    }

    result
}

/// Unescape URI parameters and headers
fn unescape_param(s: &str) -> Result<String> {
    unescape_user_info(s) // Same algorithm
}

/// Check if a string is a valid IPv4 address
fn is_valid_ipv4(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();

    if parts.len() != 4 {
        return false;
    }

    for part in parts {
        match part.parse::<u8>() {
            Ok(_) => continue,
            Err(_) => return false,
        }
    }

    true
}

/// Check if a string is a valid IPv6 address (simplified validation)
fn is_valid_ipv6(s: &str) -> bool {
    // Check for basic IPv6 format
    let parts = s.split(':').collect::<Vec<&str>>();

    // IPv6 has 8 parts max, or fewer if contains ::
    if parts.len() > 8 {
        return false;
    }

    // Check for empty parts (::)
    let empty_parts = parts.iter().filter(|p| p.is_empty()).count();

    // Handle :: (consecutive colons)
    if empty_parts > 0 {
        if empty_parts > 2 || (empty_parts == 2 && !s.contains("::")) {
            return false;
        }
    }

    // Validate each part
    for part in parts {
        if part.is_empty() {
            continue; // Empty part due to ::
        }

        // Check if it's an IPv4 address in the last part (IPv4-mapped IPv6)
        if part.contains('.') {
            return is_valid_ipv4(part);
        }

        // Each part should be a valid hex number with at most 4 digits
        if part.len() > 4 || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }

    true
} 