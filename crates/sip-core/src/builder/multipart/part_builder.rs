//! MultipartPartBuilder - builder for individual MIME parts

use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
use crate::types::header::{Header, HeaderName};
use crate::types::TypedHeader;
use bytes::Bytes;
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use std::iter;

/// Builder for creating individual parts of a multipart MIME message.
///
/// This builder provides a fluent API for constructing individual parts of a multipart MIME
/// message. Each part can have its own set of headers and content, allowing for complex
/// structured messages that follow the MIME specification described in 
/// [RFC 2045](https://tools.ietf.org/html/rfc2045) and [RFC 2046](https://tools.ietf.org/html/rfc2046).
///
/// ## Key Headers for MIME Parts
///
/// - **Content-Type**: Identifies the media type of the content (e.g., "text/plain", "application/sdp")
/// - **Content-ID**: Provides a unique identifier for the part, which can be referenced by other parts
/// - **Content-Disposition**: Indicates how the part should be presented (e.g., "inline", "attachment")
/// - **Content-Transfer-Encoding**: Specifies how the content is encoded (e.g., "base64", "quoted-printable")
///
/// ## Special Considerations for SIP
///
/// In SIP applications, certain content types and part configurations are commonly used:
///
/// - **application/sdp** parts with `content-disposition: session` for SDP session descriptions
/// - **image/** parts with `content-transfer-encoding: base64` for embedding images
/// - **application/pidf+xml** for presence information
/// - **text/plain** and **text/html** for message content in alternative formats
///
/// These can be easily created with the appropriate methods in this builder.
///
/// # Examples
///
/// ## Basic Text Part
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
///
/// let part = MultipartPartBuilder::new()
///     .content_type("text/plain")
///     .body("This is a plain text message")
///     .build();
/// ```
///
/// ## SDP Session Description Part
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP session description
/// let sdp = SdpBuilder::new("SIP Call")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "203.0.113.10")
///     .connection("IN", "IP4", "203.0.113.10")
///     .time("0", "0")
///     .media_audio(49170, "RTP/AVP")
///         .formats(&["0", "8"])
///         .rtpmap("0", "PCMU/8000")
///         .rtpmap("8", "PCMA/8000")
///         .done()
///     .build()
///     .unwrap();
///
/// // Create the SDP part with session disposition
/// let sdp_part = MultipartPartBuilder::new()
///     .content_type("application/sdp")
///     .content_disposition("session")  // Indicates this is the session description
///     .body(sdp.to_string())
///     .build();
/// ```
///
/// ## Image Part with Content-ID
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
///
/// // Create an image part with Content-ID for referencing
/// let image_part = MultipartPartBuilder::new()
///     .content_type("image/jpeg")
///     .content_id("<image1@example.com>")  // Can be referenced with "cid:image1@example.com"
///     .content_transfer_encoding("base64")  // Base64 encoding for binary data
///     .content_disposition("inline")  // Should be displayed inline when possible
///     .body("/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAoHBwgHBgoICAgLCgoLDh...")  // Base64 image data (truncated)
///     .build();
/// ```
///
/// ## XML Metadata Part with Custom Headers
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
/// use rvoip_sip_core::types::header::{Header, HeaderName};
///
/// // XML metadata about a call
/// let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
/// <call-metadata xmlns="urn:example:call-info">
///   <priority>high</priority>
///   <department>sales</department>
///   <account-id>12345</account-id>
/// </call-metadata>"#;
///
/// // Create a part with the XML and custom headers
/// let part = MultipartPartBuilder::new()
///     .content_type("application/call-metadata+xml")
///     .content_disposition("handling=optional")  // Client can ignore if not understood
///     .body(xml)
///     .build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct MultipartPartBuilder {
    headers: Vec<Header>,
    content: Option<String>,
}

impl MultipartPartBuilder {
    /// Creates a new empty MultipartPartBuilder.
    ///
    /// # Returns
    ///
    /// A new MultipartPartBuilder instance
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the Content-Type header for this part.
    ///
    /// # Parameters
    ///
    /// - `content_type`: The content type value (e.g., "text/plain", "application/sdp")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new()
    ///     .content_type("text/plain");
    /// ```
    pub fn content_type(mut self, content_type: impl Into<String>) -> Self {
        self.headers.push(Header::text(HeaderName::ContentType, content_type.into()));
        self
    }

    /// Sets the Content-ID header for this part.
    ///
    /// Content-ID headers are used to uniquely identify parts in a multipart message,
    /// especially for referencing them from other parts (like in multipart/related).
    ///
    /// # Parameters
    ///
    /// - `content_id`: The content ID value (e.g., "image1@example.com")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new()
    ///     .content_id("<image1@example.com>");
    /// ```
    pub fn content_id(mut self, content_id: impl Into<String>) -> Self {
        let id = content_id.into();
        // Make sure the ID is properly formatted with angle brackets
        let formatted_id = if id.starts_with('<') && id.ends_with('>') {
            id
        } else {
            format!("<{}>", id)
        };
        self.headers.push(Header::text(HeaderName::Other("Content-ID".to_string()), formatted_id));
        self
    }

    /// Sets the Content-Disposition header for this part.
    ///
    /// Content-Disposition can specify how the part should be handled, such as
    /// "inline", "attachment", "session", etc.
    ///
    /// # Parameters
    ///
    /// - `disposition`: The disposition value (e.g., "inline", "attachment", "session")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new()
    ///     .content_disposition("attachment; filename=document.pdf");
    /// ```
    pub fn content_disposition(mut self, disposition: impl Into<String>) -> Self {
        self.headers.push(Header::text(HeaderName::ContentDisposition, disposition.into()));
        self
    }

    /// Sets the Content-Transfer-Encoding header for this part.
    ///
    /// This is used to specify how the content is encoded, which can be useful
    /// for binary data like images.
    ///
    /// # Parameters
    ///
    /// - `encoding`: The encoding value (e.g., "base64", "quoted-printable")
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new()
    ///     .content_transfer_encoding("base64");
    /// ```
    pub fn content_transfer_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.headers.push(Header::text(
            HeaderName::Other("Content-Transfer-Encoding".to_string()), 
            encoding.into()
        ));
        self
    }

    /// Sets the content body for this part.
    ///
    /// # Parameters
    ///
    /// - `body`: The content body
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let builder = MultipartPartBuilder::new()
    ///     .body("This is the content body");
    /// ```
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.content = Some(body.into());
        self
    }

    /// Builds a MimePart from this builder.
    ///
    /// # Returns
    ///
    /// A MimePart instance with the headers and content set
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartPartBuilder;
    ///
    /// let part = MultipartPartBuilder::new()
    ///     .content_type("text/plain")
    ///     .body("This is the content")
    ///     .build();
    /// ```
    pub fn build(self) -> MimePart {
        let content = self.content.unwrap_or_default();
        
        MimePart {
            headers: self.headers,
            raw_content: Bytes::from(content),
            parsed_content: None,
        }
    }
}
