use crate::error::{Error, Result};
use crate::types::{
    TypedHeader,
    header::TypedHeaderTrait,
    headers::header_access::HeaderAccess,
};
use crate::types::content_encoding::ContentEncoding;
use super::HeaderSetter;

/// Content-Encoding Header Builder for SIP Messages
///
/// This module provides builder methods for the Content-Encoding header in SIP messages,
/// which indicates the encodings that have been applied to the message body.
///
/// ## SIP Content-Encoding Header Overview
///
/// The Content-Encoding header is defined in [RFC 3261 Section 20.12](https://datatracker.ietf.org/doc/html/rfc3261#section-20.12)
/// as part of the core SIP protocol. It follows the syntax and semantics defined in 
/// [RFC 2616 Section 14.11](https://datatracker.ietf.org/doc/html/rfc2616#section-14.11) for HTTP.
/// The header indicates what encoding transformations have been applied to the message body.
///
/// ## Purpose of Content-Encoding Header
///
/// The Content-Encoding header serves several important purposes:
///
/// 1. It allows compression of message bodies to reduce bandwidth and transmission time
/// 2. It enables efficient transfer of large message bodies (e.g., file transfers via SIP)
/// 3. It provides a way to encode binary content in ways safe for transmission
/// 4. It supports interoperability with HTTP and web technologies
///
/// ## Common Content Encodings in SIP
///
/// - **gzip**: Standard GZIP compression
/// - **deflate**: ZLIB compression
/// - **compress**: UNIX "compress" program method
/// - **identity**: No transformation (default when header is absent)
///
/// ## Special Considerations
///
/// 1. **Multiple Encodings**: When multiple encodings are specified, they are applied in the order listed
/// 2. **Compact Form**: Content-Encoding has the compact form 'e', though this is less commonly used
/// 3. **Recipient Behavior**: Receiving UAs must understand all listed encodings to decode the body properly
/// 4. **Performance Trade-offs**: Compression adds processing overhead which may not be beneficial for small bodies
///
/// ## Relationship with other headers
///
/// - **Content-Encoding** + **Content-Type**: Content-Type identifies the format after decoding
/// - **Content-Encoding** + **Content-Length**: Content-Length specifies the length of the encoded body
/// - **Content-Encoding** + **Accept-Encoding**: Accept-Encoding indicates what encodings a recipient can handle
///
/// # Examples
///
/// ## Basic Usage with Compression
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentEncodingExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE with compressed text content
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
///     .content_type_text()
///     .content_encoding("gzip")  // Specifies that the body is GZIP compressed
///     // In a real app, you would compress the body before setting it
///     .body("This represents compressed content")
///     .build();
/// ```
///
/// ## Multiple Encoding Transformations
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentEncodingExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE with multiple encodings applied in sequence
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
///     .content_type_text()
///     .content_encodings(&["gzip", "base64"])  // First compressed, then base64 encoded
///     // In a real application, you would need to apply these transformations in order
///     .body("VGhpcyByZXByZXNlbnRzIGNvbXByZXNzZWQgYW5kIGJhc2U2NCBlbmNvZGVkIGNvbnRlbnQ=")
///     .build();
/// ```
///
/// ## File Transfer Scenario
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentEncodingExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Function to represent compressing a file (in a real app)
/// fn compress_file_content(content: &[u8]) -> Vec<u8> {
///     // In a real application, you would use a compression library here
///     // For example: let compressed = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
///     content.to_vec() // Placeholder for actual compression
/// }
///
/// // Simulate a file transfer with compression
/// let file_content = b"Imagine this is a large file content...";
/// let compressed_content = compress_file_content(file_content);
///
/// // Create a MESSAGE request for file transfer
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:recipient@example.com").unwrap()
///     .content_type("application/octet-stream")
///     .content_encoding("gzip")
///     // In a real app, use the compressed body bytes
///     .body("Compressed file content would go here")
///     .build();
/// ```

