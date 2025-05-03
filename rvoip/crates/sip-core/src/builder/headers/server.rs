//! Server header builder
//!
//! This module provides builder methods for the Server header.
//! 
//! # Examples
//! 
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! 
//! // Create a response with a simple Server header
//! let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
//!     .server("RVOIP/1.0")
//!     .build();
//! 
//! // Create a response with a more detailed Server header
//! let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
//!     .server_products(vec!["RVOIP/1.0", "Rust/1.68", "(Linux x86_64)"])
//!     .build();
//! 
//! // Check the Server header on a response
//! if let Some(TypedHeader::Server(products)) = response.header(&HeaderName::Server) {
//!     println!("Server: {}", products.join(" "));
//! }
//! ```

use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
};
use crate::types::server::ServerInfo;
use super::HeaderSetter;

/// Extension trait that adds Server header building capabilities to request and response builders
pub trait ServerBuilderExt {
    /// Add a Server header with a single product token
    fn server(self, product: impl Into<String>) -> Self;
    
    /// Add a Server header with multiple product tokens
    fn server_products(self, products: Vec<impl Into<String>>) -> Self;
}

impl<T> ServerBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn server(self, product: impl Into<String>) -> Self {
        let server = ServerInfo::new()
            .with_product(&product.into(), None);
        self.set_header(server)
    }
    
    fn server_products(self, products: Vec<impl Into<String>>) -> Self {
        let mut server = ServerInfo::new();
        
        for product in products {
            let product_str = product.into();
            
            // Check if it's a comment (in parentheses)
            if product_str.starts_with('(') && product_str.ends_with(')') {
                let comment = &product_str[1..product_str.len()-1];
                server = server.with_comment(comment);
            } else if let Some(pos) = product_str.find('/') {
                // It's a product with version
                let (name, version) = product_str.split_at(pos);
                server = server.with_product(name, Some(&version[1..]));
            } else {
                // Just a product name without version
                server = server.with_product(&product_str, None);
            }
        }
        
        self.set_header(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_response_server() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server("Example-SIP-Server/1.0")
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
            assert_eq!(server.len(), 1);
            assert_eq!(server[0], "Example-SIP-Server/1.0");
        } else {
            panic!("Server header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_server_products() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server_products(vec!["Example-SIP-Server/1.0", "(Platform/OS Version)"])
            .build();
            
        let headers = &response.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
            assert_eq!(server.len(), 2);
            assert_eq!(server[0], "Example-SIP-Server/1.0");
            assert_eq!(server[1], "(Platform/OS Version)");
        } else {
            panic!("Server header not found or has wrong type");
        }
    }

    #[test]
    fn test_multiple_server_headers() {
        let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .server("First-Server/1.0")
            .server("Second-Server/2.0")
            .build();
        
        // Get all Server headers
        let server_headers: Vec<_> = response.headers.iter()
            .filter_map(|h| match h {
                TypedHeader::Server(s) => Some(s),
                _ => None
            })
            .collect();
        
        // Check header count - there might be 1 or 2 depending on implementation
        if server_headers.len() == 1 {
            // If there's only one header (replacement occurred), it should be the last one set
            assert_eq!(server_headers[0][0], "Second-Server/2.0");
        } else if server_headers.len() == 2 {
            // If there are two headers (append occurred), they should be in order of addition
            assert_eq!(server_headers[0][0], "First-Server/1.0");
            assert_eq!(server_headers[1][0], "Second-Server/2.0");
            
            // But the response.header() method should return the first matching header
            if let Some(TypedHeader::Server(server)) = response.header(&HeaderName::Server) {
                assert_eq!(server[0], "First-Server/1.0");
            } else {
                panic!("Server header not found or has wrong type");
            }
        } else {
            panic!("Unexpected number of Server headers: {}", server_headers.len());
        }
    }
} 