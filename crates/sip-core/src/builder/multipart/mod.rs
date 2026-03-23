pub mod part_builder;
pub mod builder;
#[cfg(test)]
mod tests;

pub use part_builder::MultipartPartBuilder;
pub use builder::{MultipartBuilder, MultipartBuilt};

use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
use crate::types::header::{Header, HeaderName};
use crate::types::TypedHeader;
use bytes::Bytes;
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;
use std::iter;

/// # Multipart MIME Builder for SIP Messages
///
/// This module provides a builder API for creating multipart MIME bodies in SIP messages,
/// following the standards defined in [RFC 5621](https://datatracker.ietf.org/doc/html/rfc5621)
/// and [RFC 2046](https://datatracker.ietf.org/doc/html/rfc2046).
///
/// ## What is Multipart MIME?
///
/// Multipart MIME allows a SIP message to include multiple content parts with different 
/// Content-Types in a single message body. Each part has its own headers and content,
/// and the parts are separated by a boundary string. This is particularly useful in SIP for:
///
/// - **Multiple related contents**: Combining SDP (Session Description Protocol) with 
///   other content types such as XML metadata or application data
/// - **Alternative representations**: Providing the same content in different formats
///   (e.g., plain text and HTML)
/// - **Mixed content**: Including different types of content like text, images, and 
///   application data in a single message
/// - **Rich media sharing**: Attaching files, images, or multimedia content to SIP messages
///
/// ## Structure of a Multipart MIME Message
///
/// A multipart MIME body consists of:
///
/// 1. A unique boundary string specified in the Content-Type header
/// 2. An optional preamble (text before the first boundary)
/// 3. Multiple body parts, each with their own headers and content
/// 4. An optional epilogue (text after the final boundary)
///
/// For example, a simple multipart message might look like this:
///
/// ```text
/// --boundary-abc123
/// Content-Type: text/plain
///
/// This is plain text content
/// --boundary-abc123
/// Content-Type: text/html
///
/// <html><body><p>This is HTML content</p></body></html>
/// --boundary-abc123--
/// ```
///
/// ## Using the MultipartBodyBuilder
///
/// The `MultipartBodyBuilder` makes it easy to construct properly formatted multipart bodies
/// without having to manually handle boundaries, Content-Type headers, or MIME formatting.
///
/// ### Basic Example
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart body with text and HTML parts
/// let multipart = MultipartBodyBuilder::new()
///     .add_text_part("This is the plain text version of the message")
///     .add_html_part("<html><body><p>This is the <b>HTML</b> version</p></body></html>")
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version_1_0()  // Adding MIME-Version is recommended for multipart bodies
///     .content_type(format!("multipart/alternative; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// ### Working with SDP and Other Media Types
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP description
/// let sdp = SdpBuilder::new("SIP Call")
///     .origin("user", "1234567890", "2", "IN", "IP4", "192.168.1.100")
///     .connection("IN", "IP4", "192.168.1.100")
///     .time("0", "0")
///     .media_audio(49170, "RTP/AVP")
///         .formats(&["0", "8"]) // PCMU and PCMA
///         .rtpmap("0", "PCMU/8000")
///         .rtpmap("8", "PCMA/8000")
///         .done()
///     .build()
///     .unwrap();
///
/// // XML metadata about the call
/// let xml_metadata = r#"<?xml version="1.0"?>
/// <call-info xmlns="urn:example:call-info">
///   <source>Conference System</source>
///   <recording>true</recording>
///   <encryption>true</encryption>
/// </call-info>"#;
///
/// // Create a multipart body with both SDP and XML
/// let multipart = MultipartBodyBuilder::new()
///     .add_sdp_part(sdp.to_string())
///     .add_xml_part(xml_metadata)
///     .build();
///
/// // Create a SIP INVITE with the multipart body
/// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .mime_version_1_0()
///     .content_type(format!("multipart/mixed; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// ## Common Multipart MIME Content-Types
///
/// When using multipart content, you should specify the appropriate subtype in the Content-Type header:
///
/// - `multipart/mixed`: For mixed content types that don't have a particular relationship
/// - `multipart/alternative`: For different representations of the same information
/// - `multipart/related`: For related content where one part references others
/// - `multipart/form-data`: For form submissions (less common in SIP)
///
/// ## Handling Boundaries
///
/// The `MultipartBodyBuilder` automatically generates a random boundary string, but you can
/// specify your own if needed using the `boundary()` method. Boundaries must be chosen to 
/// not appear in any of the part contents.
///
/// ## Best Practices
///
/// 1. Always include a MIME-Version header (usually "1.0") when using multipart bodies
/// 2. Use meaningful Content-Type headers in each part
/// 3. Choose the appropriate multipart subtype for your use case
/// 4. Keep parts reasonably sized to avoid fragmentation issues
/// 5. Consider message size limits in your SIP infrastructure
///
/// For detailed implementation information, see the documentation for specific methods below.
///
/// # Examples
///
/// ## Text and HTML Alternative
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart body with text and HTML parts
/// let multipart = MultipartBodyBuilder::new()
///     .add_text_part("This is the plain text version of the message")
///     .add_html_part("<html><body><p>This is the <b>HTML</b> version</p></body></html>")
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version_1_0()
///     .content_type(format!("multipart/alternative; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// ## Custom Headers and Content-ID
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::types::header::{Header, HeaderName};
/// use bytes::Bytes;
///
/// // Create a multipart body with custom headers
/// let multipart = MultipartBodyBuilder::new()
///     .add_part(
///         vec![
///             Header::text(HeaderName::ContentType, "text/plain"),
///             Header::text(HeaderName::ContentDisposition, "render; handling=optional"),
///         ],
///         Bytes::from("This is text with custom headers")
///     )
///     .add_image_part(
///         "image/jpeg",
///         Bytes::from(&[0xFF, 0xD8, 0xFF, 0xE0][..]), // Just a sample (not real JPEG data)
///         Some("image1@example.com") // Content-ID
///     )
///     .build();
/// ```
///
/// ## With Preamble and Epilogue
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
///
/// let multipart = MultipartBodyBuilder::new()
///     .preamble("This is a multipart message in MIME format.")
///     .add_text_part("Plain text content")
///     .add_json_part(r#"{"name":"John Doe","type":"contact"}"#)
///     .epilogue("End of multipart message.")
///     .build();
/// ```
/// 
/// ## SIP INVITE with SDP and Additional Information
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart body
/// let sdp = "v=0\r\no=user 123456 654321 IN IP4 192.168.0.1\r\ns=A SIP Call\r\nc=IN IP4 192.168.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0 8\r\n";
/// let info = "Call initiated by conference system";
///
/// let multipart = MultipartBodyBuilder::new()
///     .add_sdp_part(sdp)
///     .add_text_part(info)
///     .build();
///
/// // Create a SIP INVITE with multipart
/// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .mime_version_1_0()
///     .content_type(format!("multipart/mixed; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// For more examples, see the documentation for individual methods and the unit tests.
#[derive(Debug, Clone, Default)]
pub struct MultipartBodyBuilder {
    /// The boundary string used to separate parts
    boundary: Option<String>,
    /// The MIME parts to include in the body
    parts: Vec<MimePart>,
    /// Optional preamble text
    preamble: Option<Bytes>,
    /// Optional epilogue text
    epilogue: Option<Bytes>,
}

