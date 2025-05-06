use std::str::FromStr;

use crate::types::{
    uri::Uri,
    to::To,
    Address,
    TypedHeader,
    Param,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding To headers to SIP message builders.
///
/// This trait provides a standard way to add To headers to both request and response builders
/// as specified in [RFC 3261 Section 20.39](https://datatracker.ietf.org/doc/html/rfc3261#section-20.39).
/// The To header field specifies the logical recipient of the request.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::ToBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .to("Bob", "sip:bob@example.com", Some("tag5678"))
///     .build();
/// ```
pub trait ToBuilderExt {
    /// Add a To header with an optional tag parameter.
    ///
    /// Creates and adds a To header with the specified display name, URI, and optional tag.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header (e.g., "Bob")
    /// - `uri`: The URI for the To header (e.g., "sip:bob@example.com")
    /// - `tag`: Optional tag parameter for dialog identification
    ///
    /// # Returns
    /// Self for method chaining
    fn to(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self;
}

impl ToBuilderExt for SimpleRequestBuilder {
    fn to(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::To(To::new(address)))
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::To(To::new(address)))
            }
        }
    }
}

impl ToBuilderExt for SimpleResponseBuilder {
    fn to(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::To(To::new(address)))
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.header(TypedHeader::To(To::new(address)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    
    #[test]
    fn test_request_to_header() {
        // Test with valid URI
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .build();
            
        let to_header = request.to().unwrap();
        assert_eq!(to_header.address().display_name(), Some("Bob"));
        assert_eq!(to_header.address().uri().to_string(), "sip:bob@example.com");
        assert_eq!(to_header.tag(), Some("tag5678"));
        
        // Test without tag
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .to("Bob", "sip:bob@example.com", None)
            .build();
            
        let to_header = request.to().unwrap();
        assert_eq!(to_header.tag(), None);
        
        // Test with invalid URI (should create a custom URI)
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .to("Bob", "invalid-uri", Some("tag5678"))
            .build();
            
        let to_header = request.to().unwrap();
        assert_eq!(to_header.address().display_name(), Some("Bob"));
        assert!(to_header.address().uri().to_string().contains("invalid-uri"));
    }
    
    #[test]
    fn test_response_to_header() {
        // Test with valid URI
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let to_header = response.to().unwrap();
        assert_eq!(to_header.address().display_name(), Some("Bob"));
        assert_eq!(to_header.address().uri().to_string(), "sip:bob@example.com");
        assert_eq!(to_header.tag(), Some("tag5678"));
        
        // Test without tag
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let to_header = response.to().unwrap();
        assert_eq!(to_header.tag(), None);
        
        // Test with invalid URI (should create a custom URI)
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "invalid-uri", Some("tag5678"))
            .call_id("test-call-id")
            .cseq(1, Method::Invite)
            .build();
            
        let to_header = response.to().unwrap();
        assert_eq!(to_header.address().display_name(), Some("Bob"));
        assert!(to_header.address().uri().to_string().contains("invalid-uri"));
    }
} 