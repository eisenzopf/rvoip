//! Require header builder
//!
//! This module provides builder methods for the Require header.

use crate::error::{Error, Result};
use crate::types::{
    require::Require,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Require header building capabilities to request and response builders
pub trait RequireBuilderExt {
    /// Add a Require header with a single option tag
    fn require_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Require header with multiple option tags
    fn require_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Require header for 100rel (reliable provisional responses)
    fn require_100rel(self) -> Self;
    
    /// Add a Require header for timer
    fn require_timer(self) -> Self;
    
    /// Add a Require header for path
    fn require_path(self) -> Self;
    
    /// Add a Require header for ICE negotiation
    fn require_ice(self) -> Self;
    
    /// Add a Require header with common WebRTC-related option tags
    fn require_webrtc(self) -> Self;
}

impl<T> RequireBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn require_tag(self, option_tag: impl Into<String>) -> Self {
        let require = Require::with_tag(option_tag);
        self.set_header(require)
    }
    
    fn require_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let mut tags = Vec::with_capacity(option_tags.len());
        for tag in option_tags {
            tags.push(tag.into());
        }
        let require = Require::new(tags);
        self.set_header(require)
    }
    
    fn require_100rel(self) -> Self {
        self.require_tag("100rel")
    }
    
    fn require_timer(self) -> Self {
        self.require_tag("timer")
    }
    
    fn require_path(self) -> Self {
        self.require_tag("path")
    }
    
    fn require_ice(self) -> Self {
        self.require_tag("ice")
    }
    
    fn require_webrtc(self) -> Self {
        self.require_tags(vec!["ice", "replaces"])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_require_tag() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_tag("100rel")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 1);
            assert!(require.requires("100rel"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_require_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .require_tags(vec!["100rel", "timer"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Require(require)) = response.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 2);
            assert!(require.requires("100rel"));
            assert!(require.requires("timer"));
            assert!(!require.requires("path"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_require_convenience_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_path()
            .build();
            
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert_eq!(require.option_tags.len(), 1);
            assert!(require.requires("path"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_require_webrtc() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .require_webrtc()
            .build();
            
        if let Some(TypedHeader::Require(require)) = request.header(&HeaderName::Require) {
            assert!(require.requires("ice"));
            assert!(require.requires("replaces"));
        } else {
            panic!("Require header not found or has wrong type");
        }
    }
} 