//! # rvoip-sip-core
//!
//! Core SIP protocol implementation for the rvoip VoIP stack.
//!
//! This crate provides a complete, RFC-compliant implementation of the Session Initiation Protocol (SIP),
//! including message parsing, serialization, and manipulation. It serves as the foundation for building
//! SIP-based communication systems like VoIP clients, proxies, and servers.
//!
//! ## Overview
//!
//! The crate is structured around the following key components:
//!
//! - **Message Types**: Core SIP message abstractions ([`Request`], [`Response`], [`Message`])
//! - **Header Types**: Strongly-typed SIP headers with parsing and serialization
//! - **URI Handling**: Comprehensive SIP URI parsing and manipulation
//! - **SDP Support**: Session Description Protocol integration
//! - **Parsing**: Robust, efficient, and RFC-compliant message parsing
//!
//! ## Getting Started
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use bytes::Bytes;
//!
//! // Parse a SIP message from bytes
//! let data = Bytes::from(
//!     "INVITE sip:bob@example.com SIP/2.0\r\n\
//!      Via: SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds\r\n\
//!      Max-Forwards: 70\r\n\
//!      To: Bob <sip:bob@example.com>\r\n\
//!      From: Alice <sip:alice@atlanta.com>;tag=1928301774\r\n\
//!      Call-ID: a84b4c76e66710@pc33.atlanta.com\r\n\
//!      CSeq: 314159 INVITE\r\n\
//!      Contact: <sip:alice@pc33.atlanta.com>\r\n\
//!      Content-Type: application/sdp\r\n\
//!      Content-Length: 0\r\n\r\n"
//! );
//!
//! let message = parse_message(&data).expect("Valid SIP message");
//!
//! // Access message components
//! if let Message::Request(request) = message {
//!     assert_eq!(request.method(), Method::Invite);
//!     assert_eq!(request.uri().to_string(), "sip:bob@example.com");
//!     
//!     // Get headers
//!     let from = request.typed_header::<From>().expect("From header");
//!     let to = request.typed_header::<To>().expect("To header");
//!     
//!     println!("From: {}", from.address().display_name().unwrap_or(""));
//!     println!("To: {}", to.address().display_name().unwrap_or(""));
//! }
//!
//! // Create a SIP request using the builder pattern
//! let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
//!     .with_header(From::new(Address::parse("Alice <sip:alice@atlanta.com>").unwrap()))
//!     .with_header(To::new(Address::parse("Bob <sip:bob@example.com>").unwrap()))
//!     .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
//!     .with_header(CSeq::new(314159, Method::Invite))
//!     .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
//!     .with_header(MaxForwards::new(70))
//!     .with_header(Contact::new(Address::parse("<sip:alice@pc33.atlanta.com>").unwrap()))
//!     .with_header(ContentLength::new(0))
//!     .build();
//!
//! // Create a SIP response
//! let response = ResponseBuilder::new(StatusCode::Ok)
//!     .with_header(From::new(Address::parse("Alice <sip:alice@atlanta.com>").unwrap()))
//!     .with_header(To::new(Address::parse("Bob <sip:bob@example.com>").unwrap()))
//!     .with_header(CallId::new("a84b4c76e66710@pc33.atlanta.com"))
//!     .with_header(CSeq::new(314159, Method::Invite))
//!     .with_header(Via::parse("SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds").unwrap())
//!     .with_header(ContentLength::new(0))
//!     .build();
//! ```
//!
//! ## Parsing Modes
//!
//! The library supports different parsing modes to handle various levels of RFC compliance:
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use bytes::Bytes;
//!
//! let data = Bytes::from("SIP message data...");
//!
//! // Standard parsing mode
//! let message = parse_message(&data);
//!
//! // Custom parsing mode
//! let strict_message = parse_message_with_mode(&data, ParseMode {
//!     max_line_length: 8192,
//!     max_header_count: 100,
//!     max_body_size: 16384,
//!     strict: true,
//! });
//! ```
//!
//! ## Feature Flags
//!
//! - `lenient_parsing` - Enables more lenient parsing mode for torture tests and handling of non-compliant messages

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
pub use parser::message::parse_message_with_mode;
pub use parser::message::ParseMode;
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
    pub use crate::parser::message::{MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE, ParseMode};
    pub use crate::parser::parse_message;
    pub use crate::parser::message::parse_message_with_mode;
    pub use crate::types::multipart::{MultipartBody, MimePart};
    pub use crate::types::builder::{RequestBuilder, ResponseBuilder};
    pub use crate::sip_request;
    pub use crate::sip_response;
    
    // Add missing types needed for doc tests
    pub use crate::types::param::Param;
    pub use crate::types::warning::Warning;
    pub use crate::types::address::Address;
    pub use crate::types::from::From;
    pub use crate::types::to::To;
    pub use crate::types::call_id::CallId;
    pub use crate::types::cseq::CSeq;
    pub use crate::types::content_length::ContentLength;
    pub use crate::types::max_forwards::MaxForwards;
    pub use crate::types::contact::Contact;
    pub use crate::types::supported::Supported;
    pub use crate::types::unsupported::Unsupported;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
} 