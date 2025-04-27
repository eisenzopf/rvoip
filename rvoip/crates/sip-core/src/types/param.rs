//! # SIP Parameters
//! 
//! This module provides types for representing SIP parameters as defined in
//! [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261).
//!
//! Parameters are fundamental components in SIP that attach additional information
//! to headers and URIs. They appear in various contexts including:
//!
//! - URI parameters (e.g., `sip:user@example.com;transport=tcp`)
//! - Header field parameters (e.g., `Contact: <sip:bob@192.0.2.4>;expires=60`)
//! - Header field values parameters (e.g., in Via, From, To headers)
//!
//! This module provides two key types:
//!
//! - [`Param`]: An enum representing different types of SIP parameters with their values
//! - [`GenericValue`]: A type representing parameter values of different formats
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use ordered_float::NotNan;
//! use std::net::IpAddr;
//!
//! // Create a tag parameter
//! let tag = Param::Tag("1928301774".to_string());
//! assert_eq!(tag.to_string(), "tag=1928301774");
//!
//! // Create a quality value parameter
//! let q = Param::Q(NotNan::new(0.8).unwrap());
//! assert_eq!(q.to_string(), "q=0.800");
//!
//! // Create a flag parameter 
//! let lr = Param::Lr;
//! assert_eq!(lr.to_string(), "lr");
//!
//! // Create a custom parameter
//! let custom = Param::Other("x-custom".to_string(), Some("abc123".into()));
//! assert_eq!(custom.to_string(), "x-custom=abc123");
//! ```

use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use ordered_float::NotNan;
use serde::{Serialize, Deserialize};
use std::default::Default;

use crate::error::{Error, Result};
use crate::types::uri::Host; // Assuming Host type exists

// TODO: Add more specific parameter types (like rsip NewTypes) 
// e.g., Branch(String), Tag(String), Expires(u32), etc.

/// Represents the parsed value of a generic parameter.
///
/// This enum can represent different forms of parameter values as defined in RFC 3261:
///
/// - Token values: Simple alphanumeric identifiers (e.g., `transport=tcp`)
/// - Host values: Domain names or IP addresses (e.g., `maddr=example.com`)
/// - Quoted values: Values surrounded by quotes (e.g., `reason="Server Unavailable"`)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::types::param::GenericValue;
///
/// // Create a token value
/// let token = GenericValue::Token("tcp".to_string());
///
/// // Create a quoted value
/// let quoted = GenericValue::Quoted("Call in Progress".to_string());
///
/// // Convert from a string
/// let auto = GenericValue::from("simple token");  // Will be quoted due to space
/// assert!(matches!(auto, GenericValue::Quoted(_)));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)] // Added Eq/Hash assuming Host is Eq/Hash
pub enum GenericValue {
    /// A simple token value
    Token(String),
    /// A domain name or IP address
    Host(Host),
    /// A value containing special characters that requires quoting
    Quoted(String),
}

// Implement Default manually
impl Default for GenericValue {
    fn default() -> Self {
        GenericValue::Token(String::new()) // Default to empty token
    }
}

// Add helper methods
impl GenericValue {
    /// Returns the value as a string slice if it's Token or Quoted.
    ///
    /// # Returns
    ///
    /// - `Some(&str)` if the value is a Token or Quoted string
    /// - `None` if the value is a Host (which cannot be represented as a simple string)
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::param::GenericValue;
    ///
    /// let token = GenericValue::Token("value".to_string());
    /// assert_eq!(token.as_str(), Some("value"));
    ///
    /// let quoted = GenericValue::Quoted("Hello World".to_string());
    /// assert_eq!(quoted.as_str(), Some("Hello World"));
    /// ```
    pub fn as_str(&self) -> Option<&str> {
        match self {
            GenericValue::Token(s) => Some(s.as_str()),
            GenericValue::Quoted(s) => Some(s.as_str()),
            GenericValue::Host(_) => None, // Host is not a simple string slice
        }
    }