/// Extension trait for adding Content-Encoding header building capabilities
///
/// This trait provides methods to add Content-Encoding headers to SIP messages, indicating
/// what encodings have been applied to the message body.
///
/// ## When to use Content-Encoding
///
/// Content-Encoding is particularly useful in the following scenarios:
///
/// 1. **Large message bodies**: To reduce bandwidth and improve transmission speed
/// 2. **Mobile clients**: Where bandwidth may be limited or costly
/// 3. **Binary content**: When sending non-text content that benefits from compression
/// 4. **Interoperability**: When working with web services that expect compressed content
///
/// ## Best Practices
///
/// - Only use compression for bodies large enough to benefit (typically >1KB)
/// - Ensure the recipient supports the encodings you specify
/// - For SIP-to-web gateways, align with common HTTP encodings (gzip, deflate)
/// - Consider the computational cost of compression/decompression for real-time messages
///
/// # Examples
///
/// ## MESSAGE with Compressed Content
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentEncodingExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a SIP MESSAGE with compressed text
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
///     .from("Sender", "sip:sender@example.com", Some("tag1234"))
///     .to("Recipient", "sip:user@example.com", None)
///     .content_type_text()
///     .content_encoding("gzip")
///     // In a real application, you would compress the body before setting it
///     .body("This represents compressed content")
///     .build();
/// ```
///
/// ## REGISTER with Compressed Capabilities Document
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentEncodingExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a REGISTER with compressed capabilities XML
/// let capabilities_xml = r#"<?xml version="1.0"?>
/// <capabilities xmlns="urn:ietf:params:xml:ns:pidf:caps">
///   <audio>true</audio>
///   <video>true</video>
///   <text>true</text>
///   <application>true</application>
///   <control>false</control>
///   <automata>false</automata>
///   <class>personal</class>
/// </capabilities>"#;
///
/// // In real code, you would compress this XML
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
///     .from("User", "sip:user@example.com", Some("reg123"))
///     .to("User", "sip:user@example.com", None)
///     .content_type_xml()
///     .content_encoding("gzip")
///     .body(capabilities_xml)  // Pretend this is compressed
///     .build();
/// ```
pub trait ContentEncodingExt {
    /// Add a Content-Encoding header with a single encoding
    ///
    /// This method specifies a single encoding that has been applied to the message body.
    /// Common encodings include "gzip", "deflate", "compress", and "identity".
    ///
    /// # Arguments
    ///
    /// * `encoding` - The content encoding to specify
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentEncodingExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a MESSAGE with gzip-compressed content
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
    ///     .from("Sender", "sip:sender@example.com", Some("tag1234"))
    ///     .to("Recipient", "sip:user@example.com", None)
    ///     .content_type_text()
    ///     .content_encoding("gzip")  // Body is gzip compressed
    ///     // In a real application, you would compress the body before setting it
    ///     .body("This represents compressed content")
    ///     .build();
    /// ```
    ///
    /// # RFC Reference
    /// 
    /// As per [RFC 3261 Section 20.12](https://datatracker.ietf.org/doc/html/rfc3261#section-20.12),
    /// the Content-Encoding header field is used to indicate any additional content codings
    /// that have been applied to the message body.
    fn content_encoding(self, encoding: &str) -> Self;

    /// Add a Content-Encoding header with multiple encodings
    ///
    /// This method specifies multiple encodings that have been applied to the message body.
    /// The encodings are listed in the order they were applied (and should be reversed when decoding).
    ///
    /// # Arguments
    ///
    /// * `encodings` - A slice of content encodings to specify, in application order
    ///
    /// # Returns
    ///
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::{SimpleRequestBuilder, headers::ContentEncodingExt};
    /// use rvoip_sip_core::builder::headers::ContentTypeBuilderExt;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a MESSAGE with multiple encodings
    /// // First the content was compressed with gzip, then base64 encoded
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:user@example.com").unwrap()
    ///     .from("Sender", "sip:sender@example.com", Some("tag1234"))
    ///     .to("Recipient", "sip:user@example.com", None)
    ///     .content_type_text()
    ///     .content_encodings(&["gzip", "base64"])  // Applied in this order
    ///     // In a real application, you would apply these transformations
    ///     .body("VGhpcyByZXByZXNlbnRzIGNvbXByZXNzZWQgYW5kIGJhc2U2NCBlbmNvZGVkIGNvbnRlbnQ=")
    ///     .build();
    /// ```
    ///
    /// # Multiple Encodings Explained
    ///
    /// When multiple encodings are specified, they are applied in the order listed. 
    /// For example, if the header is `Content-Encoding: gzip, base64`, this means:
    /// 1. The original content was first compressed with gzip
    /// 2. Then the compressed result was encoded with base64
    ///
    /// The recipient must apply the inverse transformations in reverse order:
    /// 1. First decode from base64
    /// 2. Then decompress the gzip data
    fn content_encodings<T: AsRef<str>>(self, encodings: &[T]) -> Self;
}

impl<T> ContentEncodingExt for T 
where 
    T: HeaderSetter,
{
    fn content_encoding(self, encoding: &str) -> Self {
        let header_value = ContentEncoding::single(encoding);
        self.set_header(header_value)
    }

    fn content_encodings<S: AsRef<str>>(self, encodings: &[S]) -> Self {
        let header_value = ContentEncoding::with_encodings(encodings);
        self.set_header(header_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::SimpleRequestBuilder;
    use crate::types::header::HeaderName;
    use crate::types::ContentEncoding; // Import the actual type
    
    #[test]
    fn test_content_encoding_single() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_encoding("gzip")
            .build();
            
        // Check if Content-Encoding header exists with the correct value
        let header = request.header(&HeaderName::ContentEncoding);
        assert!(header.is_some(), "Content-Encoding header not found");
        
        if let Some(TypedHeader::ContentEncoding(content_encoding)) = header {
            // Check if the content encoding includes "gzip"
            assert!(content_encoding.has_encoding("gzip"), "gzip encoding not found");
            assert_eq!(content_encoding.encodings().len(), 1);
        } else {
            panic!("Expected Content-Encoding header");
        }
    }
    
    #[test]
    fn test_content_encodings_multiple() {
        let request = SimpleRequestBuilder::register("sip:example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .to("Alice", "sip:alice@example.com", None)
            .content_encodings(&["gzip", "deflate"])
            .build();
            
        // Check if Content-Encoding header exists with the correct values
        let header = request.header(&HeaderName::ContentEncoding);
        assert!(header.is_some(), "Content-Encoding header not found");
        
        if let Some(TypedHeader::ContentEncoding(content_encoding)) = header {
            // Check if the content encoding includes both "gzip" and "deflate"
            assert!(content_encoding.has_encoding("gzip"), "gzip encoding not found");
            assert!(content_encoding.has_encoding("deflate"), "deflate encoding not found");
            assert_eq!(content_encoding.encodings().len(), 2);
        } else {
            panic!("Expected Content-Encoding header");
        }
    }
} 