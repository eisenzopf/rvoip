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
//! - [`Param`] - Parameter implementation for headers and URIs
//!
//! ### Header Types
//!
//! This module provides typed implementations for all standard SIP headers.
//! Each header type implements the [`TypedHeaderTrait`] trait, which provides
//! methods for parsing, serializing, and accessing header data.
//!
//! #### Core Dialog Headers
//!
//! - [`from::From`] - Sender's address (RFC 3261)
//! - [`to::To`] - Recipient's address (RFC 3261)
//! - [`call_id::CallId`] - Unique identifier for a call (RFC 3261)
//! - [`cseq::CSeq`] - Command sequence number and method (RFC 3261)
//! - [`via::Via`] - Routing information (RFC 3261)
//! - [`contact::Contact`] - Where the sender can be contacted directly (RFC 3261)
//! - [`max_forwards::MaxForwards`] - Limits the number of hops a request can take (RFC 3261)
//!
//! #### Addressing and Routing Headers
//!
//! - [`route::Route`] - Routing information for proxies (RFC 3261)
//! - [`record_route::RecordRoute`] - Used by proxies to remain in the message path (RFC 3261)
//! - [`refer_to::ReferTo`] - Target URI for a REFER request (RFC 3515)
//! - [`reply_to::ReplyTo`] - Address for replies (RFC 3261)
//!
//! #### Session and Content Headers
//!
//! - [`accept::Accept`] - Acceptable media types for the response (RFC 3261)
//! - [`accept_language::AcceptLanguage`] - Acceptable languages for reason phrases, session descriptions (RFC 3261)
//! - [`allow::Allow`] - Lists the set of methods supported by the UA (RFC 3261)
//! - [`content_disposition::ContentDisposition`] - How to interpret the message body (RFC 3261)
//! - [`content_length::ContentLength`] - Size of the message body in bytes (RFC 3261)
//! - [`content_type::ContentType`] - MIME type of the message body (RFC 3261)
//! - [`media_type::MediaType`] - Media type and subtype (RFC 3261)
//! - [`expires::Expires`] - Expiration time for the message or content (RFC 3261)
//! - [`subject::Subject`] - Summary or nature of the call (RFC 3261)
//! - [`organization::Organization`] - Organization name of the originator (RFC 3261)
//! - [`in_reply_to::InReplyTo`] - Identifies a call that this call references (RFC 3261)
//!
//! #### Extension and Feature Headers
//!
//! - [`require::Require`] - Lists features that must be supported (RFC 3261)
//! - [`supported::Supported`] - Lists features that are supported (RFC 3261)
//! - [`unsupported::Unsupported`] - Lists features that are not supported (RFC 3261)
//! - [`call_info::CallInfo`] - Additional information about the call (RFC 3261)
//! - [`error_info::ErrorInfo`] - Additional information about an error (RFC 3261)
//! - [`priority::Priority`] - Urgency of the request (RFC 3261)
//! - [`retry_after::RetryAfter`] - When a service will be available again (RFC 3261)
//!
//! #### Security and Authentication Headers
//!
//! - [`auth::Authorization`] - Authentication credentials for a request (RFC 3261)
//! - [`auth::WWWAuthenticate`] - Authentication challenge (RFC 3261)
//! - [`auth::ProxyAuthenticate`] - Authentication challenge from a proxy (RFC 3261)
//! - [`auth::ProxyAuthorization`] - Authentication credentials for a proxy (RFC 3261)
//!
//! #### Miscellaneous Headers
//!
//! - [`server::Server`] - Information about the software used by the server (RFC 3261)
//! - [`warning::Warning`] - Additional information about the status of a response (RFC 3261)
//! - [`multipart::MultipartBody`] - Support for multipart message bodies (RFC 5621)
//! - [`multipart::MimePart`] - Individual part of a multipart message (RFC 5621)
//!
//! ### Data Types and Utilities
//!
//! - [`address::Address`] - SIP address with display name and URI
//! - [`header::Header`] - Generic header representation
//! - [`header::HeaderName`] - Header name enumeration
//! - [`header::HeaderValue`] - Header value representation
//! - [`header::TypedHeader`] - Typed header enumeration
//! - [`header::TypedHeaderTrait`] - Trait for implementing typed headers
//! - [`multipart::ParsedBody`] - Parsed message body
//!
//! ## Usage Examples
//!
//! ### Creating and Working with SIP URIs
//!
//! ```rust
//! use rvoip_sip_core::types::{Uri, Scheme, Host, Param};
//! use std::str::FromStr;
//!
//! // Parse a SIP URI from string
//! let uri = Uri::from_str("sip:alice@example.com:5060;transport=tcp").unwrap();
//! assert_eq!(uri.scheme, Scheme::Sip);
//! assert_eq!(uri.user, Some("alice".to_string()));
//! assert_eq!(uri.host.to_string(), "example.com");
//! assert_eq!(uri.port, Some(5060));
//! 
//! // Get parameters from the URI
//! let transport_param = uri.parameters.iter()
//!     .find(|p| p.key() == "transport")
//!     .and_then(|p| p.value())
//!     .map(|s| s.clone());
//! assert!(transport_param.is_some());
//! assert_eq!(transport_param.unwrap(), "tcp".to_string());
//!
//! // Build a SIP URI programmatically
//! let uri = Uri {
//!     scheme: Scheme::Sips,
//!     user: Some("bob".to_string()),
//!     password: None,
//!     host: Host::from_str("biloxi.example.org").unwrap(),
//!     port: Some(5061),
//!     parameters: vec![],
//!     headers: std::collections::HashMap::new(),
//!     raw_uri: None,
//! };
//! assert_eq!(uri.to_string(), "sips:bob@biloxi.example.org:5061");
//! ```
//!
//! ### Working with SIP Headers
//!
//! ```rust
//! use rvoip_sip_core::types::{Uri, Address, Param};
//! use rvoip_sip_core::types::from::From;
//! use std::str::FromStr;
//!
//! // Create a From header with display name and tag
//! let uri = Uri::from_str("sip:alice@example.com").unwrap();
//! let mut address = Address::new_with_display_name("Alice Smith", uri);
//! address.params.push(Param::tag("1234abcd"));
//! let from = From::new(address);
//! 
//! // Access the address components
//! let addr = from.address();
//! assert_eq!(addr.display_name(), Some("Alice Smith"));
//! assert_eq!(addr.uri.to_string(), "sip:alice@example.com");
//! 
//! // Access the tag parameter
//! assert_eq!(from.tag(), Some("1234abcd"));
//! 
//! // Serialize the header to a string
//! let header_str = from.to_string();
//! assert!(header_str.contains("Alice Smith"));
//! assert!(header_str.contains("sip:alice@example.com"));
//! assert!(header_str.contains("tag=1234abcd"));
//! ```
//!
//! ### Building SIP Request Messages
//!
//! ```rust
//! use rvoip_sip_core::types::Method;
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//!
//! // Create a SIP INVITE request using the builder pattern
//! let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", None)
//!     .call_id("a84b4c76e66710@pc33.atlanta.com")
//!     .cseq(314159)
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .max_forwards(70)
//!     .build();
//! 
//! assert_eq!(request.method(), Method::Invite);
//! assert_eq!(request.uri().to_string(), "sip:bob@example.com");
//! 
//! // Access specific headers
//! let from = request.from().unwrap();
//! assert_eq!(from.address().display_name(), Some("Alice"));
//! assert_eq!(from.tag(), Some("1928301774"));
//! ```
//!
//! ### Building SIP Response Messages
//!
//! ```rust
//! use rvoip_sip_core::types::{StatusCode, Method};
//! use rvoip_sip_core::builder::SimpleResponseBuilder;
//!
//! // Create a 200 OK response using the builder pattern
//! let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
//!     .from("Alice", "sip:alice@example.com", Some("1928301774"))
//!     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
//!     .call_id("a84b4c76e66710@pc33.atlanta.com")
//!     .cseq(314159, Method::Invite)
//!     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
//!     .build();
//! 
//! assert_eq!(response.status_code(), 200);
//! assert_eq!(response.reason_phrase(), "OK");
//! 
//! // Access specific headers
//! let to = response.to().unwrap();
//! assert_eq!(to.address().display_name(), Some("Bob"));
//! assert_eq!(to.tag(), Some("a6c85cf"));
//! ```
//!
//! ### Working with Authentication
//!
//! ```rust
//! use rvoip_sip_core::types::auth::{Authorization, AuthScheme};
//! use rvoip_sip_core::types::Uri;
//! use std::str::FromStr;
//!
//! // Create Authorization header with Digest credentials
//! let uri = Uri::from_str("sip:bob@example.com").unwrap();
//! let auth = Authorization::new(
//!     AuthScheme::Digest,
//!     "alice", 
//!     "example.com",
//!     "dcd98b7102dd2f0e8b11d0f600bfb0c093", 
//!     uri,
//!     "a2ea68c230e5fea1ca715740fb14db97"
//! );
//!
//! // Verify the header was created correctly
//! let auth_str = auth.to_string();
//! assert!(auth_str.contains("Digest"));
//! assert!(auth_str.contains("username=\"alice\""));
//! ```
//!
//! ### Working with Content and Media Types
//!
//! ```rust
//! use rvoip_sip_core::types::{ContentType, MediaType};
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::types::Method;
//! use std::str::FromStr;
//!
//! // Create a request with SDP content
//! let sdp_body = "v=0\r\no=alice 2890844526 2890844526 IN IP4 127.0.0.1\r\ns=Session\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
//!
//! let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
//!     .content_type("application/sdp")
//!     .body(sdp_body)
//!     .build();
//!
//! // Verify the content-type header
//! if let Some(content_type) = request.typed_header::<ContentType>() {
//!     // Get the media type string
//!     let media_type_str = content_type.to_string();
//!     assert!(media_type_str.contains("application/sdp"));
//! }
//!
//! // Get the body content
//! assert_eq!(String::from_utf8_lossy(&request.body), sdp_body);
//! ```
//!
//! ### Working with Routing Headers
//!
//! ```rust
//! use rvoip_sip_core::types::{Uri, Address, Param};
//! use rvoip_sip_core::types::route::Route;
//! use rvoip_sip_core::types::{TypedHeader, HeaderName};
//! use rvoip_sip_core::builder::SimpleRequestBuilder;
//! use rvoip_sip_core::types::Method;
//! use std::str::FromStr;
//!
//! // Create a request with Route header
//! let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap();
//!
//! // Create a Route address with lr parameter
//! let proxy_uri = Uri::from_str("sip:proxy.example.com").unwrap();
//! let mut proxy_addr = Address::new(proxy_uri);
//! proxy_addr.params.push(Param::new("lr", None::<String>));
//!
//! // Create and add the Route header 
//! let route = Route::with_address(proxy_addr);
//! let req_with_route = request.header(TypedHeader::Route(route)).build();
//!
//! // Verify the Route header exists
//! assert!(req_with_route.header(&HeaderName::Route).is_some());
//! ```
//!
//! ### Adding Custom Headers
//!
//! ```rust
//! use rvoip_sip_core::types::{HeaderName, HeaderValue};
//! use rvoip_sip_core::types::TypedHeader;
//! use rvoip_sip_core::builder::SimpleResponseBuilder;
//! use rvoip_sip_core::types::StatusCode;
//!
//! // Create a response with custom headers
//! let response = SimpleResponseBuilder::new(StatusCode::BadRequest, None)
//!     .header(TypedHeader::Other(
//!         HeaderName::Other("X-Custom-Header".to_string()),
//!         HeaderValue::text("Custom Value")
//!     ))
//!     .build();
//!
//! // Retrieve and verify the custom header
//! let custom = response.header(&HeaderName::Other("X-Custom-Header".to_string()));
//! assert!(custom.is_some());
//! assert_eq!(custom.unwrap().to_string(), "X-Custom-Header: Custom Value");
//! ```

