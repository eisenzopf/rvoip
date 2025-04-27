//! Core SIP protocol implementation for rvoip
//!
//! This crate provides the fundamental SIP message types, parsing,
//! and serialization functionality for the rvoip stack.

// Re-export core types and parsers

// Declare modules
pub mod error;
pub mod parser;
pub mod types;
pub mod sdp;
pub mod macros;

// Remove these commented out modules - they're now part of types/
// // pub mod header;
// // pub mod method;
// // pub mod status;
// // pub mod uri;
// // pub mod version;

// Re-export key public items
pub use error::{Error, Result};
pub use types::header::{Header, HeaderName, HeaderValue};
pub use types::Method;
pub use parser::parse_message;
pub use types::StatusCode;
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
    TypedHeaderTrait,
    via::Via,
    Warning,
    sdp::MediaDescription, 
    sdp::Origin,
    sdp::ConnectionData, 
    sdp::TimeDescription,
    auth::*,
    sdp::ParsedAttribute,
    Version,
};
pub use types::uri::{Uri, Host, Scheme}; // Updated path
pub use sdp::attributes::MediaDirection;
pub use types::builder::{RequestBuilder, ResponseBuilder};
pub use macros::*;

/// Re-export of common types and functions
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait}; // Updated path
    pub use crate::types::uri::{Uri, Host, Scheme}; // Updated path
    pub use crate::types::Method;
    pub use crate::types::StatusCode;
    pub use crate::types::sip_message::{Request, Response, Message};
    pub use crate::types::via::Via;
    pub use crate::types::Version; // Added Version
    pub use crate::parser::message::{MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE};
    pub use crate::parser::parse_message;
    pub use crate::types::multipart::{MultipartBody, MimePart};
    pub use crate::types::builder::{RequestBuilder, ResponseBuilder};
    pub use crate::sip_request;
    pub use crate::sip_response;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
} 