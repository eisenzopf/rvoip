//! Supported header builder
//!
//! This module provides builder methods for the Supported header.

use crate::error::{Error, Result};
use crate::types::{
    supported::Supported,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Supported header building capabilities to request and response builders
pub trait SupportedBuilderExt {
    /// Add a Supported header with a single option tag
    fn supported_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Supported header with multiple option tags
    fn supported_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Supported header for 100rel (reliable provisional responses)
    fn supported_100rel(self) -> Self;
    
    /// Add a Supported header for path
    fn supported_path(self) -> Self;
    
    /// Add a Supported header for timer
    fn supported_timer(self) -> Self;
    
    /// Add a Supported header with common WebRTC-related option tags
    fn supported_webrtc(self) -> Self;
    
    /// Add a Supported header with standard option tags used by UAs
    fn supported_standard(self) -> Self;
}

impl<T> SupportedBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn supported_tag(self, option_tag: impl Into<String>) -> Self {
        let supported = Supported::with_tag(option_tag);
        self.set_header(supported)
    }
    
    fn supported_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut tags = Vec::with_capacity(option_tags.len());
        for tag in option_tags {
            tags.push(tag.into());
        }
        let supported = Supported::new(tags);
        self.set_header(supported)
    }
    
    fn supported_100rel(self) -> Self {
        self.supported_tag("100rel")
    }
    
    fn supported_path(self) -> Self {
        self.supported_tag("path")
    }
    
    fn supported_timer(self) -> Self {
        self.supported_tag("timer")
    }
    
    fn supported_webrtc(self) -> Self {
        self.supported_tags(vec!["ice", "replaces", "outbound", "gruu"])
    }
    
    fn supported_standard(self) -> Self {
        self.supported_tags(vec!["100rel", "path", "timer", "replaces"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_supported_tag() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 1);
            assert!(supported.supports("100rel"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_supported_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .supported_tags(vec!["100rel", "path"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Supported(supported)) = response.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 2);
            assert!(supported.supports("100rel"));
            assert!(supported.supports("path"));
            assert!(!supported.supports("timer"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_supported_convenience_methods() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_timer()
            .build();
            
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert_eq!(supported.option_tags.len(), 1);
            assert!(supported.supports("timer"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }

    #[test]
    fn test_supported_webrtc() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .supported_webrtc()
            .build();
            
        if let Some(TypedHeader::Supported(supported)) = request.header(&HeaderName::Supported) {
            assert!(supported.supports("ice"));
            assert!(supported.supports("replaces"));
            assert!(supported.supports("outbound"));
            assert!(supported.supports("gruu"));
        } else {
            panic!("Supported header not found or has wrong type");
        }
    }
} 