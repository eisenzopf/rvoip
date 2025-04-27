//! # SIP Warning Header
//!
//! This module provides an implementation of the SIP Warning header as defined in
//! [RFC 3261 Section 20.43](https://datatracker.ietf.org/doc/html/rfc3261#section-20.43).
//!
//! The Warning header field is used to carry additional information about the status of a response. 
//! Warning headers are sent with responses and contain a three-digit warning code, host, and 
//! warning text.
//!
//! Warning headers are used for debugging and to provide additional information about why a
//! particular request was not fulfilled. Multiple Warning headers may be included in a response.
//!
//! ## Warning Codes
//!
//! RFC 3261 defines several standard warning codes, including:
//!
//! - 300: Incompatible network protocol
//! - 301: Incompatible network address formats
//! - 302: Incompatible transport protocol
//! - 303: Incompatible bandwidth units
//! - 305: Incompatible media format
//! - 306: Attribute not understood
//! - 307: Session description parameter not understood
//! - 330: Multicast not available
//! - 331: Unicast not available
//! - 370: Insufficient bandwidth
//! - 399: Miscellaneous warning
//!
//! ## Format
//!
//! ```text
//! Warning: 307 example.com "Session parameter 'foo' not understood"
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a Warning header
//! let agent = Uri::sip("example.com");
//! let warning = Warning::new(370, agent, "Insufficient bandwidth");
//!
//! // Format as a string for a SIP message
//! assert_eq!(warning.to_string(), "370 example.com \"Insufficient bandwidth\"");
//! ```

use crate::types::uri::Uri;
use crate::parser::headers::warning::parse_warning_value_list;
use crate::error::Result;
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use serde::{Serialize, Deserialize};

/// Typed Warning header value.
///
/// The Warning header field is used to carry additional information about the status
/// of a response. A SIP Warning consists of:
///
/// - A three-digit warning code (between 300-399)
/// - The hostname or IP address of the warning agent
/// - A quoted warning text
///
/// Warning headers are commonly used for debugging purposes and to provide more
/// detailed information about why a particular request was not fulfilled.
///
/// Multiple Warning headers can be included in a single response to indicate
/// different warnings that apply to the response.
///
/// # Common Warning Codes
///
/// RFC 3261 defines several standard warning codes:
///
/// - 300: Incompatible network protocol
/// - 301: Incompatible network address formats 
/// - 302: Incompatible transport protocol
/// - 303: Incompatible bandwidth units
/// - 305: Incompatible media format
/// - 306: Attribute not understood
/// - 307: Session description parameter not understood
/// - 330: Multicast not available
/// - 331: Unicast not available
/// - 370: Insufficient bandwidth
/// - 399: Miscellaneous warning
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a URI for the warning agent
/// let agent = Uri::sip("sip-proxy.example.com");
///
/// // Create a Warning header for "Incompatible media format"
/// let warning = Warning::new(305, agent, "Audio codec not supported");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// The warning code (300-399)
    pub code: u16,   // 3xx
    /// The hostname or IP address of the entity that added the Warning header
    pub agent: Uri, // Or maybe just Host?
    /// The warning text
    pub text: String,
}

impl Warning {
    /// Creates a new Warning header.
    ///
    /// Initializes a `Warning` header with the provided warning code, agent URI,
    /// and warning text.
    ///
    /// # Parameters
    ///
    /// - `code`: The warning code (should be in the range 300-399)
    /// - `agent`: A URI identifying the entity generating the warning
    /// - `text`: The warning text, can be any type that can be converted into a String
    ///
    /// # Returns
    ///
    /// A new `Warning` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a warning for insufficient bandwidth
    /// let agent = Uri::sip("edge-proxy.example.com");
    /// let warning = Warning::new(370, agent, "Insufficient bandwidth for video");
    ///
    /// // Create a warning for incompatible media format
    /// let agent = Uri::sip("media-server.example.com");
    /// let warning = Warning::new(305, agent, "H.264 profile not supported");
    /// ```
    pub fn new(code: u16, agent: Uri, text: impl Into<String>) -> Self {
        Self { code, agent, text: text.into() }
    }
}

impl fmt::Display for Warning {
    /// Formats the Warning header as a string.
    ///
    /// Converts the `Warning` to its string representation suitable for
    /// inclusion in a SIP message. The format is "{code} {agent} \"{text}\"",
    /// where the text is always quoted.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::fmt::Display;
    ///
    /// let agent = Uri::sip("example.com");
    /// let warning = Warning::new(370, agent, "Insufficient bandwidth");
    ///
    /// assert_eq!(warning.to_string(), "370 example.com \"Insufficient bandwidth\"");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Agent should be host or pseudo-host, URI display might be too much?
        // Using host for now.
        // Text MUST be quoted.
        write!(f, "{} {} \"{}\"", self.code, self.agent.host, self.text)
    }
}

impl FromStr for Warning {
    type Err = crate::error::Error;

    /// Parses a Warning header from a string.
    ///
    /// Attempts to parse a string representation of a Warning header into
    /// a Warning object. The expected format is "{code} {agent} \"{text}\"".
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// - `Ok(Warning)`: If parsing succeeds
    /// - `Err`: If parsing fails
    ///
    /// # Note
    ///
    /// Currently, this implementation is a placeholder and will return an error
    /// as it's not fully implemented yet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // This would parse a warning header when implemented
    /// let warning = Warning::from_str("370 example.com \"Insufficient bandwidth\"");
    /// ```
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        use crate::error::Error; // Ensure Error is in scope

        match all_consuming(parse_warning_value_list)(s.as_bytes()) {
            // TODO: Fix this logic. parse_warning_value_list returns Vec<WarningValue>
            //       We need to map that result to a single Warning struct.
            //       Placeholder: return error for now.
            Ok((_, _value)) => Err(Error::ParseError(
                "FromStr<Warning> not fully implemented yet".to_string()
            )),
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Warning header: {:?}", e)
            ))
        }
    }
}

// TODO: Implement methods if needed 