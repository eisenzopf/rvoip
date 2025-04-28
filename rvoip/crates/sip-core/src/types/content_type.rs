//! # SIP Content-Type Header
//!
//! This module provides an implementation of the SIP Content-Type header as defined in
//! [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
//!
//! The Content-Type header indicates the media type of the message body sent to
//! the recipient. It follows the syntax defined in [RFC 2045](https://datatracker.ietf.org/doc/html/rfc2045)
//! and contains a media type specification, which includes a type, subtype, and
//! optional parameters.
//!
//! ## Common Content Types in SIP
//!
//! - `application/sdp` - Session Description Protocol (for call setup)
//! - `application/pidf+xml` - Presence Information Data Format (for presence services)
//! - `message/sipfrag` - SIP message fragments (used in REFER responses)
//! - `multipart/mixed` - Multiple body parts with different content types
//! - `application/vnd.3gpp.sms` - SMS over IP Multimedia Subsystem
//!
//! ## Format
//!
//! ```text
//! Content-Type: application/sdp
//! Content-Type: application/pidf+xml;charset=UTF-8
//! Content-Type: multipart/mixed;boundary=unique-boundary-1
//! ```
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Content-Type header for SDP
//! let sdp = ContentType::from_type_subtype("application", "sdp");
//! assert_eq!(sdp.to_string(), "application/sdp");
//!
//! // Parse a Content-Type header with parameters
//! let pidf = ContentType::from_str("application/pidf+xml;charset=UTF-8").unwrap();
//! assert_eq!(pidf.to_string(), "application/pidf+xml;charset=\"UTF-8\"");
//! ```

use crate::error::{Result, Error};
use std::fmt;
use std::str::FromStr;
use nom::combinator::all_consuming;
use crate::types::param::Param;
use bytes::Bytes;
use std::collections::HashMap;
use crate::parser;
use crate::parser::headers::content_type::ContentTypeValue;
use crate::parser::headers::content_type::parse_content_type_value;
use serde::{Deserialize, Serialize};

/// Represents the Content-Type header field (RFC 3261 Section 7.3.1).
/// Describes the media type of the message body.
///
/// The Content-Type header is essential for proper interpretation of the 
/// message body by the recipient. It specifies the media type using a type/subtype 
/// syntax, optionally followed by parameters.
///
/// This implementation wraps a `ContentTypeValue` which contains the type, subtype,
/// and any parameters associated with the Content-Type header.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a Content-Type for SDP (Session Description Protocol)
/// let sdp = ContentType::from_type_subtype("application", "sdp");
/// assert_eq!(sdp.to_string(), "application/sdp");
///
/// // Parse a Content-Type with parameters
/// let xml = ContentType::from_str("application/xml;charset=UTF-8").unwrap();
/// assert_eq!(xml.to_string(), "application/xml;charset=\"UTF-8\"");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentType(pub ContentTypeValue);

impl ContentType {
    /// Creates a new Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `value`: A ContentTypeValue containing the type, subtype, and parameters
    ///
    /// # Returns
    ///
    /// A new `ContentType` instance
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::collections::HashMap;
    ///
    /// // Create a ContentTypeValue
    /// let value = ContentTypeValue {
    ///     m_type: "application".to_string(),
    ///     m_subtype: "sdp".to_string(),
    ///     parameters: HashMap::new(),
    /// };
    ///
    /// // Create a ContentType
    /// let content_type = ContentType::new(value);
    /// assert_eq!(content_type.to_string(), "application/sdp");
    /// ```
    pub fn new(value: ContentTypeValue) -> Self {
        Self(value)
    }

    /// Helper to create from basic type/subtype
    ///
    /// This is a convenience method to create a ContentType with just
    /// a media type and subtype, without any parameters.
    ///
    /// # Parameters
    ///
    /// - `m_type`: The media type (e.g., "application", "text", "audio")
    /// - `m_subtype`: The media subtype (e.g., "sdp", "plain", "xml")
    ///
    /// # Returns
    ///
    /// A new `ContentType` instance with the specified type and subtype
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create common SIP Content-Types
    /// let sdp = ContentType::from_type_subtype("application", "sdp");
    /// assert_eq!(sdp.to_string(), "application/sdp");
    ///
    /// let plain_text = ContentType::from_type_subtype("text", "plain");
    /// assert_eq!(plain_text.to_string(), "text/plain");
    ///
    /// let xml = ContentType::from_type_subtype("application", "xml");
    /// assert_eq!(xml.to_string(), "application/xml");
    /// ```
    pub fn from_type_subtype(m_type: impl Into<String>, m_subtype: impl Into<String>) -> Self {
        Self(ContentTypeValue {
            m_type: m_type.into(),
            m_subtype: m_subtype.into(),
            parameters: HashMap::new(),
        })
    }
}

impl fmt::Display for ContentType {
    /// Formats the Content-Type as a string.
    ///
    /// Converts the ContentType to its string representation according to
    /// the SIP specification, including any parameters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Simple Content-Type
    /// let sdp = ContentType::from_type_subtype("application", "sdp");
    /// assert_eq!(sdp.to_string(), "application/sdp");
    ///
    /// // Content-Type with parameters
    /// let content_type = ContentType::from_str("application/xml;charset=UTF-8").unwrap();
    /// assert_eq!(content_type.to_string(), "application/xml;charset=\"UTF-8\"");
    ///
    /// // Using in a formatted string
    /// let header_line = format!("Content-Type: {}", content_type);
    /// assert_eq!(header_line, "Content-Type: application/xml;charset=\"UTF-8\"");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0) // Delegate to MediaType display
    }
}

impl FromStr for ContentType {
    type Err = Error;

    /// Parses a string into a ContentType.
    ///
    /// This method parses a string representation of a Content-Type header
    /// into a ContentType struct. The input string should follow the format
    /// specified in RFC 3261 and RFC 2045.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ContentType, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a simple Content-Type
    /// let sdp = ContentType::from_str("application/sdp").unwrap();
    /// assert_eq!(sdp.to_string(), "application/sdp");
    ///
    /// // Parse with parameters
    /// let content_type = ContentType::from_str("text/plain;charset=UTF-8").unwrap();
    /// assert_eq!(content_type.to_string(), "text/plain;charset=\"UTF-8\"");
    ///
    /// // Parse multipart Content-Type with boundary
    /// let multipart = ContentType::from_str("multipart/mixed;boundary=unique-boundary-1").unwrap();
    /// assert_eq!(multipart.to_string(), "multipart/mixed;boundary=\"unique-boundary-1\"");
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        all_consuming(parse_content_type_value)(s.as_bytes())
            .map_err(Error::from)
            .map(|(_, value)| ContentType(value))
    }
}

// TODO: Implement methods if needed 