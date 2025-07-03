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

/// Builder for creating multipart MIME bodies with multiple parts.
///
/// This builder provides a higher-level interface than MultipartBodyBuilder,
/// with convenient factory methods for common multipart types and better
/// integration with MultipartPartBuilder. It's designed to make creating SIP
/// messages with complex multipart content easier and more intuitive.
///
/// ## Key Features:
///
/// - Specialized constructors for common multipart types: `mixed()`, `alternative()`, `related()`
/// - Simple API for adding MIME parts using `MultipartPartBuilder`
/// - Easy integration with SIP message builders via `content_type()` and `body()` methods
/// - Support for preamble, epilogue, and custom boundaries
/// - Type parameters for multipart/related content
///
/// ## Common Multipart Types in SIP
///
/// - **multipart/mixed**: For mixed content with no special relationship (most common)
/// - **multipart/alternative**: For alternative representations of the same content
/// - **multipart/related**: For related content where parts reference each other
///
/// ## Real-world SIP Multipart Scenarios
///
/// - SIP INVITE with SDP and call metadata (multipart/mixed)
/// - SIP MESSAGE with alternative text and HTML content (multipart/alternative)
/// - SIP PUBLISH with PIDF presence document and referenced avatar image (multipart/related)
/// - SIP INFO with multiple media control commands
/// - SIP NOTIFY with document updates containing inline resources
///
/// # Examples
///
/// ## Basic multipart/mixed with text and image
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart/mixed body with text and image (base64 encoded)
/// let multipart = MultipartBuilder::mixed()
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("text/plain")
///             .body("Check out this image I'm sending you!")
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("image/png")
///             .content_transfer_encoding("base64")
///             .content_id("<image1@example.com>")
///             .content_disposition("attachment; filename=logo.png")
///             .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
///             .build()
///     )
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version(1, 0)  // Required for multipart content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
///
/// ## INVITE with SDP and call metadata (real-world example)
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::builder::headers::{FromBuilderExt, ToBuilderExt, ContactBuilderExt};
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP offer
/// let sdp = SdpBuilder::new("SIP Call with Metadata")
///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "198.51.100.33")
///     .connection("IN", "IP4", "198.51.100.33")
///     .time("0", "0")
///     .media_audio(49170, "RTP/AVP")
///         .formats(&["0", "8", "96"])
///         .rtpmap("0", "PCMU/8000")
///         .rtpmap("8", "PCMA/8000")
///         .rtpmap("96", "opus/48000/2")
///         .ptime(20)
///         .done()
///     .build()
///     .unwrap();
///
/// // Create XML call metadata
/// let call_metadata = r#"<?xml version="1.0" encoding="UTF-8"?>
/// <call-metadata xmlns="urn:example:callmeta">
///   <call-type>support</call-type>
///   <priority>high</priority>
///   <reference>CASE-12345</reference>
///   <customer>
///     <account-id>ACC987654</account-id>
///     <membership-level>premium</membership-level>
///   </customer>
/// </call-metadata>"#;
///
/// // Create a multipart/mixed body with SDP and metadata
/// let multipart = MultipartBuilder::mixed()
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/sdp")
///             .content_disposition("session")
///             .body(sdp.to_string())
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/call-metadata+xml")
///             .content_disposition("handling=optional")
///             .body(call_metadata)
///             .build()
///     )
///     .build();
///
/// // Create a SIP INVITE with the multipart body
/// let invite = SimpleRequestBuilder::invite("sip:support@example.com").unwrap()
///     .from("Alice Smith", "sip:alice@example.com", Some("a73kssle"))
///     .to("Support", "sip:support@example.com", None)
///     .contact("sip:alice@198.51.100.33:5060", None)
///     .mime_version(1, 0)  // Required for multipart content
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
///
/// ## SIP PUBLISH with Presence and Referenced Avatar (multipart/related)
///
/// ```rust
/// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, MimeVersionBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart/related body with PIDF document that references an image
/// let multipart = MultipartBuilder::related()
///     .type_parameter("application/pidf+xml")  // The root document type
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("application/pidf+xml")
///             .content_id("<presence123@example.com>")
///             .body(r#"<?xml version="1.0" encoding="UTF-8"?>
/// <presence xmlns="urn:ietf:params:xml:ns:pidf" entity="sip:alice@example.com">
///   <tuple id="a1">
///     <status><basic>open</basic></status>
///     <note>Available</note>
///     <note>My avatar: <img src="cid:avatar123@example.com"/></note>
///   </tuple>
/// </presence>"#)
///             .build()
///     )
///     .add_part(
///         MultipartPartBuilder::new()
///             .content_type("image/jpeg")
///             .content_id("<avatar123@example.com>")
///             .content_transfer_encoding("base64")
///             .content_disposition("inline")
///             .body("/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAAoHBwgHBgoICAgLCgoLDh...")
///             .build()
///     )
///     .build();
///
/// // Create a SIP PUBLISH with the multipart body
/// let publish = SimpleRequestBuilder::new(Method::Publish, "sip:alice@example.com;method=PUBLISH").unwrap()
///     .mime_version(1, 0)
///     .content_type(&multipart.content_type())
///     .body(multipart.body())
///     .build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct MultipartBuilder {
    boundary: Option<String>,
    subtype: String,
    type_param: Option<String>,
    parts: Vec<MimePart>,
    preamble: Option<String>,
    epilogue: Option<String>,
}

