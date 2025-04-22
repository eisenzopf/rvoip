//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

// Core parsing modules
pub mod common;
// mod basic_rules; // REMOVED
pub mod headers;
pub mod message;
pub mod multipart;
mod request;
mod response;
pub mod uri;
mod utils;
pub mod address;
mod common_params;
mod utf8;

// Re-export top-level parsers and types, consolidate duplicates
pub use message::{parse_message, IncrementalParser, ParseState};
// pub use request::request_parser; // Removed
// pub use response::response_parser; // Removed
// Commenting out potentially unresolved imports
pub use uri::{parse_uri /*, parse_uri_params, parse_host_port*/ };
pub use multipart::{parse_multipart};
pub use crate::types::multipart::{MimePart, MultipartBody};

// Re-export specific header parsers needed by types/header.rs
// TODO: Update these exports once individual header parsers are implemented in headers/
pub use headers::{
    parse_via,
    // parse_address, // Keep commented until implemented
    parse_contact,
    parse_from,
    parse_to,
    parse_route,
    parse_record_route,
    parse_cseq,
    parse_max_forwards,
    parse_expires,
    parse_content_length,
    parse_call_id,
    parse_reply_to,
    parse_allow,
    parse_content_type,
    parse_content_disposition,
    parse_accept,
    parse_warning,
    parse_accept_encoding,
    parse_accept_language,
    parse_content_encoding,
    parse_content_language,
    parse_alert_info,
    parse_call_info,
    parse_error_info,
    parse_retry_after,
    parse_www_authenticate,
    parse_authorization,
    parse_proxy_authenticate,
    parse_proxy_authorization,
    parse_authentication_info,
};

// Maybe re-export specific header parsers if needed directly?
// pub use headers::{parse_via, parse_cseq, ...}; 

// Type alias for parser result
pub type ParseResult<'a, O> = nom::IResult<&'a [u8], O, nom::error::Error<&'a [u8]>>;

// Declare parser submodules
pub mod common_chars;
pub mod whitespace;
pub mod separators;
pub mod token;
pub mod quoted;
pub mod values;

// pub(crate) use basic_rules::{ParseResult, ...}; // REMOVE OR UPDATE COMMENT

#[cfg(test)]
mod tests {
    // Example test function
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}

// TODO: Add comprehensive tests for the parser modules. 