    /// Returns the underlying Host if it's the Host variant.
    ///
    /// # Returns
    ///
    /// - `Some(&Host)` if the value is a Host
    /// - `None` if the value is a Token or Quoted string
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::types::param::GenericValue;
    /// use rvoip_sip_core::types::uri::Host;
    ///
    /// let host = GenericValue::Host(Host::domain("example.com"));
    /// assert!(host.as_host().is_some());
    ///
    /// let token = GenericValue::Token("value".to_string());
    /// assert!(token.as_host().is_none());
    /// ```
    pub fn as_host(&self) -> Option<&Host> {
        match self {
            GenericValue::Host(h) => Some(h),
            _ => None,
        }
    }
}

// Implement Display for GenericValue for use in Param::Display
impl fmt::Display for GenericValue {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GenericValue::Token(s) => write!(f, "{}", s),
            GenericValue::Host(h) => write!(f, "{}", h), // Assuming Host implements Display
            GenericValue::Quoted(s) => write!(f, "\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")), // Re-quote safely
        }
    }
}

// Add From<&str> implementation for GenericValue
impl From<&str> for GenericValue {
    fn from(s: &str) -> Self {
        // Check if it should be quoted - if it has spaces or special chars
        if s.contains(char::is_whitespace) || s.contains(|c: char| {
            matches!(c, ';' | ',' | '?' | ':' | '@' | '&' | '=' | '+' | '$' | '/' | '<' | '>' | '#' | '['| ']' | '"')
        }) {
            GenericValue::Quoted(s.to_string())
        } else {
            GenericValue::Token(s.to_string())
        }
    }
}

/// Represents a SIP parameter with its value.
///
/// This enum covers both standard parameters defined in RFC 3261 and custom/extension
/// parameters. Each variant corresponds to a specific type of parameter with appropriate
/// value types:
///
/// - Flag parameters that have no value (e.g., `lr`)
/// - Integer parameters (e.g., `expires=3600`)
/// - String parameters (e.g., `branch=z9hG4bK776asdhds`)
/// - Special parameters with specific formats (e.g., `received=192.0.2.1`)
///
/// # Standard Parameters
///
/// RFC 3261 defines numerous standard parameters, including:
///
/// - `branch`: Identifies a specific transaction in a Via header
/// - `tag`: Identifies a dialog participant in From/To headers
/// - `ttl`: Time-to-live for multicast messages
/// - `received`: IP address where a request was received
/// - `transport`: Transport protocol (UDP, TCP, TLS, etc.)
/// - `user`: Used in SIP URIs to indicate telephone numbers (`user=phone`)
/// - `method`: Used in references to indicate the target method
/// - `q`: Quality value indicating preference (0.0-1.0)
///
/// # Examples
///
/// ```
/// use rvoip_sip_core::prelude::*;
/// use ordered_float::NotNan;
///
/// // Standard parameters
/// let branch = Param::Branch("z9hG4bK776asdhds".to_string());
/// let tag = Param::Tag("1928301774".to_string());
/// let expires = Param::Expires(3600);
/// let q = Param::Q(NotNan::new(0.8).unwrap());
/// let lr = Param::Lr;  // Flag parameter with no value
///
/// // Custom parameter
/// let custom_flag = Param::Other("x-custom-flag".to_string(), None);
/// let custom_value = Param::Other("x-custom".to_string(), Some("value123".into()));
///
/// // Displaying parameters
/// assert_eq!(branch.to_string(), "branch=z9hG4bK776asdhds");
/// assert_eq!(lr.to_string(), "lr");
/// assert_eq!(custom_flag.to_string(), "x-custom-flag");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Param {
    /// The `branch` parameter, typically used in Via headers.
    Branch(String),
    /// The `tag` parameter, used in From/To headers for dialog identification.
    Tag(String),
    /// The `expires` parameter, used in Contact/Expires headers.
    Expires(u32),
    /// The `received` parameter, used in Via headers to indicate the source IP.
    Received(IpAddr),
    /// The `maddr` parameter, used in Via headers.
    Maddr(String), // Keep as string for now, could be Host
    /// The `ttl` parameter, used in Via headers.
    Ttl(u8),
    /// The `lr` parameter (loose routing), a flag parameter used in Via/Route.
    Lr,
    /// The `q` parameter (quality value), used in Contact headers.
    Q(NotNan<f32>),
    /// Transport parameter.
    Transport(String), // Consider using a Transport enum later
    /// User parameter.
    User(String),
    /// Method parameter (rarely used in URIs).
    Method(String), // Consider using types::Method later
    /// Handling parameter (added for Content-Disposition)
    Handling(String), // Added for Content-Disposition (Need Handling enum later)
    /// Duration parameter (added for Retry-After)
    Duration(u32), // Added for Retry-After
    /// The `rport` parameter, a flag in requests, carries port value in responses.
    Rport(Option<u16>),
    /// Generic parameter represented as key-value.
    Other(String, Option<GenericValue>), // Changed value type
}

