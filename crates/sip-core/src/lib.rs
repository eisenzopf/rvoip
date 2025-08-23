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
//! let request = RequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .call_id("a84b4c76e66710@pc33.atlanta.com")
//!     .cseq(314159)
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .max_forwards(70)
//!     .contact("sip:alice@pc33.atlanta.com", None)
//!     .content_type("application/sdp")
//!     .body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n")
//!     .build();
//!
//! // Create a SIP response with the ResponseBuilder
//! let response = ResponseBuilder::new(StatusCode::Ok, Some("OK"))
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
//!     .call_id("a84b4c76e66710@pc33.atlanta.com")
//!     .cseq(1, Method::Invite)
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .contact("sip:bob@192.168.1.2", None)
//!     .content_type("application/sdp")
//!     .body("v=0\r\no=bob 123 456 IN IP4 192.168.1.2\r\ns=A call\r\nt=0 0\r\n")
//!     .build();
//! ```
//!
//! #### Using Macros (recommended for simple messages)
//!
//! ```ignore
//! use rvoip_sip_core::prelude::*;
//! use rvoip_sip_core::{sip_request, sip_response};
//!
//! // Create a SIP request with the sip_request! macro 
//! let request = sip_request! {
//!     method: Method::Invite,
//!     uri: "sip:bob@example.com",
//!     from_name: "Alice", 
//!     from_uri: "sip:alice@example.com",
//!     from_tag: "1928301774",
//!     call_id: "a84b4c76e66710",
//!     cseq: 1
//! };
//!
//! // Create a SIP response with the sip_response! macro
//! let response = sip_response! {
//!     status: StatusCode::Ok,
//!     reason: "OK",
//!     from_name: "Alice", 
//!     from_uri: "sip:alice@example.com", 
//!     to_name: "Bob", 
//!     to_uri: "sip:bob@example.com",
//!     call_id: "a84b4c76e66710",
//!     cseq: 1, 
//!     cseq_method: Method::Invite
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
pub mod types;
pub mod builder;
pub mod parser;
pub mod macros;
pub mod validation;
#[cfg(feature = "sdp")]
pub mod sdp;
/// JSON representation and access layer for SIP types
pub mod json;
/// Prelude module that exports commonly used types and traits
pub mod prelude;
/// SDP prelude module that exports SDP-related types and traits
#[cfg(feature = "sdp")]
pub mod sdp_prelude;

// Re-export key public items
pub use error::{Error, Result};
pub use types::header::{Header, HeaderValue, TypedHeader, TypedHeaderTrait};
pub use types::headers::HeaderName;
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
    Via,  // Changed from via::Via
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
#[cfg(feature = "sdp")]
pub use sdp::attributes::MediaDirection;
#[cfg(feature = "sdp")]
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
pub use builder::{SimpleRequestBuilder as RequestBuilder, SimpleResponseBuilder as ResponseBuilder};
pub use macros::*;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}