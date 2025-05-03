//! Call-ID header builder
//!
//! This module provides builder methods for the Call-ID header,
//! which is required in all SIP messages and used to uniquely identify
//! a dialog or registration.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a request with a custom Call-ID
//! let request = RequestBuilder::new(Method::Register, "sip:example.com")
//!     .call_id_header("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
//!     .build();
//!
//! // Create a request with a random Call-ID
//! let request = RequestBuilder::new(Method::Register, "sip:example.com")
//!     .random_call_id()
//!     .build();
//!
//! // Create a request with a random Call-ID including host
//! let request = RequestBuilder::new(Method::Register, "sip:example.com")
//!     .random_call_id_with_host("example.com")
//!     .build();
//! ```

use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    call_id::CallId,
};
use crate::builder::headers::HeaderSetter;

/// Extension trait that adds Call-ID building capabilities to request and response builders
pub trait CallIdBuilderExt {
    /// Add a Call-ID header with a custom value
    ///
    /// This method sets the Call-ID header with a custom string value.
    ///
    /// # Parameters
    ///
    /// - `call_id`: The Call-ID value to use
    ///
    /// # Returns
    ///
    /// The builder with the Call-ID header set
    fn call_id_header(self, call_id: &str) -> Self;

    /// Add a randomly generated Call-ID header
    ///
    /// This method sets a Call-ID header with a randomly generated UUID,
    /// providing a high probability of uniqueness.
    ///
    /// # Returns
    ///
    /// The builder with a random Call-ID header set
    fn random_call_id(self) -> Self;

    /// Add a randomly generated Call-ID header with a host part
    ///
    /// This method sets a Call-ID header with a random UUID and appends
    /// a host part, following the recommended format in RFC 3261.
    ///
    /// # Parameters
    ///
    /// - `host`: The host part to append (domain name or IP address)
    ///
    /// # Returns
    ///
    /// The builder with a random Call-ID header including host set
    fn random_call_id_with_host(self, host: &str) -> Self;
}

impl<T> CallIdBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn call_id_header(self, call_id: &str) -> Self {
        let call_id = CallId::new(call_id);
        self.set_header(call_id)
    }

    fn random_call_id(self) -> Self {
        let call_id = CallId::random();
        self.set_header(call_id)
    }

    fn random_call_id_with_host(self, host: &str) -> Self {
        let call_id = CallId::random_with_host(host);
        self.set_header(call_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{method::Method, StatusCode};
    use crate::{RequestBuilder, ResponseBuilder};
    use crate::types::headers::HeaderAccess;
    use std::str::FromStr;

    #[test]
    fn test_request_with_custom_call_id() {
        let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
            .call_id_header("test-call-id@example.com")
            .build();
            
        if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
            assert_eq!(call_id.as_str(), "test-call-id@example.com");
        } else {
            panic!("Call-ID header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_random_call_id() {
        let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
            .random_call_id()
            .build();
            
        if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
            // Just check it's not empty and is a valid UUID
            assert!(!call_id.as_str().is_empty());
            assert!(uuid::Uuid::parse_str(call_id.as_str()).is_ok());
        } else {
            panic!("Call-ID header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_random_call_id_with_host() {
        let request = RequestBuilder::new(Method::Register, "sip:example.com").unwrap()
            .random_call_id_with_host("example.com")
            .build();
            
        if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
            let parts: Vec<&str> = call_id.as_str().split('@').collect();
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[1], "example.com");
            // Check the first part is a valid UUID
            assert!(uuid::Uuid::parse_str(parts[0]).is_ok());
        } else {
            panic!("Call-ID header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_with_custom_call_id() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .call_id_header("test-call-id@example.com")
            .build();
            
        if let Some(TypedHeader::CallId(call_id)) = response.header(&HeaderName::CallId) {
            assert_eq!(call_id.as_str(), "test-call-id@example.com");
        } else {
            panic!("Call-ID header not found or has wrong type");
        }
    }
} 