impl MultipartBuilder {
    /// Creates a multipart/mixed builder.
    ///
    /// Multipart/mixed is used for content with different types that don't
    /// have a specific relationship to each other.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/mixed
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed();
    /// ```
    pub fn mixed() -> Self {
        Self {
            subtype: "mixed".to_string(),
            ..Default::default()
        }
    }

    /// Creates a multipart/alternative builder.
    ///
    /// Multipart/alternative is used when the same content is provided in
    /// different formats, with the last part being the preferred format.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/alternative
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::alternative();
    /// ```
    pub fn alternative() -> Self {
        Self {
            subtype: "alternative".to_string(),
            ..Default::default()
        }
    }

    /// Creates a multipart/related builder.
    ///
    /// Multipart/related is used when parts reference each other, such as an
    /// HTML document with embedded images.
    ///
    /// # Returns
    ///
    /// A new MultipartBuilder configured for multipart/related
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::related();
    /// ```
    pub fn related() -> Self {
        Self {
            subtype: "related".to_string(),
            ..Default::default()
        }
    }

    /// Sets a custom boundary string for the multipart body.
    ///
    /// By default, a random boundary is generated when building.
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
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .boundary("custom-boundary-123");
    /// ```
    pub fn boundary(mut self, boundary: impl Into<String>) -> Self {
        self.boundary = Some(boundary.into());
        self
    }

    /// Sets the type parameter for multipart/related content.
    ///
    /// The type parameter indicates the MIME type of the "root" part
    /// in a multipart/related body.
    ///
    /// # Parameters
    ///
    /// - `type_param`: The type parameter value
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::related()
    ///     .type_parameter("text/html");
    /// ```
    pub fn type_parameter(mut self, type_param: impl Into<String>) -> Self {
        self.type_param = Some(type_param.into());
        self
    }

    /// Sets the preamble text that appears before the first boundary.
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
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .preamble("This is a multipart message in MIME format.");
    /// ```
    pub fn preamble(mut self, preamble: impl Into<String>) -> Self {
        self.preamble = Some(preamble.into());
        self
    }

    /// Sets the epilogue text that appears after the final boundary.
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
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .epilogue("End of multipart message.");
    /// ```
    pub fn epilogue(mut self, epilogue: impl Into<String>) -> Self {
        self.epilogue = Some(epilogue.into());
        self
    }

