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
//! - **Builder Patterns**: Fluent APIs for constructing SIP and SDP messages
//! - **Macros**: Convenient macros for creating SIP requests and responses
//!
//! ## Getting Started
//!
//! ### Creating SIP Messages
//!
//! The recommended way to create SIP messages is to use either the builder pattern or macros:
//!
//! #### Using the Builder Pattern (recommended for complex messages)
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a SIP request with the RequestBuilder
//! let bob_uri = "sip:bob@example.com".parse::<Uri>().unwrap();
//! let alice_uri = "sip:alice@atlanta.com".parse::<Uri>().unwrap();
//! let contact_uri = "sip:alice@pc33.atlanta.com".parse::<Uri>().unwrap();
//! 
//! let request = RequestBuilder::new(Method::Invite, &bob_uri.to_string())
//!     .unwrap()
//!     .header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", alice_uri.clone()))))
//!     .header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", bob_uri.clone()))))
//!     .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
//!     .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
//!     .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "pc33.atlanta.com", None, vec![Param::branch("z9hG4bK776asdhds")]).unwrap()))
//!     .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
//!     .header(TypedHeader::Contact(Contact::new_params(vec![ContactParamInfo { 
//!         address: Address::new(contact_uri)
//!     }])))
//!     .header(TypedHeader::ContentLength(ContentLength::new(0)))
//!     .build();
//!
//! // Create a SIP response with the ResponseBuilder
//! let response = ResponseBuilder::new(StatusCode::Ok)
//!     .header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", alice_uri))))
//!     .header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", bob_uri))))
//!     .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
//!     .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
//!     .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "pc33.atlanta.com", None, vec![Param::branch("z9hG4bK776asdhds")]).unwrap()))
//!     .header(TypedHeader::ContentLength(ContentLength::new(0)))
//!     .build();
//! ```
//!
//! #### Using Macros (recommended for simple messages)
//!
//! ```no_run
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::{sip_request, sip_response};
//!
//! // Create a SIP request with the sip_request! macro
//! let request = sip_request! {
//!     method: Method::Invite,
//!     uri: "sip:bob@example.com",
//!     headers: {
//!         From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
//!         To: "Bob <sip:bob@example.com>",
//!         CallId: "a84b4c76e66710@pc33.atlanta.com",
//!         CSeq: "314159 INVITE",
//!         Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
//!         MaxForwards: "70",
//!         Contact: "<sip:alice@pc33.atlanta.com>",
//!         ContentLength: "0"
//!     }
//! };
//!
//! // Create a SIP response with the sip_response! macro
//! let response = sip_response! {
//!     status: StatusCode::Ok,
//!     headers: {
//!         From: "Alice <sip:alice@atlanta.com>;tag=1928301774",
//!         To: "Bob <sip:bob@example.com>",
//!         CallId: "a84b4c76e66710@pc33.atlanta.com",
//!         CSeq: "314159 INVITE",
//!         Via: "SIP/2.0/UDP pc33.atlanta.com;branch=z9hG4bK776asdhds",
//!         ContentLength: "0"
//!     }
//! };
//! ```
//!
//! ### Creating SDP Messages
//!
//! For SDP messages, you can use either the SdpBuilder (for programmatic creation) or the sdp! macro (for declarative creation):
//!
//! #### Using the SdpBuilder Pattern
//!
//! ```rust
//! use rvoip_sip_core::sdp_prelude::*;
//!
//! // Create an SDP session with the SdpBuilder
//! let sdp = SdpBuilder::new("My Session")
//!     .origin("-", "1234567890", "2", "IN", "IP4", "127.0.0.1")
//!     .time("0", "0")  // Time 0-0 means permanent session
//!     .media_audio(49170, "RTP/AVP")
//!         .formats(&["0", "8"])
//!         .direction(MediaDirection::SendRecv)
//!         .rtpmap("0", "PCMU/8000")
//!         .rtpmap("8", "PCMA/8000")
//!         .done()
//!     .build();
//! ```
//!
//! #### Using the sdp! Macro (recommended for simple messages)
//!
//! ```rust
//! use rvoip_sip_core::sdp;
//! use rvoip_sip_core::sdp_prelude::*;
//!
//! // Create an SDP session with the sdp! macro
//! let sdp_result = sdp! {
//!     origin: ("-", "1234567890", "2", "IN", "IP4", "192.168.1.100"),
//!     session_name: "Audio Call",
//!     connection: ("IN", "IP4", "192.168.1.100"),
//!     time: ("0", "0"),
//!     media: {
//!         type: "audio",
//!         port: 49170,
//!         protocol: "RTP/AVP",
//!         formats: ["0", "8"],
//!         rtpmap: ("0", "PCMU/8000"),
//!         rtpmap: ("8", "PCMA/8000"),
//!         direction: "sendrecv"
//!     }
//! };
//!
//! let sdp = sdp_result.expect("Valid SDP");
//! ```
//!
//! ### Parsing SIP Messages
//!
//! The library provides robust parsing for SIP messages:
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
//!     // Get headers and display them
//!     if let Some(from_header) = request.header(&HeaderName::From) {
//!         println!("From: {}", from_header);
//!     }
//!     if let Some(to_header) = request.header(&HeaderName::To) {
//!         println!("To: {}", to_header);
//!     }
//! }
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
//! let strict_message = parse_message_with_mode(&data, ParseMode::Strict);
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
pub mod builder;
pub mod simple_builder;

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
    sip_request::Request,
    sip_response::Response,
    sdp::SdpSession,
    TypedHeader, 
    TypedHeaderTrait,
    via::Via,
    Warning,
    warning::{WarnAgent, WarningValue},
    sdp::MediaDescription,
    sdp::Origin,
    sdp::ConnectionData,
    sdp::TimeDescription,
    auth::*,
    sdp::ParsedAttribute,
    sdp::RtpMapAttribute,
    sdp::FmtpAttribute,
    sdp::CandidateAttribute,
    sdp::SsrcAttribute,
    sdp::RepeatTime,
    Version,
    Allow,
    Accept,
    Subject,
    CallInfo,
};
pub use types::uri::{Uri, Host};
pub use sdp::attributes::MediaDirection;
pub use sdp::parser::{
    validate_sdp,
    validate_network_type,
    validate_address_type,
    is_valid_address,
    is_valid_ipv4,
    is_valid_ipv6,
    is_valid_hostname,
    parse_bandwidth_line,
    parse_sdp
};
pub use builder::{RequestBuilder, ResponseBuilder};
pub use sdp::builder::SdpBuilder;
pub use macros::*;

