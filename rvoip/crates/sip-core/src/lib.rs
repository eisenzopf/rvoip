//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

mod error;
mod header;
mod message;
mod method;
mod parser;
mod uri;
mod version;

pub use error::{Error, Result};
pub use header::{Header, HeaderName, HeaderValue};
pub use message::{Message, Request, Response, StatusCode};
pub use method::Method;
pub use parser::{
    parse_message, parse_message_bytes, IncrementalParser, ParseState, 
    parse_via, parse_multiple_vias, Via, 
    parse_uri, parse_multipart, MimePart, MultipartBody,
    MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE,
};
pub use uri::{Uri, Scheme, Host};
pub use version::Version;

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{
        Error, Header, HeaderName, HeaderValue, Host, Message, Method, 
        Request, Response, Result, Scheme, StatusCode, Uri, Version, 
        Via, parse_message, parse_message_bytes, IncrementalParser, ParseState, MultipartBody,
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