    /// Adds a MIME part to the multipart body.
    ///
    /// # Parameters
    ///
    /// - `part`: The MimePart to add
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let part = MultipartPartBuilder::new()
    ///     .content_type("text/plain")
    ///     .body("This is text content")
    ///     .build();
    ///
    /// let builder = MultipartBuilder::mixed()
    ///     .add_part(part);
    /// ```
    pub fn add_part(mut self, part: MimePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Returns the Content-Type header value for this multipart body.
    ///
    /// This includes the multipart type, boundary parameter, and any other
    /// parameters like the type parameter for multipart/related.
    ///
    /// # Returns
    ///
    /// A string containing the Content-Type value
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::MultipartBuilder;
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .boundary("custom-boundary")
    ///     .build();
    ///
    /// let content_type = multipart.content_type();
    /// assert_eq!(content_type, "multipart/mixed; boundary=\"custom-boundary\"");
    /// ```
    pub fn content_type(&self) -> String {
        let mut content_type = format!("multipart/{}", self.subtype);
        
        // Add boundary parameter
        if let Some(boundary) = &self.boundary {
            content_type.push_str(&format!("; boundary=\"{}\"", boundary));
        }
        
        // Add type parameter for multipart/related
        if let Some(type_param) = &self.type_param {
            content_type.push_str(&format!("; type=\"{}\"", type_param));
        }
        
        content_type
    }

    /// Returns the body content for this multipart message.
    ///
    /// # Returns
    ///
    /// A string containing the serialized multipart body
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("text/plain")
    ///             .body("Text content")
    ///             .build()
    ///     )
    ///     .build();
    ///
    /// let body = multipart.body();
    /// assert!(body.contains("Content-Type: text/plain"));
    /// assert!(body.contains("Text content"));
    /// ```
    pub fn body(&self) -> String {
        let boundary = self.boundary.clone().unwrap_or_else(|| {
            let random_suffix: String = iter::repeat(())
                .map(|()| thread_rng().sample(Alphanumeric))
                .map(char::from)
                .take(16)
                .collect();
                
            format!("boundary-{}", random_suffix)
        });

        let mut body_builder = MultipartBodyBuilder::new()
            .boundary(boundary);
            
        // Add all parts
        let mut builder = body_builder;
        for part in &self.parts {
            builder = builder.add_mime_part(part.clone());
        }
        
        // Add preamble and epilogue if present
        if let Some(preamble) = &self.preamble {
            builder = builder.preamble(preamble);
        }
        
        if let Some(epilogue) = &self.epilogue {
            builder = builder.epilogue(epilogue);
        }
        
        builder.build().to_string()
    }

    /// Builds a MultipartBody from this builder.
    ///
    /// # Returns
    ///
    /// A MultipartBody instance
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::multipart::{MultipartBuilder, MultipartPartBuilder};
    ///
    /// let multipart = MultipartBuilder::mixed()
    ///     .add_part(
    ///         MultipartPartBuilder::new()
    ///             .content_type("text/plain")
    ///             .body("Text content")
    ///             .build()
    ///     )
    ///     .build();
    /// ```
    pub fn build(self) -> MultipartBuilt {
        let boundary = self.boundary.unwrap_or_else(|| {
            let random_suffix: String = iter::repeat(())
                .map(|()| thread_rng().sample(Alphanumeric))
                .map(char::from)
                .take(16)
                .collect();
                
            format!("boundary-{}", random_suffix)
        });

        let inner_multipart = MultipartBody {
            boundary: boundary.clone(),
            parts: self.parts,
            preamble: self.preamble.map(Bytes::from),
            epilogue: self.epilogue.map(Bytes::from),
        };

        MultipartBuilt {
            boundary,
            subtype: self.subtype,
            type_param: self.type_param,
            inner_multipart,
        }
    }
}

/// Represents a built multipart MIME body with convenient methods for integration with SIP messages.
#[derive(Debug, Clone)]
pub struct MultipartBuilt {
    boundary: String,
    subtype: String,
    type_param: Option<String>,
    inner_multipart: MultipartBody,
}

impl MultipartBuilt {
    /// Returns the Content-Type header value for this multipart body.
    ///
    /// This includes the multipart type, boundary parameter, and any other
    /// parameters like the type parameter for multipart/related.
    ///
    /// # Returns
    ///
    /// A string containing the Content-Type value
    pub fn content_type(&self) -> String {
        let mut content_type = format!("multipart/{}", self.subtype);
        
        // Add boundary parameter
        content_type.push_str(&format!("; boundary=\"{}\"", self.boundary));
        
        // Add type parameter for multipart/related
        if let Some(type_param) = &self.type_param {
            content_type.push_str(&format!("; type=\"{}\"", type_param));
        }
        
        content_type
    }