impl MultipartBodyBuilder {
    /// Creates a new MultipartBodyBuilder with a random boundary.
    ///
    /// The random boundary will be a string of the form "boundary-xxxxxxxx" where
    /// x is a random alphanumeric character. This helps ensure that the boundary
    /// doesn't accidentally appear in the body content.
    ///
    /// # Returns
    ///
    /// A new MultipartBodyBuilder instance
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new();
    /// // Builder is ready to add parts
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a custom boundary string for the multipart body.
    ///
    /// By default, a random boundary is generated. This method allows you to
    /// specify a custom boundary if needed.
    ///
    /// # Parameters
    ///
    /// - `boundary`: The boundary string to use
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .boundary("custom-boundary-string");
    /// ```
    pub fn boundary(mut self, boundary: impl Into<String>) -> Self {
        self.boundary = Some(boundary.into());
        self
    }

    /// Sets the preamble text that appears before the first boundary.
    ///
    /// The preamble is typically used to provide information to recipients
    /// who might not be able to handle multipart messages.
    ///
    /// # Parameters
    ///
    /// - `preamble`: The preamble text
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .preamble("This is a multipart message in MIME format.");
    /// ```
    pub fn preamble(mut self, preamble: impl Into<String>) -> Self {
        self.preamble = Some(Bytes::from(preamble.into()));
        self
    }

    /// Sets the epilogue text that appears after the final boundary.
    ///
    /// The epilogue is typically ignored by MIME processors but can
    /// contain information for non-MIME clients.
    ///
    /// # Parameters
    ///
    /// - `epilogue`: The epilogue text
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .epilogue("End of multipart message.");
    /// ```
    pub fn epilogue(mut self, epilogue: impl Into<String>) -> Self {
        self.epilogue = Some(Bytes::from(epilogue.into()));
        self
    }