/// Re-export of common types and functions for SIP
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::types::header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait}; // Updated path
    pub use crate::types::uri::{Uri, Host, Scheme}; // Use the original URI Scheme
    pub use crate::types::Method;
    pub use crate::types::StatusCode;
    pub use crate::types::sip_message::Message;
    pub use crate::types::sip_request::Request;
    pub use crate::types::sip_response::Response;
    pub use crate::types::via::Via;
    pub use crate::types::Version; // Added Version
    pub use crate::parser::message::{MAX_LINE_LENGTH, MAX_HEADER_COUNT, MAX_BODY_SIZE, ParseMode};
    pub use crate::parser::parse_message;
    pub use crate::parser::message::parse_message_with_mode;
    pub use crate::types::multipart::{MultipartBody, MimePart, ParsedBody}; // Add multipart types
    pub use crate::builder::{RequestBuilder, ResponseBuilder};
    pub use crate::types::param::Param;
    pub use crate::types::param::GenericValue;
    pub use crate::types::warning::Warning;
    pub use crate::types::warning::{WarnAgent, WarningValue};
    pub use crate::types::address::Address;
    pub use crate::types::from::From;
    pub use crate::types::to::To;
    pub use crate::types::call_id::CallId;
    pub use crate::types::cseq::CSeq;
    pub use crate::types::content_length::ContentLength;
    pub use crate::types::content_type::ContentType;
    pub use crate::parser::headers::content_type::ContentTypeValue;
    pub use crate::types::max_forwards::MaxForwards;
    pub use crate::types::contact::Contact;
    pub use crate::types::contact::{ContactParamInfo, ContactValue}; // Add Contact-related types
    pub use crate::types::supported::Supported;
    pub use crate::types::unsupported::Unsupported;
    pub use crate::types::content_disposition::{ContentDisposition, DispositionType, DispositionParam, Handling}; // Add ContentDisposition-related types
    pub use crate::types::error_info::{ErrorInfo, ErrorInfoHeader, ErrorInfoList}; // Add Error-Info related types
    pub use crate::types::expires::Expires; // Add Expires type
    pub use crate::types::in_reply_to::InReplyTo; // Add In-Reply-To type
    pub use crate::types::MediaType; // Add MediaType for MIME content types
    pub use crate::types::organization::Organization; // Add Organization type
    pub use crate::types::priority::Priority; // Add Priority type
    pub use crate::types::record_route::RecordRoute; // Add RecordRoute type
    pub use crate::types::record_route::RecordRouteEntry; // Add RecordRouteEntry type
    pub use crate::types::refer_to::ReferTo; // Add ReferTo type for doctests
    pub use crate::types::reply_to::ReplyTo; // Add ReplyTo type for doctests
    pub use crate::types::require::Require; // Add Require type for doctests
    pub use crate::types::retry_after::RetryAfter; // Add RetryAfter type for doctests
    pub use crate::parser::headers::route::RouteEntry as ParserRouteValue; // Add ParserRouteValue for doctests
    pub use crate::types::route::Route; // Add Route type for doctests
    
    // Server-related types needed for doc tests
    pub use crate::types::server::{ServerInfo, ServerProduct, Product, ServerVal};
    
    // Authentication-related types needed for doc tests
    pub use crate::types::auth::{
        AuthParam, AuthenticationInfo, AuthenticationInfoParam, Authorization,
        Challenge, Credentials, DigestParam, ProxyAuthenticate, ProxyAuthorization,
        WwwAuthenticate, AuthScheme
    };
    pub use crate::types::{Algorithm, Qop};
    
    // Additional header types previously missing
    pub use crate::types::Allow;
    pub use crate::types::Accept;
    pub use crate::parser::headers::accept::AcceptValue;
    pub use crate::types::Subject;
    pub use crate::types::call_info::{CallInfo, CallInfoValue, InfoPurpose};
    pub use crate::types::AcceptLanguage;
    pub use crate::parser::headers::accept_language::LanguageInfo;
    pub use crate::simple_builder::{SimpleRequestBuilder, SimpleResponseBuilder};
}

/// Re-export of common types and functions for SDP
pub mod sdp_prelude {
    pub use crate::types::sdp::{SdpSession, Origin, ConnectionData, TimeDescription, MediaDescription};
    pub use crate::types::sdp::{ParsedAttribute, RtpMapAttribute, FmtpAttribute, CandidateAttribute, SsrcAttribute, RepeatTime};
    pub use crate::sdp::attributes::MediaDirection;
    pub use crate::sdp::attributes::rid::{RidAttribute, RidDirection};
    pub use crate::sdp::parser::{
        validate_sdp,
        validate_network_type,
        validate_address_type,
        is_valid_address,
        is_valid_ipv4,
        is_valid_ipv6,
        is_valid_hostname,
        parse_bandwidth_line,
        parse_sdp
    };
    pub use crate::sdp::builder::SdpBuilder;
    pub use crate::sdp;  // For the macro
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}