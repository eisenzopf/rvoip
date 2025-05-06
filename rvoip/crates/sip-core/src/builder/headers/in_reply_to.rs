//! In-Reply-To header builder
//!
//! This module provides builder methods for the In-Reply-To header,
//! which allows SIP requests to reference previous Call-IDs as defined
//! in RFC 3261 Section 20.22.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a request with a single In-Reply-To Call-ID
//! let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .in_reply_to("70710@saturn.bell-tel.com")
//!     .build();
//!
//! // Create a request with multiple In-Reply-To Call-IDs
//! let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .in_reply_to_multiple(vec!["70710@saturn.bell-tel.com", "17320@venus.bell-tel.com"])
//!     .build();
//! ```

use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    in_reply_to::InReplyTo,
    call_id::CallId,
};
use crate::builder::headers::HeaderSetter;

/// Extension trait that adds In-Reply-To building capabilities to request and response builders
pub trait InReplyToBuilderExt {
    /// Add an In-Reply-To header with a single Call-ID
    ///
    /// This method sets the In-Reply-To header with a single Call-ID value.
    /// The In-Reply-To header is used to reference Call-IDs of previous requests.
    ///
    /// # Parameters
    ///
    /// - `call_id`: The Call-ID value to reference
    ///
    /// # Returns
    ///
    /// The builder with the In-Reply-To header set
    fn in_reply_to(self, call_id: &str) -> Self;

    /// Add an In-Reply-To header with multiple Call-IDs
    ///
    /// This method sets the In-Reply-To header with multiple Call-ID values.
    /// The In-Reply-To header is used to reference Call-IDs of previous requests.
    ///
    /// # Parameters
    ///
    /// - `call_ids`: A vector of Call-ID strings to reference
    ///
    /// # Returns
    ///
    /// The builder with the In-Reply-To header set
    fn in_reply_to_multiple(self, call_ids: Vec<&str>) -> Self;
}

impl<T> InReplyToBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn in_reply_to(self, call_id: &str) -> Self {
        let in_reply_to = InReplyTo::new(call_id);
        self.set_header(in_reply_to)
    }

    fn in_reply_to_multiple(self, call_ids: Vec<&str>) -> Self {
        let call_id_strings = call_ids.into_iter().map(|s| s.to_string()).collect();
        let in_reply_to = InReplyTo::with_multiple_strings(call_id_strings);
        self.set_header(in_reply_to)
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
    fn test_request_with_single_in_reply_to() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .in_reply_to("70710@saturn.bell-tel.com")
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = request.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 1);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_multiple_in_reply_to() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .in_reply_to_multiple(vec![
                "70710@saturn.bell-tel.com", 
                "17320@venus.bell-tel.com"
            ])
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = request.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 2);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
            assert!(in_reply_to.contains("17320@venus.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_with_in_reply_to() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .in_reply_to("70710@saturn.bell-tel.com")
            .build();
            
        if let Some(TypedHeader::InReplyTo(in_reply_to)) = response.header(&HeaderName::InReplyTo) {
            assert_eq!(in_reply_to.len(), 1);
            assert!(in_reply_to.contains("70710@saturn.bell-tel.com"));
        } else {
            panic!("In-Reply-To header not found or has wrong type");
        }
    }
} 