    /// Adds a generic MIME part with custom headers and content.
    ///
    /// This is the most flexible method for adding parts, allowing you to
    /// specify custom headers and raw content.
    ///
    /// # Parameters
    ///
    /// - `headers`: Vector of headers for the part
    /// - `content`: Raw content bytes
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    /// use rvoip_sip_core::types::header::{Header, HeaderName};
    /// use bytes::Bytes;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_part(
    ///         vec![
    ///             Header::text(HeaderName::ContentType, "text/plain"),
    ///             Header::text(HeaderName::ContentDisposition, "inline"),
    ///         ],
    ///         Bytes::from("This is a custom part with custom headers")
    ///     );
    /// ```
    pub fn add_part(mut self, headers: Vec<Header>, content: Bytes) -> Self {
        let mut part = MimePart::new();
        part.headers = headers;
        part.raw_content = content;
        self.parts.push(part);
        self
    }

    /// Adds a text/plain MIME part.
    ///
    /// This is a convenience method for adding a plain text part with the
    /// appropriate Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `text`: The text content for the part
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_text_part("This is plain text content");
    /// ```
    pub fn add_text_part(self, text: impl Into<String>) -> Self {
        let headers = vec![Header::text(HeaderName::ContentType, "text/plain")];
        self.add_part(headers, Bytes::from(text.into()))
    }

    /// Adds a text/html MIME part.
    ///
    /// This is a convenience method for adding an HTML part with the
    /// appropriate Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `html`: The HTML content for the part
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_html_part("<html><body><p>This is HTML content</p></body></html>");
    /// ```
    pub fn add_html_part(self, html: impl Into<String>) -> Self {
        let headers = vec![Header::text(HeaderName::ContentType, "text/html")];
        self.add_part(headers, Bytes::from(html.into()))
    }