impl Param {
    /// Creates a new transport parameter.
    ///
    /// # Parameters
    ///
    /// - `transport`: The transport protocol (e.g., "udp", "tcp", "tls")
    ///
    /// # Returns
    ///
    /// A new transport parameter
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let transport = Param::transport("tcp");
    /// assert_eq!(transport.to_string(), "transport=tcp");
    /// ```
    pub fn transport(transport: impl Into<String>) -> Self {
        Param::Transport(transport.into())
    }

    /// Creates a new tag parameter.
    ///
    /// # Parameters
    ///
    /// - `tag`: The tag value
    ///
    /// # Returns
    ///
    /// A new tag parameter
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let tag = Param::tag("1928301774");
    /// assert_eq!(tag.to_string(), "tag=1928301774");
    /// ```
    pub fn tag(tag: impl Into<String>) -> Self {
        Param::Tag(tag.into())
    }

    /// Creates a new branch parameter.
    ///
    /// # Parameters
    ///
    /// - `branch`: The branch value (should start with "z9hG4bK" for RFC 3261 compliance)
    ///
    /// # Returns
    ///
    /// A new branch parameter
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let branch = Param::branch("z9hG4bK776asdhds");
    /// assert_eq!(branch.to_string(), "branch=z9hG4bK776asdhds");
    /// ```
    pub fn branch(branch: impl Into<String>) -> Self {
        Param::Branch(branch.into())
    }

    /// Creates a new ttl parameter.
    ///
    /// # Parameters
    ///
    /// - `ttl`: The time-to-live value
    ///
    /// # Returns
    ///
    /// A new ttl parameter
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let ttl = Param::ttl(60);
    /// assert_eq!(ttl.to_string(), "ttl=60");
    /// ```
    pub fn ttl(ttl: u8) -> Self {
        Param::Ttl(ttl)
    }
}

impl fmt::Display for Param {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Param::Branch(val) => write!(f, "branch={}", val),
            Param::Tag(val) => write!(f, "tag={}", val),
            Param::Expires(val) => write!(f, "expires={}", val),
            Param::Received(val) => write!(f, "received={}", val),
            Param::Maddr(val) => write!(f, "maddr={}", val),
            Param::Ttl(val) => write!(f, "ttl={}", val),
            Param::Lr => write!(f, "lr"),
            Param::Q(val) => write!(f, "q={:.3}", val.into_inner()),
            Param::Transport(val) => write!(f, "transport={}", val),
            Param::User(val) => write!(f, "user={}", val),
            Param::Method(val) => write!(f, "method={}", val),
            Param::Handling(val) => write!(f, "handling={}", val),
            Param::Duration(val) => write!(f, "duration={}", val),
            Param::Rport(Some(val)) => write!(f, "rport={}", val),
            Param::Rport(None) => write!(f, "rport"),
            Param::Other(key, Some(val)) => write!(f, "{}={}", key, val), // Use GenericValue::Display
            Param::Other(key, None) => write!(f, "{}", key),
        }
    }
}

// Note: A FromStr or TryFrom implementation will be added 
// once the parser logic in parser/uri.rs is updated. 