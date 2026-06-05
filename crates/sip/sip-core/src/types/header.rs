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

// Import the parser module
// Import the types module itself
// Import DateTime specifically
// Import FixedOffset
use std::string::FromUtf8Error; // Import FromUtf8Error

// Import directly from parser
// Keep parser type if no types::* yet
// Import directly from parser
// Keep parser type if no types::* yet
// Import from parser
// Import RouteEntry from parser
// Use our new AcceptLanguage type
// Add explicit import for Address
// Import Contact
// Import ContentDisposition
// Rename From to avoid conflict
// Needed for Allow parsing
// Import Priority type
// Import RecordRouteEntry from types module
// Add ReferTo import
// Import Require type
// Import Subject type
// Import Supported type
// Rename To to avoid conflict
// Import Unsupported type
// Import Scheme
// Import Uri
// Import both Via and ViaHeader
// Add WarnAgent import

// Add log for debug printing
extern crate log;

// Import the HeaderName and HeaderValue from headers module
use crate::types::headers;

// Re-export the HeaderName, HeaderValue, and TypedHeader publically
pub use headers::header::Header;
pub use headers::header_name::HeaderName;
pub use headers::header_value::HeaderValue;
pub use headers::typed_header::{TypedHeader, TypedHeaderTrait};

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
