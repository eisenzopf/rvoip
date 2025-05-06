//! Call-ID header builder
//!
//! This module provides builder methods for the Call-ID header,
//! which is required in all SIP messages and used to uniquely identify
//! a dialog or registration.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::builder::headers::CallIdBuilderExt;
//! use rvoip_sip_core::types::Method;
//!
//! // Create a request with a custom Call-ID
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
//!     .build();
//!
//! // Create a request with a random Call-ID
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .random_call_id()
//!     .build();
//!
//! // Create a request with a random Call-ID including host
//! let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
//!     .random_call_id_with_host("example.com")
//!     .build();
//! ```

use crate::types::{
    call_id::CallId,
    TypedHeader,
};
use crate::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use crate::builder::headers::HeaderSetter;

/// Extension trait for adding Call-ID headers to SIP message builders.
///
/// This trait provides a standard way to add Call-ID headers to both request and response builders
/// as specified in [RFC 3261 Section 20.8](https://datatracker.ietf.org/doc/html/rfc3261#section-20.8).
/// The Call-ID header uniquely identifies a particular invitation or all registrations of a particular client.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::builder::headers::CallIdBuilderExt;
/// use rvoip_sip_core::types::Method;
///
/// // Create a request with a custom Call-ID
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
///     .build();
///
/// // Create a request with a random Call-ID
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .random_call_id()
///     .build();
///
/// // Create a request with a random Call-ID including host
/// let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com").unwrap()
///     .random_call_id_with_host("example.com")
///     .build();
/// ```
pub trait CallIdBuilderExt {
    /// Add a Call-ID header
    ///
    /// Creates and adds a Call-ID header as specified in [RFC 3261 Section 20.8](https://datatracker.ietf.org/doc/html/rfc3261#section-20.8).
    /// The Call-ID header uniquely identifies a particular invitation or all registrations of a particular client.
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value (e.g., "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
    ///
    /// # Returns
    /// Self for method chaining
    fn call_id(self, call_id: &str) -> Self;

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

impl CallIdBuilderExt for SimpleRequestBuilder {
    fn call_id(self, call_id: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::new(call_id)))
    }
    
    fn random_call_id(self) -> Self {
        self.header(TypedHeader::CallId(CallId::random()))
    }
    
    fn random_call_id_with_host(self, host: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::random_with_host(host)))
    }
}

impl CallIdBuilderExt for SimpleResponseBuilder {
    fn call_id(self, call_id: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::new(call_id)))
    }
    
    fn random_call_id(self) -> Self {
        self.header(TypedHeader::CallId(CallId::random()))
    }
    
    fn random_call_id_with_host(self, host: &str) -> Self {
        self.header(TypedHeader::CallId(CallId::random_with_host(host)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Method, StatusCode};
    use crate::types::headers::HeaderAccess;
    use crate::builder::headers::cseq::CSeqBuilderExt;

    #[test]
    fn test_request_call_id_header() {
        let call_id_value = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com";
        
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .call_id(call_id_value)
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        assert_eq!(call_id_headers[0].value(), call_id_value);
    }
    
    #[test]
    fn test_response_call_id_header() {
        let call_id_value = "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com";
        
        let response = SimpleResponseBuilder::ok()
            .from("Alice", "sip:alice@example.com", Some("tag1234"))
            .to("Bob", "sip:bob@example.com", Some("tag5678"))
            .call_id(call_id_value)
            .cseq_with_method(101, Method::Invite)
            .build();
            
        let call_id_headers = response.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        assert_eq!(call_id_headers[0].value(), call_id_value);
    }
    
    #[test]
    fn test_request_with_random_call_id() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .random_call_id()
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        // Check it's not empty and is a valid UUID
        let value = call_id_headers[0].value();
        assert!(!value.is_empty());
        assert!(uuid::Uuid::parse_str(&value).is_ok());
    }

    #[test]
    fn test_request_with_random_call_id_with_host() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .random_call_id_with_host("example.com")
            .build();
            
        let call_id_headers = request.all_headers().iter()
            .filter_map(|h| if let TypedHeader::CallId(c) = h { Some(c) } else { None })
            .collect::<Vec<_>>();
            
        assert_eq!(call_id_headers.len(), 1);
        let value = call_id_headers[0].value();
        let parts: Vec<&str> = value.split('@').collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[1], "example.com");
        // Check the first part is a valid UUID
        assert!(uuid::Uuid::parse_str(parts[0]).is_ok());
    }
} 