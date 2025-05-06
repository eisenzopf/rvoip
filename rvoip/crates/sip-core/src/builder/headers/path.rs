//! Path header builder
//!
//! This module provides builder methods for the Path header.
//! 
//! # Examples
//! 
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! 
//! // Create a request with a Path header containing a single URI
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .path("sip:proxy.example.com;lr").unwrap()
//!     .build();
//! 
//! // Create a request with a Path header containing multiple URIs
//! let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .path_addresses(vec!["sip:p1.example.com;lr", "sip:p2.example.com;lr"]).unwrap()
//!     .build();
//! 
//! // Check the Path header on a response
//! if let Some(TypedHeader::Path(path)) = request.header(&HeaderName::Path) {
//!     println!("Found Path: {}", path);
//! }
//! ```

use std::str::FromStr;
use crate::error::{Error, Result};
use crate::types::{
    headers::HeaderName,
    headers::TypedHeader,
    headers::header_access::HeaderAccess,
    path::Path,
    uri::Uri,
    Address
};
use super::HeaderSetter;

/// Extension trait that adds Path header building capabilities to request and response builders
pub trait PathBuilderExt {
    /// Add a Path header with a single URI
    fn path(self, uri: impl AsRef<str>) -> Result<Self> where Self: Sized;
    
    /// Add a Path header with multiple URIs
    fn path_addresses(self, uris: Vec<impl AsRef<str>>) -> Result<Self> where Self: Sized;
}

impl<T> PathBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn path(self, uri: impl AsRef<str>) -> Result<Self> {
        let uri = Uri::from_str(uri.as_ref())?;
        Ok(self.set_header(Path::with_uri(uri)))
    }
    
    fn path_addresses(self, uris: Vec<impl AsRef<str>>) -> Result<Self> {
        let mut path = Path::empty();
        
        for uri_str in uris {
            let uri = Uri::from_str(uri_str.as_ref())?;
            path.add_uri(uri);
        }
        
        Ok(self.set_header(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, uri::Uri, version::Version, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use std::str::FromStr;

    #[test]
    fn test_request_path() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path("sip:proxy.example.com;lr").unwrap()
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Path(path)) = request.header(&HeaderName::Path) {
            assert_eq!(path.len(), 1);
            assert_eq!(path[0].0.uri.to_string(), "sip:proxy.example.com;lr");
        } else {
            panic!("Path header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_path_addresses() {
        let request = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path_addresses(vec!["sip:p1.example.com;lr", "sip:p2.example.com;lr"]).unwrap()
            .build();
            
        let headers = &request.headers;
        assert_eq!(headers.len(), 1);
        
        if let Some(TypedHeader::Path(path)) = request.header(&HeaderName::Path) {
            assert_eq!(path.len(), 2);
            assert_eq!(path[0].0.uri.to_string(), "sip:p1.example.com;lr");
            assert_eq!(path[1].0.uri.to_string(), "sip:p2.example.com;lr");
        } else {
            panic!("Path header not found or has wrong type");
        }
    }

    #[test]
    fn test_error_handling() {
        // Invalid URI
        let result = RequestBuilder::new(Method::Register, "sip:registrar.example.com").unwrap()
            .path("invalid uri");
            
        assert!(result.is_err());
    }
} 