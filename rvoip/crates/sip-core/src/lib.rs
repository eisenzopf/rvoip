//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

mod error;
mod header;
mod header_parsers;
mod message;
mod method;
mod multipart;
mod parser;
mod uri;
mod version;

pub use error::{Error, Result};
pub use header::{Header, HeaderName, HeaderValue};
pub use header_parsers::{
    parse_auth_params, parse_contact, parse_address, parse_via, 
    parse_multiple_vias, parse_cseq, parse_content_type,
};
pub use message::{Message, Request, Response, StatusCode};
pub use method::Method;
pub use multipart::{MimePart, MultipartBody};
pub use parser::{
    parse_message, IncrementalParser, ParseState, MAX_LINE_LENGTH,
    MAX_HEADER_COUNT, MAX_BODY_SIZE,
};
pub use uri::{Host, Scheme, Uri};
pub use version::Version;

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{
        Error, Header, HeaderName, HeaderValue, Host, Message, Method, 
        Request, Response, Result, Scheme, StatusCode, Uri, Version, 
        parse_message, IncrementalParser, ParseState, MultipartBody,
        MimePart, MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE,
    };
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
} 