//! # SIP Multipart MIME Bodies
//!
//! This module provides types for handling multipart MIME bodies in SIP messages, as described in
//! [RFC 5621](https://datatracker.ietf.org/doc/html/rfc5621).
//!
//! Multipart MIME allows a SIP message to contain multiple body parts with different content types,
//! separated by a boundary string. This is commonly used for:
//!
//! - Combining SDP with other content types
//! - Including alternative representations of the same content
//! - Attaching files or additional information to SIP messages
//! - Supporting mixed content types in a single message
//!
//! ## Structure of a Multipart Body
//!
//! A multipart MIME body consists of:
//!
//! 1. An optional preamble (text before the first boundary)
//! 2. One or more body parts, each with its own headers and content
//! 3. An optional epilogue (text after the last boundary)
//!
//! Each part is separated by a boundary string, which is specified in the Content-Type header
//! of the SIP message.
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use bytes::Bytes;
//!
//! // Create a multipart body with a boundary
//! let mut body = MultipartBody::new("boundary-string-1234");
//!
//! // Create a MIME part with headers and content
//! let mut part = MimePart::new();
//! part.headers.push(Header::new(
//!     HeaderName::ContentType,
//!     "text/plain".into()
//! ));
//! part.raw_content = Bytes::from("This is the text content");
//!
//! // Add the part to the multipart body
//! body.add_part(part);
//! ```

use bytes::Bytes;
use crate::types::header::{Header, HeaderName};
use crate::types::content_type::ContentType;
use crate::error::{Error, Result};
use crate::parser;
use crate::SdpSession;
use std::fmt;
use serde::{Deserialize, Serialize};

/// Represents a parsed MIME part.
///
/// A MIME part is a section within a multipart MIME message that contains
/// its own set of headers and body content. Each part is self-contained
/// and can have a different content type.
///
/// In SIP applications, MIME parts are commonly used to include different types
/// of content in a single message, such as SDP session descriptions, XML data,
/// or text content.
///
/// # Structure
///
/// Each MIME part consists of:
/// - Headers that describe the content (e.g., Content-Type, Content-Disposition)
/// - Raw binary content
/// - Optionally parsed content based on the Content-Type
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use bytes::Bytes;
///
/// // Create a new MIME part
/// let mut part = MimePart::new();
///
/// // Add headers to describe the content
/// part.headers.push(Header::new(
///     HeaderName::ContentType,
///     "application/sdp".into()
/// ));
///
/// // Set the content
/// part.raw_content = Bytes::from("v=0\r\no=- 1234 1234 IN IP4 127.0.0.1\r\ns=Example\r\n");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MimePart {
    /// Headers associated with this part.
    pub headers: Vec<Header>,
    /// The raw content bytes of this part.
    pub raw_content: Bytes,
    /// Optionally parsed content based on Content-Type.
    pub parsed_content: Option<ParsedBody>,
}

impl MimePart {
    /// Creates a new, empty MIME part.
    ///
    /// This constructor initializes a MIME part with empty headers and content.
    /// You'll typically add headers and content to the part after creation.
    ///
    /// # Returns
    ///
    /// A new `MimePart` instance with empty headers and content
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let part = MimePart::new();
    /// assert!(part.headers.is_empty());
    /// assert!(part.raw_content.is_empty());
    /// assert!(part.parsed_content.is_none());
    /// ```
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            raw_content: Bytes::new(),
            parsed_content: None,
        }
    }

    /// Returns the Content-Type value for this MIME part, if present.
    ///
    /// This method searches the part's headers for a Content-Type header
    /// and returns its value as a string.
    ///
    /// # Returns
    ///
    /// - `Some(String)` containing the Content-Type if found
    /// - `None` if no Content-Type header is present
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut part = MimePart::new();
    /// assert!(part.content_type().is_none());
    ///
    /// // Add a Content-Type header
    /// part.headers.push(Header::new(
    ///     HeaderName::ContentType,
    ///     "application/sdp".into()
    /// ));
    ///
    /// assert_eq!(part.content_type(), Some("application/sdp".to_string()));
    /// ```
    pub fn content_type(&self) -> Option<String> {
        self.headers.iter()
            .find(|h| h.name == HeaderName::ContentType)
            .and_then(|h| h.value.as_text())
            .map(|s| s.to_string())
    }
}

impl Default for MimePart {
    /// Returns a default empty MIME part.
    ///
    /// This is equivalent to calling `MimePart::new()`.
    ///
    /// # Returns
    ///
    /// A new empty `MimePart` instance
    fn default() -> Self {
        Self::new()
    }
}