    /// Adds an application/json MIME part.
    ///
    /// This is a convenience method for adding a JSON part with the
    /// appropriate Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `json`: The JSON content for the part
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_json_part(r#"{"message": "Hello", "from": "Alice"}"#);
    /// ```
    pub fn add_json_part(self, json: impl Into<String>) -> Self {
        let headers = vec![Header::text(HeaderName::ContentType, "application/json")];
        self.add_part(headers, Bytes::from(json.into()))
    }

    /// Adds an application/xml MIME part.
    ///
    /// This is a convenience method for adding an XML part with the
    /// appropriate Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `xml`: The XML content for the part
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_xml_part(r#"<?xml version="1.0"?><root><node>value</node></root>"#);
    /// ```
    pub fn add_xml_part(self, xml: impl Into<String>) -> Self {
        let headers = vec![Header::text(HeaderName::ContentType, "application/xml")];
        self.add_part(headers, Bytes::from(xml.into()))
    }

    /// Adds an application/sdp MIME part.
    ///
    /// This is a convenience method for adding an SDP part with the
    /// appropriate Content-Type header.
    ///
    /// # Parameters
    ///
    /// - `sdp`: The SDP content for the part (as string)
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    ///
    /// // Create an SDP session
    /// let sdp = SdpBuilder::new("Call with Alice")
    ///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.0.2.1")
    ///     .connection("IN", "IP4", "192.0.2.1")
    ///     .time("0", "0")
    ///     .media_audio(49170, "RTP/AVP")
    ///         .formats(&["0", "8"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .rtpmap("8", "PCMA/8000")
    ///         .done()
    ///     .build()
    ///     .unwrap();
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_sdp_part(sdp.to_string());
    /// ```
    pub fn add_sdp_part(self, sdp: impl Into<String>) -> Self {
        let headers = vec![Header::text(HeaderName::ContentType, "application/sdp")];
        self.add_part(headers, Bytes::from(sdp.into()))
    }

    /// Adds an image MIME part.
    ///
    /// This method allows you to add an image with the appropriate Content-Type
    /// and optionally a Content-ID for referencing the image from other parts.
    ///
    /// # Parameters
    ///
    /// - `image_type`: The image MIME type (e.g., "image/jpeg", "image/png")
    /// - `image_data`: The raw image data
    /// - `content_id`: Optional Content-ID for the image
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    /// use bytes::Bytes;
    ///
    /// // In a real application, this would be actual image data
    /// let image_data = Bytes::from_static(&[0xFF, 0xD8, 0xFF, 0xE0]); // Not real JPEG data
    ///
    /// let builder = MultipartBodyBuilder::new()
    ///     .add_image_part("image/jpeg", image_data, Some("image1@example.com"));
    /// ```
    pub fn add_image_part(
        self,
        image_type: impl Into<String>,
        image_data: Bytes,
        content_id: Option<impl Into<String>>,
    ) -> Self {
        let mut headers = vec![Header::text(HeaderName::ContentType, image_type.into())];
        
        // Create a content-id header if needed
        if let Some(id) = content_id {
            let content_id_header = Header::text(HeaderName::Other("Content-ID".to_string()), format!("<{}>", id.into()));
            headers.push(content_id_header);
        }
        
        self.add_part(headers, image_data)
    }

    /// Adds a MIME part to the builder.
    ///
    /// Internal helper to add a part directly.
    ///
    /// # Parameters
    ///
    /// - `part`: The MimePart to add
    ///
    /// # Returns
    ///
    /// Self for method chaining
    fn add_mime_part(mut self, part: MimePart) -> Self {
        self.parts.push(part);
        self
    }
    
    /// Adds multiple MIME parts from a vector.
    ///
    /// Internal helper method for converting between builder types.
    ///
    /// # Parameters
    ///
    /// - `parts`: Vector of MimePart instances to add
    ///
    /// # Returns
    ///
    /// Self for method chaining
    fn add_parts_from_vec(mut self, parts: Vec<MimePart>) -> Self {
        self.parts.extend(parts);
        self
    }

    /// Generates a random boundary string if one hasn't been set.
    ///
    /// The random boundary has the format "boundary-" followed by 16 random
    /// alphanumeric characters.
    ///
    /// # Returns
    ///
    /// A random boundary string
    fn generate_boundary(&self) -> String {
        if let Some(boundary) = &self.boundary {
            return boundary.clone();
        }
        
        let random_suffix: String = iter::repeat(())
            .map(|()| thread_rng().sample(Alphanumeric))
            .map(char::from)
            .take(16)
            .collect();
            
        format!("boundary-{}", random_suffix)
    }

    /// Builds a MultipartBody from this builder.
    ///
    /// # Returns
    ///
    /// A MultipartBody instance with all the parts and properties set
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let multipart = MultipartBodyBuilder::new()
    ///     .add_text_part("Plain text content")
    ///     .add_html_part("<html><body><p>HTML content</p></body></html>")
    ///     .build();
    ///
    /// assert_eq!(multipart.parts.len(), 2);
    /// ```
    pub fn build(self) -> MultipartBody {
        let boundary = self.generate_boundary();
        
        MultipartBody {
            boundary,
            parts: self.parts,
            preamble: self.preamble,
            epilogue: self.epilogue,
        }
    }
}

impl MultipartBody {
    /// Converts the multipart body to a string representation.
    ///
    /// This method serializes the entire multipart body, including the boundary
    /// markers, headers for each part, and the content of each part.
    ///
    /// # Returns
    ///
    /// A string containing the serialized multipart body
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
    ///
    /// let multipart = MultipartBodyBuilder::new()
    ///     .boundary("simple-boundary")
    ///     .add_text_part("Text content")
    ///     .build();
    ///
    /// let body_string = multipart.to_string();
    /// assert!(body_string.contains("--simple-boundary"));
    /// assert!(body_string.contains("Content-Type: text/plain"));
    /// assert!(body_string.contains("Text content"));
    /// assert!(body_string.contains("--simple-boundary--"));
    /// ```
    pub fn to_string(&self) -> String {
        let mut result = String::new();
        
        // Add preamble if present
        if let Some(preamble) = &self.preamble {
            if let Ok(text) = std::str::from_utf8(preamble) {
                result.push_str(text);
                result.push_str("\r\n");
            }
        }
        
        // Add each part
        for part in &self.parts {
            // Add boundary
            result.push_str("--");
            result.push_str(&self.boundary);
            result.push_str("\r\n");
            
            // Add headers
            for header in &part.headers {
                result.push_str(&header.to_string());
                result.push_str("\r\n");
            }
            
            // Empty line between headers and content
            result.push_str("\r\n");
            
            // Add content
            if let Ok(text) = std::str::from_utf8(&part.raw_content) {
                result.push_str(text);
            } else {
                // For binary data, we'd ideally use base64 encoding here
                // but for simplicity we'll just skip it in this example
                result.push_str("[Binary data not shown]");
            }
            
            result.push_str("\r\n");
        }
        
        // Add final boundary
        result.push_str("--");
        result.push_str(&self.boundary);
        result.push_str("--\r\n");
        
        // Add epilogue if present
        if let Some(epilogue) = &self.epilogue {
            if let Ok(text) = std::str::from_utf8(epilogue) {
                result.push_str(text);
            }
        }
        
        result
    }
}

