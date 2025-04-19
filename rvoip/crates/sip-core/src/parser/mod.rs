//! SIP protocol parser implementation
//!
//! This module contains parsers for SIP messages, headers, and related structures.
//! All parsers use the nom parser combinator library.

// Core parsing modules
mod common;
pub mod headers;
mod message;
mod multipart;
mod request;
mod response;
pub mod uri;
mod utils;

// Re-export top-level parsers and types, consolidate duplicates
pub use message::{parse_message, IncrementalParser, ParseState};
pub use request::request_parser;
pub use response::response_parser;
// Commenting out potentially unresolved imports
pub use uri::{parse_uri /*, parse_uri_params, parse_host_port*/ };
pub use multipart::{parse_multipart, MimePart, MultipartBody};

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