    /// Returns the body content for this multipart message.
    ///
    /// # Returns
    ///
    /// A string containing the serialized multipart body
    pub fn body(&self) -> String {
        self.inner_multipart.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::builder::headers::ContentTypeBuilderExt;
    use crate::types::Method;
    use crate::sdp::SdpBuilder;
    
    #[test]
    fn test_basic_builder() {
        let multipart = MultipartBodyBuilder::new()
            .add_text_part("Text content")
            .add_html_part("<html><body><p>HTML content</p></body></html>")
            .build();
            
        assert_eq!(multipart.parts.len(), 2);
        assert!(multipart.boundary.starts_with("boundary-"));
        
        // Check first part (text)
        assert_eq!(multipart.parts[0].content_type().unwrap(), "text/plain");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            "Text content"
        );
        
        // Check second part (HTML)
        assert_eq!(multipart.parts[1].content_type().unwrap(), "text/html");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[1].raw_content).unwrap(),
            "<html><body><p>HTML content</p></body></html>"
        );
    }
    
    #[test]
    fn test_custom_boundary() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("custom-test-boundary")
            .add_text_part("Content")
            .build();
            
        assert_eq!(multipart.boundary, "custom-test-boundary");
    }
    
    #[test]
    fn test_preamble_epilogue() {
        let multipart = MultipartBodyBuilder::new()
            .preamble("This is the preamble")
            .epilogue("This is the epilogue")
            .add_text_part("Content")
            .build();
            
        assert_eq!(
            std::str::from_utf8(&multipart.preamble.unwrap()).unwrap(),
            "This is the preamble"
        );
        assert_eq!(
            std::str::from_utf8(&multipart.epilogue.unwrap()).unwrap(),
            "This is the epilogue"
        );
    }
    
    #[test]
    fn test_json_part() {
        let json = r#"{"name":"Alice","age":30}"#;
        let multipart = MultipartBodyBuilder::new()
            .add_json_part(json)
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/json");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            json
        );
    }
    
    #[test]
    fn test_xml_part() {
        let xml = r#"<?xml version="1.0"?><root><node>value</node></root>"#;
        let multipart = MultipartBodyBuilder::new()
            .add_xml_part(xml)
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/xml");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            xml
        );
    }
    
    #[test]
    fn test_sdp_part() {
        let sdp = SdpBuilder::new("Test Session")
            .origin("test", "123456", "789012", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .done()
            .build()
            .unwrap();
            
        let multipart = MultipartBodyBuilder::new()
            .add_sdp_part(sdp.to_string())
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "application/sdp");
        assert_eq!(
            std::str::from_utf8(&multipart.parts[0].raw_content).unwrap(),
            sdp.to_string()
        );
    }
    
    #[test]
    fn test_image_part() {
        let image_data = Bytes::from_static(&[0xFF, 0xD8, 0xFF, 0xE0]); // Fake JPEG header
        let multipart = MultipartBodyBuilder::new()
            .add_image_part("image/jpeg", image_data.clone(), Some("img1@example.com"))
            .build();
            
        assert_eq!(multipart.parts.len(), 1);
        assert_eq!(multipart.parts[0].content_type().unwrap(), "image/jpeg");
        
        // Check Content-ID header
        let content_id_headers = multipart.parts[0].headers.iter()
            .filter(|h| h.name == HeaderName::Other("Content-ID".to_string()))
            .collect::<Vec<_>>();
            
        assert_eq!(content_id_headers.len(), 1);
        assert!(content_id_headers[0].to_string().contains("<img1@example.com>"));
            
        // Check image data
        assert_eq!(multipart.parts[0].raw_content, image_data);
    }
    
    #[test]
    fn test_to_string() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("simple-boundary")
            .add_text_part("Text content")
            .build();
            
        let body_string = multipart.to_string();
        
        // Check basic structure
        assert!(body_string.contains("--simple-boundary\r\n"));
        assert!(body_string.contains("Content-Type: text/plain\r\n"));
        assert!(body_string.contains("\r\nText content\r\n"));
        assert!(body_string.contains("--simple-boundary--\r\n"));
    }
    
    #[test]
    fn test_sip_message_integration() {
        let multipart = MultipartBodyBuilder::new()
            .boundary("test-boundary")
            .add_text_part("Plain text")
            .add_html_part("<html><body>HTML</body></html>")
            .build();
            
        let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
            .content_type(format!("multipart/alternative; boundary={}", multipart.boundary).as_str())
            .body(multipart.to_string())
            .build();
            
        let headers = message.all_headers();
        let content_type_headers = headers.iter()
            .filter(|h| match h {
                crate::types::TypedHeader::ContentType(_) => true,
                _ => false,
            })
            .collect::<Vec<_>>();
            
        assert_eq!(content_type_headers.len(), 1);
        assert!(content_type_headers[0].to_string().contains("multipart/alternative"));
        assert!(content_type_headers[0].to_string().contains("boundary=\"test-boundary\""));
        
        // Check body
        let body_str = message.body();
        assert!(std::str::from_utf8(body_str).unwrap().contains("--test-boundary"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Content-Type: text/plain"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Plain text"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("Content-Type: text/html"));
        assert!(std::str::from_utf8(body_str).unwrap().contains("<html><body>HTML</body></html>"));
    }
    
    #[test]
    fn test_multipart_part_builder() {
        // Basic part with content-type and body
        let part = MultipartPartBuilder::new()
            .content_type("text/plain")
            .body("This is text content")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "text/plain");
        assert_eq!(
            std::str::from_utf8(&part.raw_content).unwrap(),
            "This is text content"
        );
        
        // Part with content-id
        let part = MultipartPartBuilder::new()
            .content_type("text/plain")
            .content_id("<text123@example.com>")
            .body("This is text content with ID")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "text/plain");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::Other("Content-ID".to_string()) && 
            h.value.as_text() == Some("<text123@example.com>")
        ));
        
        // Part with content-disposition
        let part = MultipartPartBuilder::new()
            .content_type("application/sdp")
            .content_disposition("session")
            .body("v=0\r\no=- 1234 1234 IN IP4 127.0.0.1\r\ns=Test\r\n")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "application/sdp");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::ContentDisposition && 
            h.value.as_text() == Some("session")
        ));
        
        // Part with content-transfer-encoding
        let part = MultipartPartBuilder::new()
            .content_type("image/png")
            .content_transfer_encoding("base64")
            .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
            .build();
            
        assert_eq!(part.content_type().unwrap(), "image/png");
        assert!(part.headers.iter().any(|h| 
            h.name == HeaderName::Other("Content-Transfer-Encoding".to_string()) && 
            h.value.as_text() == Some("base64")
        ));
    }
    
    #[test]
    fn test_multipart_builder_mixed() {
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Plain text part")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/json")
                    .body(r#"{"key":"value"}"#)
                    .build()
            )
            .build();
            
        // Check content type
        let content_type = multipart.content_type();
        assert!(content_type.starts_with("multipart/mixed; boundary="));
        
        // Get boundary, handling both quoted and unquoted formats
        let boundary_part = content_type.split("boundary=").nth(1).unwrap_or("");
        let boundary = boundary_part.trim_matches('"');
        
        // Check body
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/plain"));
        assert!(body.contains("Plain text part"));
        assert!(body.contains("Content-Type: application/json"));
        assert!(body.contains(r#"{"key":"value"}"#));
        
        // Body should contain the boundary
        assert!(body.contains(&format!("--{}", boundary)));
        
        // Body should end with boundary--
        assert!(body.contains(&format!("--{}--", boundary)));
    }
    
    #[test]
    fn test_multipart_builder_alternative() {
        let multipart = MultipartBuilder::alternative()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("This is plain text")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/html")
                    .body("<html><body><p>This is HTML</p></body></html>")
                    .build()
            )
            .build();
            
        let content_type = multipart.content_type();
        assert!(content_type.starts_with("multipart/alternative; boundary="));
        
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/plain"));
        assert!(body.contains("This is plain text"));
        assert!(body.contains("Content-Type: text/html"));
        assert!(body.contains("<html><body><p>This is HTML</p></body></html>"));
    }
    
    #[test]
    fn test_multipart_builder_related() {
        let multipart = MultipartBuilder::related()
            .type_parameter("text/html")
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/html")
                    .content_id("<main@example.com>")
                    .body("<html><body><img src=\"cid:image@example.com\"></body></html>")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("image/png")
                    .content_id("<image@example.com>")
                    .content_transfer_encoding("base64")
                    .content_disposition("inline")
                    .body("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=")
                    .build()
            )
            .build();
            
        // Check content type - should contain both multipart/related and text/html
        let content_type = multipart.content_type();
        assert!(content_type.contains("multipart/related"));
        assert!(content_type.contains("text/html"));
        
        let body = multipart.body();
        assert!(body.contains("Content-Type: text/html"));
        assert!(body.contains("Content-ID: <main@example.com>"));
        assert!(body.contains("<img src=\"cid:image@example.com\">"));
        assert!(body.contains("Content-Type: image/png"));
        assert!(body.contains("Content-ID: <image@example.com>"));
        assert!(body.contains("Content-Transfer-Encoding: base64"));
    }
    
    #[test]
    fn test_multipart_builder_preamble_epilogue() {
        let preamble = "This is a multipart message in MIME format.";
        let epilogue = "This is the epilogue. It is also ignored.";
        
        let multipart = MultipartBuilder::mixed()
            .preamble(preamble)
            .epilogue(epilogue)
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Test content")
                    .build()
            )
            .build();
            
        let body = multipart.body();
        
        // Preamble should be before the first boundary
        let content_type_str = multipart.content_type();
        let boundary_part = content_type_str.split("boundary=").nth(1).unwrap_or("");
        let boundary = boundary_part.trim_matches('"');
        let first_boundary_pos = body.find(&format!("--{}", boundary)).unwrap_or(0);
        let preamble_in_body = &body[0..first_boundary_pos];
        assert_eq!(preamble_in_body.trim(), preamble);
        
        // Epilogue should be after the last boundary
        let last_boundary_pos = body.rfind(&format!("--{}--", boundary)).map(|pos| pos + boundary.len() + 4).unwrap_or(body.len()); // +4 for "--" and "--"
        let epilogue_in_body = &body[last_boundary_pos..].trim().to_string();
        assert_eq!(epilogue_in_body, epilogue);
    }
    
    #[test]
    fn test_multipart_builder_custom_boundary() {
        let custom_boundary = "a-custom-boundary-string";
        
        let multipart = MultipartBuilder::mixed()
            .boundary(custom_boundary.to_string())
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Test with custom boundary")
                    .build()
            )
            .build();
            
        // Check content type has our custom boundary
        let content_type = multipart.content_type();
        assert!(content_type.contains(&format!("boundary=\"{}\"", custom_boundary)));
        
        // Check body uses our custom boundary
        let body = multipart.body();
        assert!(body.contains(&format!("--{}", custom_boundary)));
        assert!(body.contains(&format!("--{}--", custom_boundary)));
    }
    
    #[test]
    fn test_integration_with_sip_message() {
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("text/plain")
                    .body("Hello SIP world!")
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/json")
                    .body(r#"{"greeting":"Hello JSON world!"}"#)
                    .build()
            )
            .build();
            
        let request = SimpleRequestBuilder::new(Method::Message, "sip:test@example.com").unwrap()
            .content_type(&multipart.content_type())
            .body(multipart.body())
            .build();
            
        // Check the request has proper Content-Type header
        let content_type_header = request.all_headers().iter()
            .find(|h| match h {
                TypedHeader::ContentType(_) => true,
                _ => false,
            })
            .unwrap();
        
        // Check the body is set correctly
        let header_str = content_type_header.to_string();
        assert!(header_str.contains("multipart/mixed") && header_str.contains("boundary="));
        
        // Check the body is set correctly
        assert_eq!(request.body(), multipart.body().as_bytes());
    }
    
    #[test]
    fn test_multipart_with_sdp() {
        let sdp = SdpBuilder::new("Test SDP")
            .origin("test", "123456", "789012", "IN", "IP4", "127.0.0.1")
            .connection("IN", "IP4", "127.0.0.1")
            .time("0", "0")
            .media_audio(49170, "RTP/AVP")
                .formats(&["0", "8"])
                .rtpmap("0", "PCMU/8000")
                .rtpmap("8", "PCMA/8000")
                .done()
            .build()
            .unwrap();
            
        let multipart = MultipartBuilder::mixed()
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/sdp")
                    .content_disposition("session")
                    .body(sdp.to_string())
                    .build()
            )
            .add_part(
                MultipartPartBuilder::new()
                    .content_type("application/xml")
                    .body("<metadata><session-id>12345</session-id></metadata>")
                    .build()
            )
            .build();
            
        // Check content type
        assert!(multipart.content_type().starts_with("multipart/mixed; boundary="));
        
        // Check body contains SDP
        let body = multipart.body();
        assert!(body.contains("v=0"));
        assert!(body.contains("m=audio 49170 RTP/AVP 0 8"));
        assert!(body.contains("a=rtpmap:0 PCMU/8000"));
        
        // Check body contains XML
        assert!(body.contains("<metadata><session-id>12345</session-id></metadata>"));
    }
} 