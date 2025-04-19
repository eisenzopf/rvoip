//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

// Core parsing modules
mod common;
pub mod headers; 
pub mod message; // Make public if needed for IncrementalParser, ParseState
pub mod multipart;
pub mod request;
pub mod response;
pub mod uri; 
pub mod utils;

// --- Remove ALL re-exports --- 
// Other modules must use full paths, e.g.:
// crate::parser::message::parse_message
// crate::parser::headers::parse_via

// Re-export necessary top-level parsers
pub use message::parse_message;
pub use request::request_parser;
pub use response::response_parser;
pub use uri::parse_uri;
pub use multipart::parse_multipart;

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
    pub use super::uri::parse_uri;
    pub use super::multipart::{parse_multipart, MimePart, MultipartBody};

    // Add new exports as they are created
    // pub use super::parse_message;
    // pub use super::request::parse_request;
    // pub use super::response::parse_response;
}

// Export necessary parsers
pub mod common;
pub mod request;
pub mod response;
pub mod message;
pub mod uri;
pub mod utils;
pub mod multipart;
pub mod headers; // Ensure this is public

// Re-export top-level parsers
pub use message::parse_message;
pub use request::request_parser;
pub use response::response_parser;
pub use uri::{parse_uri, parse_uri_params, parse_host_port};

// Re-export specific header parsers needed by types/header.rs
pub use headers::{
    parse_via,
    parse_address,
    parse_contact,
    parse_cseq,
    parse_content_type,
    parse_allow,
    parse_accept,
    parse_content_disposition,
    parse_warning,
    parse_call_id,
    parse_content_length,
    parse_expires,
    parse_max_forwards,
    parse_www_authenticate,
    parse_authorization,
    parse_proxy_authenticate,
    parse_proxy_authorization,
    parse_authentication_info,
    parse_route,
    parse_record_route,
    parse_reply_to,
};

// Maybe re-export specific header parsers if needed directly?
// pub use headers::{parse_via, parse_cseq, ...}; 