

use crate::types::{
    content_length::ContentLength,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Content-Length Header Builder for SIP Messages
///
/// This module provides builder methods for the Content-Length header in SIP messages,
/// which indicates the size of the message body in bytes.
///
/// ## SIP Content-Length Header Overview
///
/// The Content-Length header is defined in [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14)
/// as part of the core SIP protocol. It specifies the size of the message body in octets (bytes),
/// allowing recipients to properly handle message framing, especially in stream-based transports like TCP.
///
/// ## Purpose of Content-Length Header
///
/// The Content-Length header serves several critical purposes in SIP:
///
/// 1. It enables correct message framing when SIP messages are carried over stream-based transports
/// 2. It allows recipients to verify they have received the complete message body
/// 3. It helps distinguish between empty bodies and missing bodies
/// 4. It helps prevent truncation or misinterpretation of message bodies
///
/// ## Special Considerations
///
/// 1. **Automatic Content-Length**: In most cases, the `body()` method on SIP message builders 
///    automatically sets the Content-Length header based on the actual body size
/// 2. **Zero Content-Length**: A Content-Length of 0 explicitly indicates an empty body
/// 3. **Transport Differences**: Content-Length is mandatory for TCP, TLS, and WebSocket transports, 
///    but optional for UDP (where message boundaries are handled by the datagram)
/// 4. **Compact Form**: Content-Length has the compact form 'l', though this is less commonly used
///
/// ## Relationship with other headers
///
/// - **Content-Length** + **Content-Type**: Content-Length indicates body size while Content-Type specifies format
/// - **Content-Length** vs **Transfer-Encoding**: These are mutually exclusive; if Transfer-Encoding is used 
///   (e.g., "chunked"), Content-Length should not be present
/// - **Zero Content-Length**: Often used with response codes that don't permit bodies (e.g., 100, 304)
/// - **Missing Content-Length**: In UDP, a missing Content-Length means the body extends to the end of the UDP datagram
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with a specific Content-Length
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .content_length(1024)
///     .build();
/// ```
///
/// ## Automatic Content-Length with body
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
///
/// // Content-Length is set automatically when you add a body
/// let request = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .body("Hello, this is a text message")
///     .build();
///
/// // The Content-Length header will be set to the length of the body in bytes (27)
/// ```
///
/// ## Zero Content-Length for SIP ACK
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an ACK request with explicit zero Content-Length
/// let ack = SimpleRequestBuilder::new(Method::Ack, "sip:bob@example.com").unwrap()
///     .no_content()  // Sets Content-Length: 0
///     .build();
/// ```
///
/// ## INVITE with SDP Body
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, ContentLengthBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create an INVITE with SDP body
/// let sdp_body = "v=0\r\n\
///                 o=user1 53655765 2353687637 IN IP4 192.168.0.1\r\n\
///                 s=My SDP Session\r\n\
///                 c=IN IP4 192.168.0.1\r\n\
///                 t=0 0\r\n\
///                 m=audio 49170 RTP/AVP 0\r\n\
///                 a=rtpmap:0 PCMU/8000\r\n";
///                 
/// let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .content_type_sdp()
///     .body(sdp_body)  // Content-Length set automatically based on body size
///     .build();
///
/// // The Content-Length header will be set to the actual byte length of the SDP body
/// ```
///
/// ## 100 Trying Response with Empty Body
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
/// use rvoip_sip_core::types::{StatusCode, Method};
///
/// // Create a 100 Trying response with explicit zero Content-Length
/// let trying = SimpleResponseBuilder::new(StatusCode::Trying, None)
///     .no_content()  // Sets Content-Length: 0
///     .build();
/// ```
///
/// ## Handling Large Message Bodies
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentTypeBuilderExt, ContentLengthBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Function that generates a large message body
/// fn generate_large_body() -> String {
///     let mut large_body = String::with_capacity(8192);
///     // In a real application, you would fill this with actual content
///     for i in 0..8192 {
///         large_body.push_str("A");
///     }
///     large_body
/// }
///
/// // Create a MESSAGE request with a large text body
/// let large_body = generate_large_body();
/// let body_length = large_body.len() as u32;  // Calculate exact byte length
///
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .content_type_text()
///     // Explicitly set Content-Length for a large body
///     // Note: builder.body() would automatically set this, but for very large bodies
///     // you might want to set it explicitly before generating the body
///     .content_length(body_length)
///     .body(large_body)
///     .build();
/// ```
///
/// Note: In most cases, you don't need to explicitly set the Content-Length header,
/// as it's automatically set when you add a body to the message using the `body()` method.
///
/// ## Setting Content-Length for a Message Request
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLengthBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::Method;
///
/// // Create a MESSAGE request with a text body
/// let message = SimpleRequestBuilder::new(Method::Message, "sip:bob@example.com").unwrap()
///     .content_type_text()
///     .body("Hello, this is a text message")  // Content-Length set automatically
///     .build();
/// ```
///
/// ## Setting Content-Length to Zero for an ACK
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create an ACK request with zero Content-Length
/// let ack = SimpleRequestBuilder::new(Method::Ack, "sip:bob@biloxi.com").unwrap()
///     .no_content()  // Explicitly sets Content-Length: 0
///     .build();
/// ```
///
/// ## Response with Content-Length
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::builder::headers::{ContentLengthBuilderExt, ContentTypeBuilderExt};
/// use rvoip_sip_core::types::{StatusCode, Method};
///
/// // Create a 200 OK response to an OPTIONS request (no body)
/// let options_ok = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .no_content()  // Sets Content-Length: 0
///     .build();
///
/// // Create a 200 OK response to a MESSAGE with a body
/// let message_response = SimpleResponseBuilder::new(StatusCode::Ok, None)
///     .content_type_text()
///     .body("Message received, thank you!")  // Content-Length set automatically
///     .build();
/// ```
pub trait ContentLengthBuilderExt {
    /// Add a Content-Length header with a specific byte length value
    ///
    /// Creates and adds a Content-Length header as specified in [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14).
    /// The Content-Length header indicates the size of the message body in bytes.
    ///
    /// # Parameters
    /// - `length`: The size of the message body in bytes
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::{ContentLengthBuilderExt, ContentTypeBuilderExt};
    /// use rvoip_sip_core::types::Method;
    ///
    /// // REGISTER request with client capabilities (XML body)
    /// let xml_body = "<?xml version=\"1.0\"?>\r\n\
    ///                  <capabilities>\r\n\
    ///                    <audio>true</audio>\r\n\
    ///                    <video>false</video>\r\n\
    ///                    <text>true</text>\r\n\
    ///                  </capabilities>\r\n";
    ///
    /// let register = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
    ///     .content_type_xml()
    ///     .content_length(xml_body.len() as u32)  // Explicit Content-Length
    ///     .body(xml_body)  // Body matches the declared length
    ///     .build();
    /// ```
    ///
    /// # Note
    /// In most cases, you don't need to explicitly set the Content-Length header,
    /// as it's automatically set when you add a body to the message using the `body()` method.
    /// If you set both Content-Length and body, the body() method will override your Content-Length
    /// with the actual length of the body content.
    fn content_length(self, length: u32) -> Self;
    
    /// Add a Content-Length header with value 0
    ///
    /// Creates and adds a Content-Length header with value 0, indicating that the message has no body.
    /// This is particularly useful for methods like ACK, CANCEL, and some responses like 100 Trying
    /// which typically don't have bodies.
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create an OPTIONS request with no body
    /// let options = SimpleRequestBuilder::new(Method::Options, "sip:example.com").unwrap()
    ///     .no_content()  // Explicitly signals no body with Content-Length: 0
    ///     .build();
    ///
    /// // Create a CANCEL request for an existing INVITE
    /// let cancel = SimpleRequestBuilder::new(Method::Cancel, "sip:bob@example.com").unwrap()
    ///     .no_content()  // CANCEL requests should have no body
    ///     .build();
    /// ```
    ///
    /// # Note
    /// While some transports can infer an empty body from the absence of body content,
    /// explicitly setting Content-Length: 0 is generally considered good practice for
    /// clarity and interoperability, especially for messages that typically don't have bodies.
    fn no_content(self) -> Self;
}

impl ContentLengthBuilderExt for SimpleRequestBuilder {
    fn content_length(self, length: u32) -> Self {
        self.header(TypedHeader::ContentLength(ContentLength::new(length)))
    }
    
    fn no_content(self) -> Self {
        self.content_length(0)
    }
}

impl ContentLengthBuilderExt for SimpleResponseBuilder {
    fn content_length(self, length: u32) -> Self {
        self.header(TypedHeader::ContentLength(ContentLength::new(length)))
    }
    
    fn no_content(self) -> Self {
        self.content_length(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;
    use crate::builder::headers::from::FromBuilderExt;
    use crate::builder::headers::to::ToBuilderExt;

    #[test]
    fn test_request_content_length() {
        let length = 1024;
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_length(length)
            .build();
            
        let content_length_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentLength(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_length_headers.len(), 1);
        assert_eq!(content_length_headers[0].0, length);
    }
    
    #[test]
    fn test_response_content_length() {
        let length = 2048;
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .cseq_with_method(101, Method::Invite)
            .content_length(length)
            .build();
            
        let content_length_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentLength(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_length_headers.len(), 1);
        assert_eq!(content_length_headers[0].0, length);
    }
    
    #[test]
    fn test_request_no_content() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .no_content()
            .build();
            
        let content_length_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentLength(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_length_headers.len(), 1);
        assert_eq!(content_length_headers[0].0, 0);
    }
    
    #[test]
    fn test_content_length_with_body() {
        let body = "Hello, world!";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .body(body)
            .build();
            
        let content_length_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentLength(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_length_headers.len(), 1);
        assert_eq!(content_length_headers[0].0, body.len() as u32);
    }
    
    #[test]
    fn test_override_content_length_with_body() {
        let body = "Hello, world!";
        let wrong_length = 1000; // This is wrong, but we'll override it
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_length(wrong_length) // This will be overridden by the body method
            .body(body)
            .build();
            
        let content_length_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::ContentLength(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(content_length_headers.len(), 1);
        assert_eq!(content_length_headers[0].0, body.len() as u32); // It should use the actual length
    }
} 