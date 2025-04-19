//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

// Re-export core types and parsers

// Declare modules
pub mod error;
pub mod header;
pub mod method;
pub mod parser;
pub mod status;
pub mod types;
pub mod uri;
pub mod sdp;
pub mod version;

// Re-export key public items
pub use error::{Error, Result};
pub use header::{Header, HeaderName, HeaderValue};
pub use crate::types::Method;
pub use parser::{parse_message, IncrementalParser, ParseState};
pub use crate::types::StatusCode;
pub use types::{
    Address, 
    CallId, 
    Contact, 
    ContentDisposition, 
    ContentLength, 
    ContentType, 
    CSeq, 
    Expires, 
    From, 
    MaxForwards, 
    MediaType, 
    sip_message::Message,
    sip_message::Request,
    sip_message::Response,
    sdp::SdpSession,
    TypedHeader, 
    via::Via,
    Warning,
    sdp::MediaDescription, 
    sdp::Origin,
    sdp::ConnectionData, 
    sdp::TimeDescription,
    auth::*,
    sdp::ParsedAttribute,
};
pub use uri::Uri;
pub use sdp::attributes::MediaDirection;

/// Re-export of common types and functions
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::header::{Header, HeaderName, HeaderValue};
    pub use crate::uri::{Uri, Host, Scheme};
    pub use crate::types::Method;
    pub use crate::types::StatusCode;
    pub use crate::types::sip_message::{Request, Response, Message};
    pub use crate::types::via::Via;
    pub use crate::parser::message::{MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE};
    pub use crate::parser::{parse_message, IncrementalParser, ParseState};
    pub use crate::parser::multipart::{MultipartBody, MimePart};
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
} 