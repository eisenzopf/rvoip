//! Unsupported header builder
//!
//! This module provides builder methods for the Unsupported header.

use crate::error::{Error, Result};
use crate::types::{
    unsupported::Unsupported,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Unsupported header building capabilities to request and response builders
pub trait UnsupportedBuilderExt {
    /// Add an Unsupported header with a single option tag
    fn unsupported_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add an Unsupported header with multiple option tags
    fn unsupported_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add an Unsupported header for 100rel (reliable provisional responses)
    fn unsupported_100rel(self) -> Self;
    
    /// Add an Unsupported header for timer
    fn unsupported_timer(self) -> Self;
    
    /// Add an Unsupported header for path
    fn unsupported_path(self) -> Self;
}

impl<T> UnsupportedBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn unsupported_tag(self, option_tag: impl Into<String>) -> Self {
        let mut unsupported = Unsupported::new();
        let tag_str = option_tag.into();
        unsupported.add_option_tag(&tag_str);
        self.set_header(unsupported)
    }
    
    fn unsupported_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut unsupported = Unsupported::new();
        for tag_impl in option_tags {
            let tag = tag_impl.into();
            unsupported.add_option_tag(&tag);
        }
        self.set_header(unsupported)
    }
    
    fn unsupported_100rel(self) -> Self {
        self.unsupported_tag("100rel")
    }
    
    fn unsupported_timer(self) -> Self {
        self.unsupported_tag("timer")
    }
    
    fn unsupported_path(self) -> Self {
        self.unsupported_tag("path")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_unsupported_tag() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("100rel"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_unsupported_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .unsupported_tags(vec!["100rel", "path"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Unsupported(unsupported)) = response.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 2);
            assert!(unsupported.has_option_tag("100rel"));
            assert!(unsupported.has_option_tag("path"));
            assert!(!unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_unsupported_convenience_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_timer()
            .build();
            
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
    }

    #[test]
    fn test_unsupported_multiple_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .unsupported_timer()
            .unsupported_100rel()
            .build();
        
        // When adding multiple headers with the same name, they get added as separate headers
        // rather than being merged. The header() method returns the first one it finds.
        if let Some(TypedHeader::Unsupported(unsupported)) = request.header(&HeaderName::Unsupported) {
            assert_eq!(unsupported.option_tags().len(), 1);
            assert!(unsupported.has_option_tag("timer"));
        } else {
            panic!("Unsupported header not found or has wrong type");
        }
        
        // Verify that there are actually two Unsupported headers
        let unsupported_headers: Vec<_> = request.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::Unsupported(u) => Some(u),
                _ => None
            })
            .collect();
        
        assert_eq!(unsupported_headers.len(), 2);
        assert!(unsupported_headers[0].has_option_tag("timer"));
        assert!(unsupported_headers[1].has_option_tag("100rel"));
    }
} 