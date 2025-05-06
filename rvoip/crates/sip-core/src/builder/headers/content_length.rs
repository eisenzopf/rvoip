//! Content-Length header builder
//!
//! This module provides builder methods for the Content-Length header,
//! which indicates the size of the message body in bytes.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::builder::headers::ContentLengthBuilderExt;
//! use rvoip_sip_core::types::Method;
//!
//! // Create a request with a specific Content-Length
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .content_length(1024)
//!     .build();
//! ```
//!
//! Note: In most cases, you don't need to explicitly set the Content-Length header,
//! as it's automatically set when you add a body to the message using the `body()` method.

use crate::types::{
    content_length::ContentLength,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding Content-Length headers to SIP message builders.
///
/// This trait provides a standard way to add Content-Length headers to both request and response builders
/// as specified in [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14).
/// The Content-Length header indicates the size of the message body in bytes.
///
/// # Examples
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
/// Note: In most cases, you don't need to explicitly set the Content-Length header,
/// as it's automatically set when you add a body to the message using the `body()` method.
pub trait ContentLengthBuilderExt {
    /// Add a Content-Length header
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
    /// # Note
    /// In most cases, you don't need to explicitly set the Content-Length header,
    /// as it's automatically set when you add a body to the message using the `body()` method.
    fn content_length(self, length: u32) -> Self;
    
    /// Add a Content-Length header with value 0
    ///
    /// Creates and adds a Content-Length header with value 0, indicating that the message has no body.
    ///
    /// # Returns
    /// Self for method chaining
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