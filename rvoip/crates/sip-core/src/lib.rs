//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

// Re-export core types and parsers

// Declare modules
pub mod error;
pub mod header;
pub mod uri;
pub mod version;
pub mod parser;
pub mod types;
pub mod sdp;

// Re-export key public items
pub use error::{Error, Result};
pub use header::{Header, HeaderName, HeaderValue};
pub use uri::{Uri, Scheme, Host};
pub use version::Version;
pub use types::{
    Method, StatusCode, Request, Response, Message,
    Address, Param, Via, CSeq, From, To, Contact, Route, RecordRoute, ReplyTo,
    ContentType, MediaType, Accept, Allow, ContentDisposition, DispositionType,
    Warning, ContentLength, Expires, MaxForwards, CallId,
    auth,
    SdpSession, MediaDescription, ParsedAttribute,
};
pub use parser::prelude::*;

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