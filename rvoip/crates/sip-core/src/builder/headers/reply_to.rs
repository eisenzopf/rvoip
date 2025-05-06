//! Reply-To header builder
//!
//! This module provides builder methods for the Reply-To header,
//! which specifies where the user would prefer responses to be sent,
//! as defined in RFC 3261 Section 20.32.
//!
//! # Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::builder::headers::ReplyToBuilderExt;
//! use std::str::FromStr;
//!
//! // Create a request with a Reply-To header using a string URI
//! let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .reply_to("sip:support@example.com").unwrap()
//!     .build();
//!
//! // Create a request with a Reply-To header with a display name
//! let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
//!     .reply_to_with_display_name("Support Team", "sip:support@example.com").unwrap()
//!     .build();
//! ```

use crate::error::{Error, Result};
use crate::types::{
    header::{Header, HeaderName},
    headers::TypedHeader,
    reply_to::ReplyTo,
    address::Address,
    uri::Uri,
};
use crate::builder::headers::HeaderSetter;
use std::str::FromStr;

/// Extension trait that adds Reply-To building capabilities to request and response builders
pub trait ReplyToBuilderExt {
    /// Set a Reply-To header using a URI string
    ///
    /// This method sets the Reply-To header with the provided URI string.
    /// The Reply-To header indicates where the user would prefer replies to be sent.
    ///
    /// # Parameters
    ///
    /// - `uri_str`: A string representation of the SIP URI
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set
    ///
    /// # Errors
    ///
    /// Returns an error if the URI string is invalid
    fn reply_to(self, uri_str: &str) -> Result<Self>
    where
        Self: Sized;

    /// Set a Reply-To header using a URI string and display name
    ///
    /// This method sets the Reply-To header with the provided URI string and
    /// a display name. The Reply-To header indicates where the user would
    /// prefer replies to be sent.
    ///
    /// # Parameters
    ///
    /// - `display_name`: The display name to show
    /// - `uri_str`: A string representation of the SIP URI
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set
    ///
    /// # Errors
    ///
    /// Returns an error if the URI string is invalid
    fn reply_to_with_display_name(self, display_name: &str, uri_str: &str) -> Result<Self>
    where
        Self: Sized;

    /// Set a Reply-To header using a pre-constructed Address
    ///
    /// This method sets the Reply-To header with the provided Address object.
    /// The Reply-To header indicates where the user would prefer replies to be sent.
    ///
    /// # Parameters
    ///
    /// - `address`: The Address object to use
    ///
    /// # Returns
    ///
    /// The builder with the Reply-To header set
    fn reply_to_address(self, address: Address) -> Self;
}

impl<T> ReplyToBuilderExt for T 
where 
    T: HeaderSetter,
{
    fn reply_to(self, uri_str: &str) -> Result<Self> {
        let uri = Uri::from_str(uri_str)?;
        let address = Address::new(uri);
        Ok(self.reply_to_address(address))
    }

    fn reply_to_with_display_name(self, display_name: &str, uri_str: &str) -> Result<Self> {
        let uri = Uri::from_str(uri_str)?;
        let address = Address::new_with_display_name(display_name, uri);
        Ok(self.reply_to_address(address))
    }

    fn reply_to_address(self, address: Address) -> Self {
        let reply_to = ReplyTo::new(address);
        self.set_header(reply_to)
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
    fn test_request_with_reply_to_uri() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to("sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
            assert_eq!(reply_to.address().display_name(), None);
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_reply_to_and_display_name() {
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to_with_display_name("Support Team", "sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
            assert_eq!(reply_to.address().display_name(), Some("Support Team"));
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_request_with_reply_to_address() {
        let uri = Uri::from_str("sip:sales@example.com").unwrap();
        let address = Address::new_with_display_name("Sales Department", uri);
        
        let request = RequestBuilder::new(Method::Invite, "sip:example.com").unwrap()
            .reply_to_address(address)
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = request.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:sales@example.com");
            assert_eq!(reply_to.address().display_name(), Some("Sales Department"));
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }

    #[test]
    fn test_response_with_reply_to() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .reply_to("sip:support@example.com").unwrap()
            .build();
            
        if let Some(TypedHeader::ReplyTo(reply_to)) = response.header(&HeaderName::ReplyTo) {
            assert_eq!(reply_to.uri().to_string(), "sip:support@example.com");
        } else {
            panic!("Reply-To header not found or has wrong type");
        }
    }
} 