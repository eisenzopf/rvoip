use crate::types::{
    headers::HeaderName,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding MIME-Version headers to SIP message builders.
///
/// This trait provides a standard way to add MIME-Version headers to both request and response builders
/// as specified in [RFC 3261 Section 20.24](https://datatracker.ietf.org/doc/html/rfc3261#section-20.24).
/// The MIME-Version header is typically included in messages that contain MIME content, especially those
/// with multipart message bodies.
///
/// # Examples
///
/// ## Basic MIME-Version Header
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
/// ## With Multipart Content
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
/// ## SIP INVITE with SDP and XML (Advanced Example)
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
        self.header(TypedHeader::MimeVersion((1, 0)))
    }
    
    fn mime_version(self, major: u32, minor: u32) -> Self {
        self.header(TypedHeader::MimeVersion((major, minor)))
    }
}

impl MimeVersionBuilderExt for SimpleResponseBuilder {
    fn mime_version_1_0(self) -> Self {
        self.header(TypedHeader::MimeVersion((1, 0)))
    }
    
    fn mime_version(self, major: u32, minor: u32) -> Self {
        self.header(TypedHeader::MimeVersion((major, minor)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    
    #[test]
    fn test_request_mime_version_1_0() {
        let request = SimpleRequestBuilder::new(Method::Message, "sip:example.com").unwrap()
            .mime_version_1_0()
            .build();
            
        let mime_version_headers = request.headers(&HeaderName::MimeVersion);
            
        assert_eq!(mime_version_headers.len(), 1);
        if let TypedHeader::MimeVersion((major, minor)) = mime_version_headers[0] {
            assert_eq!(*major, 1);
            assert_eq!(*minor, 0);
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
        if let TypedHeader::MimeVersion((major, minor)) = mime_version_headers[0] {
            assert_eq!(*major, 2);
            assert_eq!(*minor, 1);
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
        if let TypedHeader::MimeVersion((major, minor)) = mime_version_headers[0] {
            assert_eq!(*major, 1);
            assert_eq!(*minor, 0);
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
        if let TypedHeader::MimeVersion((major, minor)) = mime_version_headers[0] {
            assert_eq!(*major, 2);
            assert_eq!(*minor, 1);
        } else {
            panic!("Expected MimeVersion header");
        }
    }
} 