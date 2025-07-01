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
use std::str::from_utf8;
use crate::types::uri::Host;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the agent in a Warning header
/// 
/// The warn-agent can be either a host:port combination
/// or a pseudonym (token).
#[derive(Debug, PartialEq, Clone)]
pub enum WarnAgent {
    /// A host with an optional port number
    HostPort(Host, Option<u16>),
    /// A simple token/name string
    Pseudonym(String),
}

/// Internal structure for parsed Warning value components
#[derive(Debug, PartialEq, Clone)]
pub struct WarningValue {
    /// The warning code (300-399)
    pub code: u16,
    /// The warning agent (hostname or pseudonym)
    pub agent: WarnAgent,
    /// The warning text (raw bytes)
    pub text: Vec<u8>,
}

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
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a warning header
    /// let warning = Warning::from_str("370 example.com \"Insufficient bandwidth\"").unwrap();
    /// 
    /// // Verify the parsed values
    /// assert_eq!(warning.code, 370);
    /// assert_eq!(warning.agent.host.to_string(), "example.com");
    /// assert_eq!(warning.text, "Insufficient bandwidth");
    /// ```
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        use crate::error::Error; // Ensure Error is in scope

        match all_consuming(parse_warning_value_list)(s.as_bytes()) {
            Ok((_, values)) => {
                // Get the first warning value from the list
                if let Some(value) = values.first() {
                    // Convert the agent to a URI
                    let agent = match &value.agent {
                        // Use the imported WarnAgent enum
                        WarnAgent::HostPort(host, port) => {
                            // Create a SIP URI with the host
                            let mut uri = Uri::sip(host.to_string());
                            // Add port if present
                            if let Some(p) = port {
                                uri.port = Some(*p);
                            }
                            uri
                        },
                        WarnAgent::Pseudonym(name) => {
                            // For pseudonyms, just create a simple SIP URI
                            Uri::sip(name)
                        }
                    };
                    
                    // Convert text from Vec<u8> to String
                    let text = match from_utf8(&value.text) {
                        Ok(s) => s.to_string(),
                        Err(_) => return Err(Error::ParseError("Invalid UTF-8 in warning text".to_string()))
                    };
                    
                    // Create and return the Warning struct
                    Ok(Warning {
                        code: value.code,
                        agent,
                        text
                    })
                } else {
                    Err(Error::ParseError("No warning values found".to_string()))
                }
            },
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Warning header: {:?}", e)
            ))
        }
    }
}

/// A wrapper for a list of Warning headers.
///
/// This type is used to properly implement TypedHeaderTrait for a collection
/// of Warning headers, as required by the TypedHeader::Warning variant which
/// expects Vec<Warning>.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a WarningHeader with a single warning
/// let agent = Uri::sip("example.com");
/// let warning = Warning::new(370, agent, "Insufficient bandwidth");
/// let warning_header = WarningHeader::new(vec![warning]);
///
/// // Convert to a generic header
/// let header = warning_header.to_header();
/// assert_eq!(header.name, HeaderName::Warning);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningHeader {
    /// The list of warnings in this header
    pub warnings: Vec<Warning>,
}

impl WarningHeader {
    /// Creates a new WarningHeader with the given warnings.
    ///
    /// # Parameters
    ///
    /// - `warnings`: A vector of Warning objects
    ///
    /// # Returns
    ///
    /// A new WarningHeader instance
    pub fn new(warnings: Vec<Warning>) -> Self {
        Self { warnings }
    }

    /// Creates a new WarningHeader with a single warning.
    ///
    /// # Parameters
    ///
    /// - `code`: The warning code (should be in the range 300-399)
    /// - `agent`: A URI identifying the entity generating the warning
    /// - `text`: The warning text
    ///
    /// # Returns
    ///
    /// A new WarningHeader instance with a single warning
    pub fn single(code: u16, agent: Uri, text: impl Into<String>) -> Self {
        Self { warnings: vec![Warning::new(code, agent, text)] }
    }

    /// Adds a warning to this header.
    ///
    /// # Parameters
    ///
    /// - `warning`: The Warning to add
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn add_warning(&mut self, warning: Warning) -> &mut Self {
        self.warnings.push(warning);
        self
    }

    /// Adds a new warning with the given parameters.
    ///
    /// # Parameters
    ///
    /// - `code`: The warning code (should be in the range 300-399)
    /// - `agent`: A URI identifying the entity generating the warning
    /// - `text`: The warning text
    ///
    /// # Returns
    ///
    /// Self for method chaining
    pub fn add(&mut self, code: u16, agent: Uri, text: impl Into<String>) -> &mut Self {
        self.warnings.push(Warning::new(code, agent, text));
        self
    }
}

impl fmt::Display for WarningHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.warnings.is_empty() {
            return Ok(());
        }

        for (i, warning) in self.warnings.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", warning)?;
        }
        Ok(())
    }
}

impl FromStr for WarningHeader {
    type Err = crate::error::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        use crate::error::Error;

        match all_consuming(parse_warning_value_list)(s.as_bytes()) {
            Ok((_, values)) => {
                let mut warnings = Vec::with_capacity(values.len());
                
                for value in values {
                    // Convert the agent to a URI
                    let agent = match &value.agent {
                        WarnAgent::HostPort(host, port) => {
                            let mut uri = Uri::sip(host.to_string());
                            if let Some(p) = port {
                                uri.port = Some(*p);
                            }
                            uri
                        },
                        WarnAgent::Pseudonym(name) => {
                            Uri::sip(name)
                        }
                    };
                    
                    // Convert text from Vec<u8> to String
                    let text = match from_utf8(&value.text) {
                        Ok(s) => s.to_string(),
                        Err(_) => return Err(Error::ParseError("Invalid UTF-8 in warning text".to_string()))
                    };
                    
                    warnings.push(Warning {
                        code: value.code,
                        agent,
                        text
                    });
                }
                
                Ok(WarningHeader { warnings })
            },
            Err(e) => Err(Error::ParseError(
                format!("Failed to parse Warning header: {:?}", e)
            ))
        }
    }
}

// Implement TypedHeaderTrait for Warning
impl TypedHeaderTrait for Warning {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Warning
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(crate::error::Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    Warning::from_str(s.trim())
                } else {
                    Err(crate::error::Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            _ => Err(crate::error::Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

// Implement TypedHeaderTrait for WarningHeader
impl TypedHeaderTrait for WarningHeader {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::Warning
    }

    fn to_header(&self) -> Header {
        Header::new(Self::header_name(), HeaderValue::Raw(self.to_string().into_bytes()))
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(crate::error::Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    WarningHeader::from_str(s.trim())
                } else {
                    Err(crate::error::Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            _ => Err(crate::error::Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
} 