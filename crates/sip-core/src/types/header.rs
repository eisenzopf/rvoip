//! # SIP Headers
//! 
//! This module provides a comprehensive implementation of SIP headers as defined in 
//! [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261) and related RFCs.
//! 
//! The header system is built around three key types:
//! 
//! - [`HeaderName`]: Represents standard and custom SIP header names
//! - [`TypedHeader`]: A strongly-typed representation of parsed SIP headers
//! - [`Header`]: A more generic representation with [`HeaderName`] and [`HeaderValue`]
//! 
//! ## Architecture
//! 
//! The header system uses a two-tiered approach:
//! 
//! 1. During parsing, headers are initially parsed into a [`Header`] with a [`HeaderName`] and 
//!    possibly complex [`HeaderValue`].
//! 
//! 2. These can then be converted into [`TypedHeader`] variants which provide a strongly-typed 
//!    API for each header type.
//! 
//! This design allows for both flexibility when handling unknown headers and type safety
//! when working with standard headers.
//! 
//! ## Usage Examples
//! 
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//! 
//! // Creating a typed header directly
//! let call_id = CallId::new("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
//! let header = TypedHeader::CallId(call_id);
//! 
//! // Working with generic headers
//! let generic_header = Header::text(HeaderName::CallId, "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com");
//! assert_eq!(generic_header.name, HeaderName::CallId);
//! 
//! // Parsing a header through a parser (not directly with from_str)
//! // This is just an example, not actual code to run
//! // let from_header = TypedHeader::try_from(parse_header(header_str).unwrap()).unwrap();
//! ```

use crate::error::{Error, Result};
use crate::types; // Import the types module itself
use crate::parser; // Import the parser module
use std::convert::TryFrom;
use nom::combinator::all_consuming;
use ordered_float::NotNan;
use chrono::DateTime; // Import DateTime specifically
use chrono::FixedOffset; // Import FixedOffset
use std::fmt;
use std::str::FromStr;
use std::string::FromUtf8Error; // Import FromUtf8Error

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::types::param::Param;
use crate::types::uri::Uri; // Import Uri
use crate::types::uri::Scheme; // Import Scheme
use crate::types::address::Address; // Add explicit import for Address
use crate::types::contact::{Contact, ContactValue as TypesContactValue}; // Import Contact
use crate::types::from::From as FromHeaderValue; // Rename From to avoid conflict
use crate::types::to::To as ToHeaderValue; // Rename To to avoid conflict
use crate::types::route::Route;
use crate::parser::headers::route::RouteEntry; // Import RouteEntry from parser
use crate::types::record_route::RecordRoute;
use crate::types::record_route::RecordRouteEntry; // Import RecordRouteEntry from types module
use crate::types::via::{Via, ViaHeader}; // Import both Via and ViaHeader
use crate::types::cseq::CSeq;
use crate::types::call_id::CallId;
use crate::types::content_length::ContentLength;
use crate::types::content_type::ContentType;
use crate::parser::headers::content_type::ContentTypeValue; // Import directly from parser
use crate::types::expires::Expires;
use crate::types::max_forwards::MaxForwards;
use crate::types::allow::Allow;
use crate::types::accept::Accept;
use crate::parser::headers::accept::AcceptValue; // Import directly from parser
use crate::types::auth::{Authorization, WwwAuthenticate, ProxyAuthenticate, ProxyAuthorization, AuthenticationInfo};
use crate::types::reply_to::ReplyTo;
use crate::parser::headers::reply_to::ReplyToValue; // Import from parser
use crate::types::warning::{Warning, WarnAgent}; // Add WarnAgent import
use crate::types::content_disposition::{ContentDisposition, DispositionType, DispositionParam, Handling}; // Import ContentDisposition
use crate::types::method::Method; // Needed for Allow parsing
use crate::types::priority::Priority; // Import Priority type
use crate::types::require::Require; // Import Require type
use crate::parser::headers::content_type::parse_content_type_value;
use crate::types::retry_after::RetryAfter;
use crate::types::subject::Subject; // Import Subject type
use crate::types::accept_language::AcceptLanguage; // Use our new AcceptLanguage type
use crate::parser::headers::alert_info::AlertInfoValue; // Keep parser type if no types::* yet
use crate::parser::headers::error_info::ErrorInfoValue; // Keep parser type if no types::* yet
use crate::types::refer_to::ReferTo; // Add ReferTo import
use crate::types::call_info::{CallInfo, CallInfoValue};
use crate::types::supported::Supported; // Import Supported type
use crate::types::unsupported::Unsupported; // Import Unsupported type
use crate::types::accept_encoding::AcceptEncoding;
use crate::types::content_encoding::ContentEncoding;
use crate::types::content_language::ContentLanguage;
use crate::parser::headers::accept_encoding::EncodingInfo;
use crate::prelude::GenericValue;

// Add log for debug printing
extern crate log;
use log::debug;

// Import the HeaderName and HeaderValue from headers module
use crate::types::headers;

// Re-export the HeaderName, HeaderValue, and TypedHeader publically
pub use headers::header_name::HeaderName;
pub use headers::header_value::HeaderValue;
pub use headers::typed_header::{TypedHeader, TypedHeaderTrait};
pub use headers::header::Header;

// Helper From implementation for Error
impl From<FromUtf8Error> for crate::error::Error {
    fn from(err: FromUtf8Error) -> Self {
        crate::error::Error::ParseError(format!("UTF-8 Error: {}", err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_header_name_from_str() {
        assert_eq!(HeaderName::from_str("Via").unwrap(), HeaderName::Via);
        assert_eq!(HeaderName::from_str("v").unwrap(), HeaderName::Via);
        assert_eq!(HeaderName::from_str("To").unwrap(), HeaderName::To);
        assert_eq!(HeaderName::from_str("t").unwrap(), HeaderName::To);
        assert_eq!(HeaderName::from_str("cSeQ").unwrap(), HeaderName::CSeq);
        
        // Extension header
        let custom = HeaderName::from_str("X-Custom").unwrap();
        assert!(matches!(custom, HeaderName::Other(s) if s == "X-Custom"));
        
        // Empty header name is invalid
        assert!(HeaderName::from_str("").is_err());
    }
} 