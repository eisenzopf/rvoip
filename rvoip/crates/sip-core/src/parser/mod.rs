//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

mod message;
mod headers;
mod via;
mod uri_parser;
mod multipart;
mod utils;

pub use message::{
    parse_message, parse_message_bytes, IncrementalParser, ParseState, 
    MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE,
};
pub use headers::{
    parse_header, parse_headers, parse_auth_params, parse_contact, parse_address,
    parse_cseq, parse_content_type,
};
pub use via::{Via, parse_via, parse_multiple_vias};
pub use uri_parser::parse_uri;
pub use multipart::{parse_multipart, MimePart, MultipartBody};

// Re-export common parsing functions
pub mod prelude {
    pub use super::{
        parse_message, parse_header, parse_headers, parse_via, parse_multiple_vias,
        parse_uri, parse_multipart, IncrementalParser, ParseState,
        Via, MimePart, MultipartBody,
    };
} 