/// Represents the different types of parsed body content.
///
/// This enum is used to store the parsed content of a MIME part,
/// depending on its Content-Type. It allows the application to work
/// with structured content instead of raw bytes when the content
/// type is recognized and supported.
///
/// # Variants
///
/// - `Sdp`: Session Description Protocol data (application/sdp)
/// - `Text`: Plain text content (text/plain)
/// - `Other`: Other content types stored as raw bytes
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use bytes::Bytes;
///
/// // Creating SDP parsed content
/// let sdp_session = SdpSession::default(); // In practice, this would be a parsed SDP
/// let parsed_sdp = ParsedBody::Sdp(sdp_session);
///
/// // Creating text parsed content
/// let parsed_text = ParsedBody::Text("Hello, SIP world!".to_string());
///
/// // Creating other content
/// let raw_bytes = Bytes::from(&b"Some binary data"[..]);
/// let parsed_other = ParsedBody::Other(raw_bytes);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedBody {
    /// Session Description Protocol data.
    Sdp(SdpSession),
    /// Plain text content.
    Text(String),
    /// Other content types stored as raw bytes.
    Other(Bytes),
}

/// Represents a parsed multipart MIME body.
///
/// A multipart MIME body contains multiple MIME parts, each with its own headers
/// and content. The parts are separated by a boundary string, which is specified
/// in the Content-Type header of the SIP message.
///
/// Multipart bodies are used in SIP for various purposes, such as including
/// both SDP and XML content in a single message, or providing alternative
/// representations of the same content.
///
/// # Structure
///
/// A multipart body consists of:
/// - A boundary string that separates the parts
/// - One or more MIME parts
/// - An optional preamble (content before the first boundary)
/// - An optional epilogue (content after the last boundary)
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use bytes::Bytes;
///
/// // Create a multipart body
/// let mut body = MultipartBody::new("boundary-string-1234");
///
/// // Create and add MIME parts
/// let mut text_part = MimePart::new();
/// text_part.headers.push(Header::new(
///     HeaderName::ContentType,
///     "text/plain".into()
/// ));
/// text_part.raw_content = Bytes::from("This is the text content");
///
/// let mut sdp_part = MimePart::new();
/// sdp_part.headers.push(Header::new(
///     HeaderName::ContentType,
///     "application/sdp".into()
/// ));
/// sdp_part.raw_content = Bytes::from("v=0\r\no=- 1234 1234 IN IP4 127.0.0.1\r\n");
///
/// // Add the parts to the multipart body
/// body.add_part(text_part);
/// body.add_part(sdp_part);
///
/// // Verify the structure
/// assert_eq!(body.boundary, "boundary-string-1234");
/// assert_eq!(body.parts.len(), 2);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MultipartBody {
    /// The boundary string used to separate parts.
    pub boundary: String,
    /// The MIME parts contained in the body.
    pub parts: Vec<MimePart>,
    /// Content appearing before the first boundary (optional).
    pub preamble: Option<Bytes>,
    /// Content appearing after the last boundary (optional).
    pub epilogue: Option<Bytes>,
}

impl MultipartBody {
    /// Creates a new MultipartBody with a given boundary.
    ///
    /// The boundary string is used to separate the different parts in the
    /// multipart body. It should be unique and not appear in any of the
    /// parts' content.
    ///
    /// # Parameters
    ///
    /// - `boundary`: The boundary string to use for separating parts
    ///
    /// # Returns
    ///
    /// A new `MultipartBody` instance with the specified boundary and no parts
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a multipart body with a boundary
    /// let body = MultipartBody::new("boundary-string-1234");
    ///
    /// assert_eq!(body.boundary, "boundary-string-1234");
    /// assert!(body.parts.is_empty());
    /// assert!(body.preamble.is_none());
    /// assert!(body.epilogue.is_none());
    /// ```
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            parts: Vec::new(),
            preamble: None,
            epilogue: None,
        }
    }

    /// Adds a MIME part to the body.
    ///
    /// This method appends a new MIME part to the multipart body.
    /// The parts are stored in the order they're added, which is
    /// important for the final serialized representation.
    ///
    /// # Parameters
    ///
    /// - `part`: The MIME part to add to the body
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use bytes::Bytes;
    ///
    /// // Create a multipart body
    /// let mut body = MultipartBody::new("boundary-string-1234");
    ///
    /// // Create a MIME part
    /// let mut part = MimePart::new();
    /// part.headers.push(Header::new(
    ///     HeaderName::ContentType,
    ///     "text/plain".into()
    /// ));
    /// part.raw_content = Bytes::from("This is the text content");
    ///
    /// // Add the part to the body
    /// body.add_part(part);
    ///
    /// assert_eq!(body.parts.len(), 1);
    /// assert_eq!(
    ///     body.parts[0].content_type(),
    ///     Some("text/plain".to_string())
    /// );
    /// ```
    pub fn add_part(&mut self, part: MimePart) {
        self.parts.push(part);
    }
}

// TODO: Add methods for serialization if needed. 