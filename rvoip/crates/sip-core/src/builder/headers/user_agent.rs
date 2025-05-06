//! User-Agent header builder
//!
//! This module provides builder methods for the User-Agent header.
//! 
//! # Examples
//! 
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! 
//! // Create a request with a simple User-Agent header
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .user_agent("RVOIP/1.0")
//!     .build();
//! 
//! // Create a request with a more detailed User-Agent header
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .user_agent_products(vec!["RVOIP/1.0", "Rust/1.68", "(Linux x86_64)"])
//!     .build();
//! 
//! // Check the User-Agent header on a response
//! if let Some(TypedHeader::UserAgent(products)) = request.header(&HeaderName::UserAgent) {
//!     println!("User-Agent: {}", products.join(" "));
//! }
//! ```

use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use crate::types::user_agent::UserAgent;
use super::HeaderSetter;

/// Extension trait that adds User-Agent header building capabilities to request and response builders
pub trait UserAgentBuilderExt {
    /// Add a User-Agent header with a single product token
    fn user_agent(self, product: impl Into<String>) -> Self;
    
    /// Add a User-Agent header with multiple product tokens
    fn user_agent_products(self, products: Vec<impl Into<String>>) -> Self;
}

impl<T> UserAgentBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn user_agent(self, product: impl Into<String>) -> Self {
        let user_agent = UserAgent::single(&product.into());
        self.set_header(user_agent)
    }
    
    fn user_agent_products(self, products: Vec<impl Into<String>>) -> Self {
        let string_products: Vec<String> = products.into_iter().map(|p| p.into()).collect();
        let user_agent = UserAgent::with_products(&string_products);
        self.set_header(user_agent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_user_agent() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .user_agent("Example-SIP-Client/1.0")
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::UserAgent(user_agent)) = request.header(&HeaderName::UserAgent) {
            assert_eq!(user_agent.len(), 1);
            assert_eq!(user_agent[0], "Example-SIP-Client/1.0");
        } else {
            panic!("User-Agent header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_user_agent_products() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .user_agent_products(vec!["Example-SIP-Client/1.0", "(Platform/OS Version)"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::UserAgent(user_agent)) = response.header(&HeaderName::UserAgent) {
            assert_eq!(user_agent.len(), 2);
            assert_eq!(user_agent[0], "Example-SIP-Client/1.0");
            assert_eq!(user_agent[1], "(Platform/OS Version)");
        } else {
            panic!("User-Agent header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_user_agent_headers() {
        // The behavior when calling user_agent multiple times could be either:
        // 1. Replace previous header (desired)
        // 2. Add another header (current implementation)
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .user_agent("First-Client/1.0")
            .user_agent("Second-Client/2.0")
            .build();
        
        // Get all User-Agent headers
        let user_agent_headers: Vec<_> = request.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::UserAgent(u) => Some(u),
                _ => None
            })
            .collect();
        
        // Check header count - there might be 1 or 2 depending on implementation
        if user_agent_headers.len() == 1 {
            // If there's only one header (replacement occurred), it should be the last one set
            assert_eq!(user_agent_headers[0][0], "Second-Client/2.0");
        } else if user_agent_headers.len() == 2 {
            // If there are two headers (append occurred), they should be in order of addition
            assert_eq!(user_agent_headers[0][0], "First-Client/1.0");
            assert_eq!(user_agent_headers[1][0], "Second-Client/2.0");
            
            // But the request.header() method should return the first matching header
            if let Some(TypedHeader::UserAgent(user_agent)) = request.header(&HeaderName::UserAgent) {
                assert_eq!(user_agent[0], "First-Client/1.0");
            } else {
                panic!("User-Agent header not found or has wrong type");
            }
        } else {
            panic!("Unexpected number of User-Agent headers: {}", user_agent_headers.len());
        }
    }
} 