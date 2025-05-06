//! Proxy-Require header builder
//!
//! This module provides builder methods for the Proxy-Require header.
//! 
//! # Examples
//! 
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! 
//! // Create a request with a Proxy-Require header containing a single option tag
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .proxy_require_tag("sec-agree")
//!     .build();
//! 
//! // Create a request with a Proxy-Require header containing multiple option tags
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .proxy_require_tags(vec!["sec-agree", "precondition"])
//!     .build();
//! ```

use crate::error::{Error, Result};
use crate::types::{
    ProxyRequire,
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use super::HeaderSetter;

/// Extension trait that adds Proxy-Require header building capabilities to request and response builders
pub trait ProxyRequireBuilderExt {
    /// Add a Proxy-Require header with a single option tag
    fn proxy_require_tag(self, option_tag: impl Into<String>) -> Self;
    
    /// Add a Proxy-Require header with multiple option tags
    fn proxy_require_tags(self, option_tags: Vec<impl Into<String>>) -> Self;
    
    /// Add a Proxy-Require header for sec-agree (security agreement)
    fn proxy_require_sec_agree(self) -> Self;
    
    /// Add a Proxy-Require header for precondition
    fn proxy_require_precondition(self) -> Self;
    
    /// Add a Proxy-Require header for path
    fn proxy_require_path(self) -> Self;
    
    /// Add a Proxy-Require header for resource priority
    fn proxy_require_resource_priority(self) -> Self;
}

impl<T> ProxyRequireBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn proxy_require_tag(self, option_tag: impl Into<String>) -> Self {
        let proxy_require = ProxyRequire::single(&option_tag.into());
        self.set_header(proxy_require)
    }
    
    fn proxy_require_tags(self, option_tags: Vec<impl Into<String>>) -> Self {
        let tags: Vec<String> = option_tags.into_iter().map(Into::into).collect();
        let proxy_require = ProxyRequire::with_options(&tags);
        self.set_header(proxy_require)
    }
    
    fn proxy_require_sec_agree(self) -> Self {
        self.proxy_require_tag("sec-agree")
    }
    
    fn proxy_require_precondition(self) -> Self {
        self.proxy_require_tag("precondition")
    }
    
    fn proxy_require_path(self) -> Self {
        self.proxy_require_tag("path")
    }
    
    fn proxy_require_resource_priority(self) -> Self {
        self.proxy_require_tag("resource-priority")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_proxy_require_tag() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .proxy_require_tag("sec-agree")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = request.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 1);
            assert!(proxy_require.has_option("sec-agree"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_proxy_require_tags() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .proxy_require_tags(vec!["sec-agree", "precondition"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = response.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 2);
            assert!(proxy_require.has_option("sec-agree"));
            assert!(proxy_require.has_option("precondition"));
            assert!(!proxy_require.has_option("path"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }

    #[test]
    fn test_proxy_require_convenience_methods() {
        let request = RequestBuilder::new(Method::Invite, "sip:alice@example.com").unwrap()
            .proxy_require_path()
            .build();
            
        if let Some(TypedHeader::ProxyRequire(proxy_require)) = request.header(&HeaderName::ProxyRequire) {
            assert_eq!(proxy_require.options().len(), 1);
            assert!(proxy_require.has_option("path"));
        } else {
            panic!("Proxy-Require header not found or has wrong type");
        }
    }
} 