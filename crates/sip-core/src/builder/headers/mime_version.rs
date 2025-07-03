use crate::types::{
    headers::HeaderName,
    TypedHeader,
    mime_version::MimeVersion as MimeVersionType,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;
use bytes::Bytes;

/// # MIME-Version Header Builder Extension
///
/// This module provides builder methods for the MIME-Version header in SIP messages.
///
/// ## SIP MIME-Version Header Overview
///
/// The MIME-Version header is defined in [RFC 3261 Section 20.24](https://datatracker.ietf.org/doc/html/rfc3261#section-20.24)
/// as part of the core SIP protocol. It indicates which version of the MIME protocol is being used,
/// and is typically required when using MIME-formatted message bodies, particularly multipart bodies.
///
/// ## Format
///
/// ```text
/// MIME-Version: 1.0
/// ```
///
/// ## Purpose of MIME-Version Header
///
/// The MIME-Version header serves several specific purposes in SIP:
///
/// 1. It indicates compliance with the MIME specification for message body handling
/// 2. It is required when using MIME multipart bodies to properly delimit different content parts
/// 3. It enables SIP clients to correctly interpret complex body structures
/// 4. It allows for proper handling of media attachments and alternative content formats
///
/// ## When to Use MIME-Version
///
/// - **Multipart Bodies**: Always include when using multipart/mixed, multipart/alternative, etc.
/// - **Rich Content**: When including non-SDP bodies like XML or JSON
/// - **Multiple Body Types**: When a message contains multiple body parts with different content types
/// - **MIME Extensions**: When using any MIME-specific features like Content-ID references
///
/// ## Relationship with other headers
///
/// - **MIME-Version vs Content-Type**: MIME-Version indicates the MIME protocol version, while Content-Type 
///   specifies the media type of the message body. Both are typically required for proper MIME handling.
/// - **MIME-Version vs Content-Disposition**: Content-Disposition provides additional information about
///   how to present the body, while MIME-Version indicates the overall MIME compliance.
/// - **MIME-Version with multipart/mixed**: When using multipart bodies, the MIME-Version header is 
///   required along with Content-Type boundaries to correctly parse the body parts.
///
/// ## Common Values
///
/// In practice, `1.0` is almost always used as the MIME-Version value. Other versions are rarely
/// seen in SIP deployments. The header syntax supports major.minor version numbers.
///
/// ## Examples
///
/// ### Basic MIME-Version Header
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::MimeVersionBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with MIME-Version 1.0 (most common)
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .mime_version_1_0()
///     .body("Hello, world!")
///     .build();
///
/// // Create a request with a custom MIME-Version (less common)
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
///     .mime_version(2, 0)
///     .body("Hello from MIME 2.0!")
///     .build();
/// ```
///
/// ### With Multipart Content
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{MimeVersionBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::types::Method;
///
/// // Create a multipart body using the builder
/// let multipart = MultipartBodyBuilder::new()
///     .add_text_part("This is the text part")
///     .add_html_part("<html><body><p>This is the HTML part</p></body></html>")
///     .build();
/// 
/// // Create a SIP message with the multipart body and MIME-Version header
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .mime_version_1_0()  // Add MIME-Version: 1.0 header
///     .content_type(format!("multipart/mixed; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// ### SIP MESSAGE with Text and XML Alternative Formats
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{MimeVersionBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::types::Method;
///
/// // Scenario: Sending a MESSAGE with both plain text and XML formats
///
/// // Create a multipart body with text and XML parts
/// let multipart = MultipartBodyBuilder::new()
///     .add_text_part("Meeting scheduled for 3pm")
///     .add_xml_part(r#"<?xml version="1.0"?>
///       <meeting xmlns="urn:example:meeting">
///         <subject>Project Review</subject>
///         <time>15:00</time>
///         <location>Conference Room B</location>
///       </meeting>"#)
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("msg-1"))
///     .to("Bob", "sip:bob@example.com", None)
///     .mime_version_1_0()  // Required for multipart bodies
///     .content_type(format!("multipart/alternative; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
///     
/// // The recipient can choose to display either the plain text or XML version
/// ```
///
/// ### SIP INVITE with SDP and XML (Advanced Example)
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{MimeVersionBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::types::Method;
/// use rvoip_sip_core::sdp::SdpBuilder;
///
/// // Create an SDP description
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
/// // XML metadata
/// let xml_metadata = r#"<?xml version="1.0"?>
/// <metadata xmlns="urn:example:metadata">
///   <call-id>a84b4c76e66710</call-id>
///   <priority>normal</priority>
///   <security>unclassified</security>
/// </metadata>"#;
///
/// // Create a multipart body with SDP and XML
/// let multipart = MultipartBodyBuilder::new()
///     .add_sdp_part(sdp.to_string())
///     .add_xml_part(xml_metadata)
///     .build();
///
/// // Create a SIP INVITE with the multipart body and MIME-Version header
/// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .mime_version_1_0()
///     .content_type(format!("multipart/mixed; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
///
/// ### Notification with Image Attachment
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{MimeVersionBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::builder::multipart::MultipartBodyBuilder;
/// use rvoip_sip_core::types::Method;
/// use bytes::Bytes;
///
/// // Scenario: Sending a notification with text and an image attachment
///
/// // Sample image data (normally this would be actual binary data)
/// let image_data = Bytes::from_static(b"THIS_WOULD_BE_BINARY_IMAGE_DATA");
///
/// // Create a multipart body with text and image
/// let multipart = MultipartBodyBuilder::new()
///     .add_text_part("Please see the attached image for the office layout")
///     .add_image_part("image/png", image_data, Some("image1@example.com"))
///     .build();
///
/// // Create a SIP MESSAGE with the multipart body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:team@example.com").unwrap()
///     .from("Admin", "sip:admin@example.com", Some("not-1"))
///     .to("Team", "sip:team@example.com", None)
///     .mime_version_1_0()  // Required for multipart/mixed bodies
///     .content_type(format!("multipart/mixed; boundary={}", multipart.boundary).as_str())
///     .body(multipart.to_string())
///     .build();
/// ```
pub trait MimeVersionBuilderExt {
    /// Add a MIME-Version header with version 1.0
    ///
    /// This is a convenience method for setting the most common MIME version (1.0).
    /// The MIME-Version header is used in SIP messages containing MIME content,
    /// especially multipart bodies.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::MimeVersionBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .mime_version_1_0()
    ///     .body("Hello, world!")
    ///     .build();
    /// ```
    fn mime_version_1_0(self) -> Self;
    
    /// Add a MIME-Version header with custom version numbers
    ///
    /// This method allows you to set a MIME-Version header with any major and minor
    /// version numbers, though 1.0 is by far the most common in practice.
    ///
    /// # Parameters
    /// - `major`: The major version number
    /// - `minor`: The minor version number
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::MimeVersionBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
    ///     .mime_version(1, 0)  // Equivalent to mime_version_1_0()
    ///     .body("Hello, world!")
    ///     .build();
    /// ```
    fn mime_version(self, major: u32, minor: u32) -> Self;
}

impl MimeVersionBuilderExt for SimpleRequestBuilder {
    fn mime_version_1_0(self) -> Self {
        self.header(TypedHeader::MimeVersion(MimeVersionType::new(1, 0)))
    }
    
    fn mime_version(self, major: u32, minor: u32) -> Self {
        self.header(TypedHeader::MimeVersion(MimeVersionType::new(major, minor)))
    }
}

impl MimeVersionBuilderExt for SimpleResponseBuilder {
    fn mime_version_1_0(self) -> Self {
        self.header(TypedHeader::MimeVersion(MimeVersionType::new(1, 0)))
    }
    
    fn mime_version(self, major: u32, minor: u32) -> Self {
        self.header(TypedHeader::MimeVersion(MimeVersionType::new(major, minor)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode, mime_version::MimeVersion as MimeVersionType};
    use crate::types::headers::HeaderAccess;
    
    #[test]
    fn test_request_mime_version_1_0() {
        let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
            .mime_version_1_0()
            .build();
            
        let mime_version_headers = request.headers(&HeaderName::MimeVersion);
            
        assert_eq!(mime_version_headers.len(), 1);
        if let TypedHeader::MimeVersion(version_struct) = mime_version_headers[0] {
            assert_eq!(version_struct.major(), 1);
            assert_eq!(version_struct.minor(), 0);
        } else {
            panic!("Expected MimeVersion header");
        }
    }
    
    #[test]
    fn test_request_custom_mime_version() {
        let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
            .mime_version(2, 1)
            .build();
            
        let mime_version_headers = request.headers(&HeaderName::MimeVersion);
            
        assert_eq!(mime_version_headers.len(), 1);
        if let TypedHeader::MimeVersion(version_struct) = mime_version_headers[0] {
            assert_eq!(version_struct.major(), 2);
            assert_eq!(version_struct.minor(), 1);
        } else {
            panic!("Expected MimeVersion header");
        }
    }
    
    #[test]
    fn test_response_mime_version_1_0() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .mime_version_1_0()
            .build();
            
        let mime_version_headers = response.headers(&HeaderName::MimeVersion);
            
        assert_eq!(mime_version_headers.len(), 1);
        if let TypedHeader::MimeVersion(version_struct) = mime_version_headers[0] {
            assert_eq!(version_struct.major(), 1);
            assert_eq!(version_struct.minor(), 0);
        } else {
            panic!("Expected MimeVersion header");
        }
    }
    
    #[test]
    fn test_response_custom_mime_version() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
            .mime_version(2, 1)
            .build();
            
        let mime_version_headers = response.headers(&HeaderName::MimeVersion);
            
        assert_eq!(mime_version_headers.len(), 1);
        if let TypedHeader::MimeVersion(version_struct) = mime_version_headers[0] {
            assert_eq!(version_struct.major(), 2);
            assert_eq!(version_struct.minor(), 1);
        } else {
            panic!("Expected MimeVersion header");
        }
    }
} 