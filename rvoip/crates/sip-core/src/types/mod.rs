//! # SIP Protocol Types
//!
//! This module provides all the core type definitions for the SIP protocol.
//!
//! ## Module Structure
//!
//! The types are organized into the following categories:
//!
//! ### Core Message Types
//!
//! - [`Message`] - The main SIP message abstraction
//! - [`Request`] - SIP request message implementation
//! - [`Response`] - SIP response message implementation
//!
//! ### URI Components
//!
//! - [`Uri`] - SIP URI implementation (e.g., "sip:user@example.com")
//! - [`Host`] - Host part of a URI (hostname, IPv4, or IPv6 address)
//! - [`Scheme`] - URI scheme (sip, sips, tel, etc.)
//!
//! ### Protocol Elements
//!
//! - [`Method`] - SIP methods (INVITE, ACK, BYE, etc.)
//! - [`StatusCode`] - SIP response status codes (200 OK, 404 Not Found, etc.)
//! - [`Version`] - SIP protocol version
//! - [`Via`] - Via header implementation for routing
//!
//! ### Header Types
//!
//! This module provides typed implementations for all standard SIP headers.
//! Each header type implements the [`TypedHeaderTrait`] trait, which provides
//! methods for parsing, serializing, and accessing header data.
//!
//! Common headers include:
//!
//! - [`From`] - Sender's address
//! - [`To`] - Recipient's address
//! - [`CallId`] - Unique identifier for a call
//! - [`CSeq`] - Command sequence number and method
//! - [`Via`] - Routing information
//! - [`Contact`] - Where the sender can be contacted directly
//!
//! ### Builders
//!
//! - [`RequestBuilder`] - Fluent builder for creating SIP requests
//! - [`ResponseBuilder`] - Fluent builder for creating SIP responses
//!
//! ## Usage Examples
//!
//! ### Working with SIP Headers
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a From header directly without parsing
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let mut address = Address::new_with_display_name("Alice", uri);
//! address.set_tag("1234");
//! let from = From::new(address);
//! 
//! // Access the address data
//! let addr = from.address();
//! assert_eq!(addr.display_name(), Some("Alice"));
//! 
//! // Access URI data
//! assert_eq!(addr.uri.scheme, Scheme::Sip);
//! assert_eq!(addr.uri.user, Some("alice".to_string()));
//! 
//! // Access parameters
//! assert_eq!(from.tag(), Some("1234"));
//! ```
//!
//! ### Using the Builder Pattern
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use std::str::FromStr;
//!
//! // Create a SIP request
//! let uri = "sip:bob@example.com".parse::<Uri>().unwrap();
//! let request = RequestBuilder::new(Method::Invite, &uri.to_string())
//!     .unwrap()
//!     .header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", "sip:alice@example.com".parse::<Uri>().unwrap()))))
//!     .header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", uri))))
//!     .header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
//!     .header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
//!     .build();
//! 
//! assert_eq!(request.method(), Method::Invite);
//! ```

use std::str::FromStr;

pub mod via;
pub use via::Via;

pub mod method;
pub use method::Method;

pub mod status;
pub use status::StatusCode;

pub mod sip_message;
pub use sip_message::{Request, Response, Message};

pub mod param;
pub use param::Param;

// Add new URI module
pub mod uri;
pub use uri::{Uri, Host, Scheme};

// Add URI adapter module - REMOVED
// pub mod uri_adapter;
// pub use uri_adapter::UriAdapter;

// Add Version module
pub mod version;
pub use version::Version;

// SIP Core Types

pub mod accept;
pub mod accept_language;
pub mod address;
pub mod allow;
pub mod auth;
pub mod call_id;
pub mod call_info;
pub mod contact;
pub mod content_disposition;
pub mod content_length;
pub mod content_type;
pub mod media_type;
pub mod cseq;
pub mod expires;
pub mod from;
pub mod header;
pub use header::{Header, HeaderName, HeaderValue, TypedHeader, TypedHeaderTrait};
pub mod in_reply_to;
pub mod max_forwards;
pub mod organization;
pub mod record_route;
pub mod reply_to;
pub mod refer_to;
pub mod require;
pub mod route;
pub mod sdp;
pub mod subject;
pub mod to;
pub mod warning;
pub mod multipart;

// Modules missing re-exports - Add them
pub mod priority;
pub mod server;
pub mod retry_after;
pub mod error_info;
pub mod supported;
pub mod unsupported;

pub mod builder;

pub use accept::Accept;
pub use accept_language::AcceptLanguage;
pub use address::Address;
pub use allow::Allow;
pub use auth::*;
pub use call_id::CallId;
pub use call_info::{CallInfo, CallInfoValue, InfoPurpose};
pub use contact::Contact;
pub use content_disposition::*;
pub use content_length::ContentLength;
pub use content_type::ContentType;
pub use media_type::MediaType;
pub use cseq::CSeq;
pub use expires::Expires;
pub use from::From;
pub use in_reply_to::InReplyTo;
pub use max_forwards::MaxForwards;
pub use organization::Organization;
pub use require::Require;
pub use subject::Subject;
pub use warning::Warning;
pub use multipart::{MultipartBody, MimePart, ParsedBody};
pub use refer_to::ReferTo;

// Add missing pub use * directives
pub use priority::*;
pub use server::*;
pub use retry_after::*;
pub use error_info::*;
pub use supported::Supported;
pub use unsupported::Unsupported;

// Add AsRef implementations for Message
impl AsRef<Message> for Message {
    fn as_ref(&self) -> &Message {
        self
    }
}

// Fix Request and Response implementations to use a different approach
// Rather than implementing AsRef<Message>, let's modify the ContentLength and MaxForwards methods
// to accept both the specific types and the generic Message type
// Removing these problematic implementations
// impl AsRef<Message> for Request {
//     fn as_ref(&self) -> &Message {
//         &Message::Request(self.clone())
//     }
// }
// 
// impl AsRef<Message> for Response {
//     fn as_ref(&self) -> &Message {
//         &Message::Response(self.clone())
//     }
// }