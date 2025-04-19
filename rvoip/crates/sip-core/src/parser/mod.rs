//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

// Core parsing modules
mod headers;
mod message; // Keep for common utilities, incremental parser?
mod multipart;
mod request;
mod response;
mod uri; // Renamed from uri_parser
mod utils;

// Re-export key parsing functions and types
pub use message::{
    // parse_message, // Will be moved here or called from here
    // parse_message_bytes, // Will be moved here or called from here
    IncrementalParser,
    ParseState,
    MAX_LINE_LENGTH,
    MAX_HEADER_COUNT,
    MAX_BODY_SIZE,
};
pub use headers::{
    parse_header, // Keep generic header parser
    parse_headers, // Keep generic headers parser
                  // Specific header parsers might be moved internal to headers.rs or called differently
                  // parse_auth_params, parse_contact, parse_address,
                  // parse_cseq, parse_content_type,
};
// pub use via::{Via, parse_via, parse_multiple_vias}; // Via struct moved to types, parser moved to headers.rs
pub use uri::parse_uri; // Keep URI parser export
pub use multipart::{parse_multipart, MimePart, MultipartBody}; // Keep multipart exports

// Specific request/response parsers (to be added)
// pub use request::parse_request;
// pub use response::parse_response;

// Re-export common parsing functions for convenience
pub mod prelude {
    pub use super::message::{
        IncrementalParser, ParseState, MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE,
    };
    pub use super::headers::{parse_header, parse_headers};
    // pub use super::via::Via; // Via struct will be in types module
    pub use super::uri::parse_uri;
    pub use super::multipart::{parse_multipart, MimePart, MultipartBody};

    // Add new exports as they are created
    // pub use super::parse_message;
    // pub use super::request::parse_request;
    // pub use super::response::parse_response;
} 