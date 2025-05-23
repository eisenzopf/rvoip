//! # SIP Content-Length Header
//!
//! This module provides an implementation of the SIP Content-Length header as defined in
//! [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14).
//!
//! The Content-Length header indicates the size of the message body, in decimal
//! number of octets (bytes), sent to the recipient. Its purpose is to allow 
//! recipients to:
//!
//! - Detect message truncation
//! - Know when they have received the complete message body
//! - Properly allocate buffer space for the message body
//!
//! ## Format
//!
//! ```text
//! Content-Length: 349
//! ```
//!
//! ## Role in SIP
//!
//! The Content-Length header is mandatory if a message body is included in a SIP message,
//! unless the body uses chunked encoding. If no body is present, the Content-Length 
//! header can be set to 0, or it can be omitted entirely.
//! 
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a Content-Length header
//! let length = ContentLength::new(1024);
//! assert_eq!(*length, 1024);
//! assert_eq!(length.to_string(), "1024");
//!
//! // Parse a Content-Length header from a string
//! let length = ContentLength::from_str("2048").unwrap();
//! assert_eq!(*length, 2048);
//! ```

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use crate::parser;
use crate::error::{Result, Error};
use crate::parser::headers::parse_content_length;
use std::ops::Deref;
use nom::combinator::all_consuming;
use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeaderTrait};

/// Represents the Content-Length header field (RFC 3261 Section 7.3.2).
/// Indicates the size of the message body in bytes.
///
/// The Content-Length header is a simple unsigned integer that represents 
/// the size of the message body in bytes (octets). This implementation wraps 
/// a u32 value to provide type safety and SIP-specific functionality.
///
/// The Content-Length header is crucial for proper message parsing, as it allows 
/// the recipient to determine when the complete message body has been received.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use std::str::FromStr;
///
/// // Create a new Content-Length
/// let length = ContentLength::new(1024);
///
/// // Access the inner value
/// assert_eq!(*length, 1024);
///
/// // Parse from a string
/// let length = ContentLength::from_str("2048").unwrap();
/// assert_eq!(*length, 2048);
///
/// // Convert to a string
/// assert_eq!(length.to_string(), "2048");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContentLength(pub u32);

impl ContentLength {
    /// Creates a new Content-Length header value.
    ///
    /// # Parameters
    ///
    /// - `length`: The size of the message body in bytes
    ///
    /// # Returns
    ///
    /// A new `ContentLength` instance with the specified length
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a Content-Length for a message with no body
    /// let empty = ContentLength::new(0);
    /// assert_eq!(*empty, 0);
    ///
    /// // Create a Content-Length for a message with a body
    /// let length = ContentLength::new(1024);
    /// assert_eq!(*length, 1024);
    /// ```
    pub fn new(length: u32) -> Self {
        Self(length)
    }
    
    /// Extracts ContentLength from a Message or similar type.
    ///
    /// This helper method simplifies extracting ContentLength from SIP messages.
    /// Particularly useful in tests and message handling code.
    ///
    /// # Parameters
    ///
    /// - `message`: Any type that can be referenced as a Message
    ///
    /// # Returns
    ///
    /// The ContentLength value, or 0 if not present
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Extract content length from a Message
    /// let message = parse_message(raw_data).unwrap();
    /// let content_length = ContentLength::from_message(&message);
    /// ```
    pub fn from_message(message: &impl AsRef<crate::types::Message>) -> u32 {
        match message.as_ref() {
            crate::types::Message::Request(req) => {
                req.header(&crate::types::header::HeaderName::ContentLength)
                    .and_then(|h| if let crate::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0)
            },
            crate::types::Message::Response(resp) => {
                resp.header(&crate::types::header::HeaderName::ContentLength)
                    .and_then(|h| if let crate::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
                    .unwrap_or(0)
            }
        }
    }

    /// Overload of from_message that accepts a Request directly
    pub fn from_request(req: &crate::types::Request) -> u32 {
        req.header(&crate::types::header::HeaderName::ContentLength)
            .and_then(|h| if let crate::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
            .unwrap_or(0)
    }

    /// Overload of from_message that accepts a Response directly
    pub fn from_response(resp: &crate::types::Response) -> u32 {
        resp.header(&crate::types::header::HeaderName::ContentLength)
            .and_then(|h| if let crate::types::TypedHeader::ContentLength(cl) = h { Some(cl.0) } else { None })
            .unwrap_or(0)
    }
}

impl Deref for ContentLength {
    type Target = u32;

    /// Dereferences to the inner u32 value.
    ///
    /// This implementation allows using ContentLength wherever a u32 reference
    /// is expected, making it easy to access the raw length value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let length = ContentLength::new(1024);
    ///
    /// // Direct comparison with a u32 value
    /// assert_eq!(*length, 1024);
    ///
    /// // Using in calculations
    /// let double = *length * 2;
    /// assert_eq!(double, 2048);
    /// ```
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ContentLength {
    /// Formats the Content-Length as a string.
    ///
    /// Converts the ContentLength to its string representation according to
    /// the SIP specification - simply the decimal integer value.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// let length = ContentLength::new(1024);
    /// assert_eq!(length.to_string(), "1024");
    ///
    /// // Using in a formatted string
    /// assert_eq!(format!("Content-Length: {}", length), "Content-Length: 1024");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ContentLength {
    type Err = Error;

    /// Parses a string into a ContentLength.
    ///
    /// This method parses a string representation of a Content-Length header
    /// into a ContentLength struct. The input string should consist of a valid
    /// decimal integer representing the length in bytes.
    ///
    /// # Parameters
    ///
    /// - `s`: The string to parse
    ///
    /// # Returns
    ///
    /// A Result containing the parsed ContentLength, or an error if parsing fails
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Parse a valid Content-Length
    /// let length = ContentLength::from_str("1024").unwrap();
    /// assert_eq!(*length, 1024);
    ///
    /// // Parse with whitespace
    /// let length = ContentLength::from_str("  2048  ").unwrap();
    /// assert_eq!(*length, 2048);
    ///
    /// // Parse an invalid Content-Length (would return an Error)
    /// let result = ContentLength::from_str("not a number");
    /// assert!(result.is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().parse::<u32>() {
            Ok(len) => Ok(ContentLength(len)),
            Err(_) => Err(Error::ParseError(format!("Invalid Content-Length value: {}", s)))
        }
    }
}

impl TypedHeaderTrait for ContentLength {
    type Name = HeaderName;

    fn header_name() -> Self::Name {
        HeaderName::ContentLength
    }

    fn to_header(&self) -> Header {
        Header::integer(Self::header_name(), self.0 as i64)
    }

    fn from_header(header: &Header) -> Result<Self> {
        if header.name != Self::header_name() {
            return Err(Error::InvalidHeader(
                format!("Expected {} header, got {}", Self::header_name(), header.name)
            ));
        }

        match &header.value {
            HeaderValue::Raw(bytes) => {
                if let Ok(s) = std::str::from_utf8(bytes) {
                    s.trim().parse::<u32>().map(ContentLength)
                        .map_err(|_| Error::InvalidHeader(
                            format!("Invalid integer value in {} header", Self::header_name())
                        ))
                } else {
                    Err(Error::InvalidHeader(
                        format!("Invalid UTF-8 in {} header", Self::header_name())
                    ))
                }
            },
            HeaderValue::ContentLength(content_length) => Ok(*content_length),
            _ => Err(Error::InvalidHeader(
                format!("Unexpected header value type for {}", Self::header_name())
            )),
        }
    }
}

// TODO: Implement methods if needed 