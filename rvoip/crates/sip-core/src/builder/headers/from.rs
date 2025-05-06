use std::str::FromStr;

use crate::types::{
    uri::Uri,
    from::From,
    Address,
    TypedHeader,
    Param,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding From headers to SIP message builders.
///
/// This trait provides a standard way to add From headers to both request and response builders
/// as specified in [RFC 3261 Section 20.20](https://datatracker.ietf.org/doc/html/rfc3261#section-20.20).
/// The From header field indicates the logical identity of the initiator of the request.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::FromBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("tag1234"))
///     .build();
/// ```
pub trait FromBuilderExt {
    /// Add a From header with an optional tag parameter.
    ///
    /// Creates and adds a From header with the specified display name, URI, and optional tag.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header (e.g., "Alice")
    /// - `uri`: The URI for the From header (e.g., "sip:alice@example.com")
    /// - `tag`: Optional tag parameter for dialog identification
    ///
    /// # Returns
    /// Self for method chaining
    fn from(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self;
}

impl FromBuilderExt for SimpleRequestBuilder {
    fn from(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::From(From::new(address)))
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::From(From::new(address)))
            }
        }
    }
}

impl FromBuilderExt for SimpleResponseBuilder {
    fn from(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::From(From::new(address)))
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::From(From::new(address)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    
    #[test]
    fn test_request_from_header() {
        // Test with valid URI
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .build();
            
        let from_header = request.from().unwrap();
        assert_eq!(from_header.address().display_name(), Some("Alice"));
        assert_eq!(from_header.address().uri().to_string(), "sip:alice@example.com");
        assert_eq!(from_header.tag(), Some("tag1234"));
        
        // Test without tag
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", None)
            .build();
            
        let from_header = request.from().unwrap();
        assert_eq!(from_header.tag(), None);
        
        // Test with invalid URI (should create a custom URI)
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "invalid-uri", Some("tag1234"))
            .build();
            
        let from_header = request.from().unwrap();
        assert_eq!(from_header.address().display_name(), Some("Alice"));
        assert!(from_header.address().uri().to_string().contains("invalid-uri"));
    }
    
    #[test]
    fn test_response_from_header() {
        // Test with valid URI
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let from_header = response.from().unwrap();
        assert_eq!(from_header.address().display_name(), Some("Alice"));
        assert_eq!(from_header.address().uri().to_string(), "sip:alice@example.com");
        assert_eq!(from_header.tag(), Some("tag1234"));
        
        // Test without tag
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", None)
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let from_header = response.from().unwrap();
        assert_eq!(from_header.tag(), None);
        
        // Test with invalid URI (should create a custom URI)
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "invalid-uri", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let from_header = response.from().unwrap();
        assert_eq!(from_header.address().display_name(), Some("Alice"));
        assert!(from_header.address().uri().to_string().contains("invalid-uri"));
    }
} 