use std::str::FromStr;

pub mod via;
pub use via::Via;

pub mod method;
pub use method::Method;

pub mod status;
pub use status::StatusCode;

pub mod sip_message;
pub mod sip_request;
pub mod sip_response;
pub use sip_message::Message;
pub use sip_request::Request;
pub use sip_response::Response;

pub mod param;
pub use param::Param;

// Add new URI module
pub mod uri;
pub use uri::{Uri, Host, Scheme};

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
pub mod headers;
pub mod in_reply_to;
pub mod max_forwards;
pub mod organization;
pub mod record_route;
pub mod reply_to;
pub mod refer_to;
pub mod referred_by;
pub mod require;
pub mod route;
pub mod sdp;
pub mod subject;
pub mod to;
pub mod warning;
pub mod multipart;
pub mod path;
pub mod proxy_require;

// Modules missing re-exports - Add them
pub mod priority;
pub mod server;
pub mod retry_after;
pub mod error_info;
pub mod supported;
pub mod unsupported;
pub mod content_encoding;
pub mod content_language;
pub mod accept_encoding;
pub mod reason;
pub mod alert_info;

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
pub use content_encoding::ContentEncoding;
pub use content_language::ContentLanguage;
pub use accept_encoding::AcceptEncoding;
pub use path::Path;
pub use proxy_require::ProxyRequire;

// Add missing pub use * directives
pub use priority::*;
pub use server::*;
pub use retry_after::*;
pub use error_info::*;
pub use supported::Supported;
pub use unsupported::Unsupported;
pub use reason::Reason;
pub use alert_info::{AlertInfo, AlertInfoHeader, AlertInfoList};

// Add AsRef implementations for Message
impl AsRef<Message> for Message {
    fn as_ref(&self) -> &Message {
        self
    }
}

// Add user_agent module declaration
pub mod user_agent;