use std::str::FromStr;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::{
    Method, 
    Version,
    sip_request::Request,
    uri::{Uri, Host, Scheme},
    to::To,
    from::From,
    call_id::CallId,
    cseq::CSeq,
    contact::{Contact, ContactParamInfo, ContactValue},
    content_type::ContentType,
    content_length::ContentLength,
    via::{Via, ViaHeader, SentProtocol},
    Address,
    TypedHeader,
    Param,
    max_forwards::MaxForwards,
    user_agent::UserAgent,
    header::{HeaderName, HeaderValue},
};

/// # SIP Request Builder
///
/// The SimpleRequestBuilder provides a streamlined approach to creating SIP request messages
/// as defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261).
///
/// ## SIP Request Overview
///
/// SIP (Session Initiation Protocol) requests are messages sent by clients to servers to initiate
/// actions or transactions. Each request contains a method indicating the desired action, a 
/// Request-URI identifying the resource, and various headers providing additional information.
///
/// A typical SIP request looks like:
///
/// ```text
/// INVITE sip:bob@example.com SIP/2.0
/// Via: SIP/2.0/UDP alice.example.com:5060;branch=z9hG4bK776asdhds
/// Max-Forwards: 70
/// To: Bob <sip:bob@example.com>
/// From: Alice <sip:alice@example.com>;tag=1928301774
/// Call-ID: a84b4c76e66710
/// CSeq: 314159 INVITE
/// Contact: <sip:alice@192.168.1.2:5060>
/// Content-Type: application/sdp
/// Content-Length: 142
///
/// [SDP message body]
/// ```
///
/// ## Common SIP Methods
///
/// SIP defines several methods for different purposes:
///
/// - **INVITE**: Initiates a session (call) between endpoints
/// - **ACK**: Acknowledges receipt of a final response to an INVITE
/// - **BYE**: Terminates an established session
/// - **CANCEL**: Cancels a pending INVITE transaction
/// - **REGISTER**: Registers contact information with a SIP server
/// - **OPTIONS**: Queries a server about its capabilities
/// - **REFER**: Asks recipient to issue a request (typically used for transfers)
/// - **SUBSCRIBE**: Requests notification of an event
/// - **NOTIFY**: Sends a notification of an event
/// - **MESSAGE**: Sends an instant message (RFC 3428)
///
/// ## Key SIP Request Headers
///
/// - **Via**: Indicates the transport path taken by the request so far
/// - **Max-Forwards**: Limits the number of hops a request can make
/// - **From**: Identifies the logical initiator of the request
/// - **To**: Identifies the logical recipient of the request
/// - **Call-ID**: Unique identifier for this call or registration
/// - **CSeq**: Sequence number and method for ordering requests
/// - **Contact**: Direct URI at which the sender can be reached
/// - **Content-Type/Content-Length**: Describes the message body, if present
///
/// ## SIP Dialog Context
///
/// Many SIP requests operate within the context of a dialog - a peer-to-peer relationship
/// between two SIP endpoints. Dialogs are established by certain transactions (like INVITE)
/// and provide context for subsequent requests.
///
/// Dialog identification requires:
/// - Call-ID value
/// - Local tag (From header tag)
/// - Remote tag (To header tag)
///
/// ## Transaction Model
///
/// SIP uses a transaction model to group requests and responses:
///
/// - **INVITE transactions**: Used to establish sessions, includes a three-way handshake
/// - **Non-INVITE transactions**: Used for other methods, with a simpler two-way handshake
///
/// ## Benefits of Using SimpleRequestBuilder
///
/// The SimpleRequestBuilder provides several advantages:
///
/// - **Ergonomic API**: Fluent interface with method chaining
/// - **Default Handling**: Sets reasonable defaults for many optional fields
/// - **RFC Compliance**: Ensures compliance with SIP standards and conventions
/// - **Header Management**: Properly formats and validates SIP headers
/// - **Method-Specific Builders**: Convenience constructors for common requests
/// - **Type Safety**: Leverages Rust's type system to prevent invalid messages
///
/// The examples below demonstrate how to create various types of SIP requests
/// for common scenarios.
///
pub struct SimpleRequestBuilder {
    request: Request,
}

impl SimpleRequestBuilder {
    /// Create a new SimpleRequestBuilder with the specified method and URI
    ///
    /// This is the main entry point for creating a SIP request builder. The URI must be 
    /// syntactically valid according to [RFC 3261 Section 19.1.1](https://datatracker.ietf.org/doc/html/rfc3261#section-19.1.1).
    ///
    /// # Parameters
    /// - `method`: The SIP method (INVITE, REGISTER, etc.)
    /// - `uri`: The target URI as a string (e.g., "sip:user@example.com")
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a new INVITE request builder
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap();
    ///
    /// // Invalid URI will return an error
    /// let error_builder = SimpleRequestBuilder::new(Method::Invite, "invalid:uri");
    /// assert!(error_builder.is_err());
    /// ```
    pub fn new(method: Method, uri: &str) -> Result<Self> {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let request = Request::new(method, uri);
                Ok(Self { request })
            },
            Err(e) => Err(Error::InvalidUri(format!("Invalid URI: {}", e))),
        }
    }
    
    /// Create a builder from an existing Request object
    ///
    /// This allows you to modify an existing request by using the builder pattern.
    ///
    /// # Parameters
    /// - `request`: An existing SIP Request object
    ///
    /// # Returns
    /// A SimpleRequestBuilder initialized with the provided request
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::{Method, Uri, Request};
    /// use std::str::FromStr;
    ///
    /// let uri = Uri::from_str("sip:bob@example.com").unwrap();
    /// let request = Request::new(Method::Invite, uri);
    ///
    /// // Create a builder from the existing request
    /// let builder = SimpleRequestBuilder::from_request(request);
    /// ```
    pub fn from_request(request: Request) -> Self {
        Self { request }
    }
    
    /// Create an INVITE request builder
    ///
    /// This is a convenience constructor for creating an INVITE request as specified
    /// in [RFC 3261 Section 13](https://datatracker.ietf.org/doc/html/rfc3261#section-13).
    /// INVITE requests are used to establish media sessions between user agents.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic INVITE Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
    /// ```
    ///
    /// ## Complete INVITE Request with SDP Body
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    /// use rvoip_sip_core::sdp::attributes::MediaDirection;
    ///
    /// // Create SDP for a voice call using the SdpBuilder pattern
    /// let sdp_body = SdpBuilder::new("SIP Voice Call")
    ///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.168.1.2")
    ///     .connection("IN", "IP4", "192.168.1.2")
    ///     .time("0", "0")
    ///     .media_audio(49170, "RTP/AVP")
    ///         .formats(&["0", "8"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .rtpmap("8", "PCMA/8000")
    ///         .direction(MediaDirection::SendRecv)
    ///         .done()
    ///     .build()
    ///     .expect("Valid SDP");
    ///
    /// // Create an INVITE request to establish a call
    /// let invite_request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     // Add required headers for a dialog
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     // Add routing information
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     // Add SDP information
    ///     .content_type("application/sdp")
    ///     .body(sdp_body.to_string())
    ///     .build();
    /// ```
    /// 
    /// ## INVITE Request with Audio and Video
    /// 
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    /// use rvoip_sip_core::sdp::attributes::MediaDirection;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::supported::Supported;
    ///
    /// // Create an SDP with audio and video streams using SdpBuilder
    /// let sdp_body = SdpBuilder::new("Audio/Video Call")
    ///     .origin("alice", "2890844526", "2890844526", "IN", "IP4", "192.168.1.2")
    ///     .connection("IN", "IP4", "192.168.1.2")
    ///     .time("0", "0")
    ///     // Add BUNDLE group to use a single transport for both media
    ///     .group("BUNDLE", &["audio", "video"])
    ///     // Audio stream
    ///     .media_audio(49170, "RTP/AVP")
    ///         .formats(&["0", "101"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .rtpmap("101", "telephone-event/8000")
    ///         .fmtp("101", "0-16")  // DTMF events
    ///         .direction(MediaDirection::SendRecv)
    ///         .mid("audio")  // Media ID for bundling
    ///         .done()
    ///     // Video stream
    ///     .media_video(49172, "RTP/AVP")
    ///         .formats(&["96", "97"])
    ///         .rtpmap("96", "H264/90000")
    ///         .fmtp("96", "profile-level-id=42e01f;packetization-mode=1")
    ///         .rtpmap("97", "VP8/90000")
    ///         .direction(MediaDirection::SendRecv)
    ///         .mid("video")  // Media ID for bundling
    ///         .done()
    ///     .build()
    ///     .expect("Valid SDP");
    ///
    /// // Create a complete INVITE request with audio and video
    /// let invite_with_av = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     // Support for required extensions
    ///     .header(TypedHeader::Supported(Supported::new(vec![
    ///         "100rel".to_string(),  // Reliable provisional responses
    ///         "ice".to_string(),     // Interactive Connectivity Establishment
    ///         "replaces".to_string() // Call replacement
    ///     ])))
    ///     // Add SDP body with audio and video
    ///     .content_type("application/sdp")
    ///     .body(sdp_body.to_string())
    ///     .build();
    /// ```
    pub fn invite(uri: &str) -> Result<Self> {
        Self::new(Method::Invite, uri)
    }
    
    /// Create a REGISTER request builder
    ///
    /// This is a convenience constructor for creating a REGISTER request as specified
    /// in [RFC 3261 Section 10](https://datatracker.ietf.org/doc/html/rfc3261#section-10).
    /// REGISTER requests are used to add, remove, and query bindings.
    ///
    /// # Parameters
    /// - `uri`: The registrar URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic REGISTER Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap();
    /// ```
    ///
    /// ## Complete Registration with Expiration Time
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::expires::Expires;
    /// 
    /// // Create a REGISTER request with a 1-hour expiration
    /// let register_request = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
    ///     // Add required headers for registration
    ///     .from("Alice", "sip:alice@example.com", Some("reg-tag-1"))
    ///     .to("Alice", "sip:alice@example.com", None) // To header matches From without tag
    ///     .call_id("reg-call-1@192.168.1.2")
    ///     .cseq(1)
    ///     // Add routing information
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     // Add registration information
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     .header(TypedHeader::Expires(Expires::new(3600))) // 1 hour registration
    ///     .build();
    /// ```
    ///
    /// ## Registration Refresh with Authentication
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::expires::Expires;
    /// use rvoip_sip_core::types::auth::{Authorization, AuthScheme};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create a URI for the authorization header
    /// let reg_uri = Uri::from_str("sip:registrar.example.com").unwrap();
    /// 
    /// // Create an Authorization header with digest authentication
    /// let auth = Authorization::new(
    ///     AuthScheme::Digest,
    ///     "alice",                                    // username
    ///     "example.com",                              // realm
    ///     "dcd98b7102dd2f0e8b11d0f600bfb0c093",      // nonce
    ///     reg_uri,                                    // uri
    ///     "a2ea68c230e5fea1ca715740fb14db97"         // response hash
    /// );
    ///
    /// // Create a REGISTER refresh with authentication
    /// let register_refresh = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("reg-tag-1"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .call_id("reg-call-1@192.168.1.2")
    ///     .cseq(2)  // Increment CSeq for refresh
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887asdhrt"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     .header(TypedHeader::Expires(Expires::new(3600)))
    ///     .header(TypedHeader::Authorization(auth))
    ///     .build();
    /// ```
    pub fn register(uri: &str) -> Result<Self> {
        Self::new(Method::Register, uri)
    }
    
    /// Create a BYE request builder
    ///
    /// This is a convenience constructor for creating a BYE request as specified
    /// in [RFC 3261 Section 15.1](https://datatracker.ietf.org/doc/html/rfc3261#section-15.1).
    /// BYE requests are used to terminate a specific dialog.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic BYE Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::bye("sip:bob@example.com").unwrap();
    /// ```
    ///
    /// ## Complete BYE Request for an Established Dialog
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// // Create a BYE request to terminate an active dialog
    /// let bye_request = SimpleRequestBuilder::bye("sip:bob@example.com").unwrap()
    ///     // For BYE requests, both From and To tags must be present
    ///     // as they identify the dialog being terminated
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))  // To tag is required for BYE
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(2)  // CSeq increments throughout the dialog
    ///     // Add routing information
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887jhgfd"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     .build();
    /// ```
    ///
    /// ## BYE Request with Custom Reason Header
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ReasonBuilderExt;
    ///
    /// // Create a BYE request with a reason for termination
    /// let bye_request = SimpleRequestBuilder::bye("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(3)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK99alhkgj"))
    ///     .max_forwards(70)
    ///     .reason("SIP", 487, Some("Client call transfer"))  // Add reason for the termination
    ///     .build();
    /// ```
    pub fn bye(uri: &str) -> Result<Self> {
        Self::new(Method::Bye, uri)
    }
    
    /// Create an OPTIONS request builder
    ///
    /// This is a convenience constructor for creating an OPTIONS request as specified
    /// in [RFC 3261 Section 11](https://datatracker.ietf.org/doc/html/rfc3261#section-11).
    /// OPTIONS requests are used to query the capabilities of a server.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic OPTIONS Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::options("sip:bob@example.com").unwrap();
    /// ```
    ///
    /// ## Complete OPTIONS Request to Query Server Capabilities
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::accept::Accept;
    ///
    /// // Create a simple empty Accept header - simplified for doc test
    /// let accept = Accept::new();
    ///
    /// // Create an OPTIONS request to query a server's capabilities
    /// let options_request = SimpleRequestBuilder::options("sip:server.example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("opt-tag-1"))
    ///     .to("Server", "sip:server.example.com", None)
    ///     .call_id("options-call-1@192.168.1.2")
    ///     .cseq(1)
    ///     // Add routing information
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     // Add Accept header to indicate which body formats we can process
    ///     .header(TypedHeader::Accept(accept))
    ///     .build();
    /// ```
    ///
    /// ## OPTIONS Request with Supported Extensions
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::supported::Supported;
    /// use rvoip_sip_core::types::allow::Allow;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create supported extensions header
    /// let supported = Supported::new(vec![
    ///     "100rel".to_string(),
    ///     "path".to_string(),
    ///     "timer".to_string(),
    ///     "replaces".to_string()
    /// ]);
    ///
    /// // Create Allow header for methods we support
    /// let mut allow = Allow::new();
    /// allow.add_method(Method::Invite);
    /// allow.add_method(Method::Ack);
    /// allow.add_method(Method::Cancel);
    /// allow.add_method(Method::Bye);
    /// allow.add_method(Method::Options);
    /// allow.add_method(Method::Refer);
    ///
    /// // Create an OPTIONS request that advertises our capabilities
    /// let options_request = SimpleRequestBuilder::options("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("opt-tag-2"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("options-call-2@192.168.1.2")
    ///     .cseq(1)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK99nnmbhr"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.2:5060", None)
    ///     // Add headers to advertise our capabilities
    ///     .header(TypedHeader::Supported(supported))
    ///     .header(TypedHeader::Allow(allow))
    ///     .build();
    /// ```
    pub fn options(uri: &str) -> Result<Self> {
        Self::new(Method::Options, uri)
    }
    
    /// Create an ACK request builder
    ///
    /// This is a convenience constructor for creating an ACK request as specified
    /// in [RFC 3261 Section 17.1.1.3](https://datatracker.ietf.org/doc/html/rfc3261#section-17.1.1.3).
    /// ACK requests are used to acknowledge final responses to INVITE requests.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic ACK Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::ack("sip:bob@example.com").unwrap();
    /// ```
    ///
    /// ## Complete ACK for a 200 OK Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// // Create an ACK to acknowledge a 200 OK response
    /// // The tags and Call-ID must match the dialog established by the INVITE
    /// let ack_request = SimpleRequestBuilder::ack("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Same tag as INVITE
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))  // To tag from the 200 OK response
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")  // Same as INVITE
    ///     .cseq(1)  // Must match the INVITE CSeq number
    ///     // Add routing information
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887jhgfd"))
    ///     .max_forwards(70)
    ///     .build();
    /// ```
    ///
    /// ## ACK with Route Headers for Record-Route
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use rvoip_sip_core::types::route::Route;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create route URIs from Record-Route headers received in the INVITE transaction
    /// let route1 = Uri::from_str("sip:proxy1.example.com;lr").unwrap();
    /// let route2 = Uri::from_str("sip:proxy2.example.com;lr").unwrap();
    /// 
    /// // Create route for proxies - simplified for doc test
    /// let mut route = Route::new(vec![]);
    /// route.add_uri(route2);
    /// route.add_uri(route1);
    ///
    /// // Create an ACK with routing information from Record-Routes
    /// let ack_request = SimpleRequestBuilder::ack("sip:bob@192.168.2.3:5060").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887jhgfd"))
    ///     .max_forwards(70)
    ///     // Add route header to follow the same path as the INVITE
    ///     .header(TypedHeader::Route(route))
    ///     .build();
    /// ```
    pub fn ack(uri: &str) -> Result<Self> {
        Self::new(Method::Ack, uri)
    }
    
    /// Create a CANCEL request builder
    ///
    /// This is a convenience constructor for creating a CANCEL request as specified
    /// in [RFC 3261 Section 9](https://datatracker.ietf.org/doc/html/rfc3261#section-9).
    /// CANCEL requests are used to cancel a previous request sent by a client.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Basic CANCEL Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap();
    /// ```
    ///
    /// ## Complete CANCEL Request for a Pending INVITE
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// // Create a CANCEL request to terminate a pending INVITE
    /// // CANCEL must have the same request-URI, Call-ID, From, To, and CSeq number as the INVITE
    /// let cancel_request = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))  // Same as INVITE
    ///     .to("Bob", "sip:bob@example.com", None)  // Same as INVITE (no To tag yet)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")  // Same as INVITE
    ///     .cseq(1)  // Must match the INVITE CSeq number
    ///     // Add routing information (branch parameter can be different)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK776asghyt"))
    ///     .max_forwards(70)
    ///     .build();
    /// ```
    ///
    /// ## CANCEL Request with Route Headers
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::route::Route;
    /// use rvoip_sip_core::types::uri::Uri;
    /// use rvoip_sip_core::types::TypedHeader;
    /// use std::str::FromStr;
    ///
    /// // Route header from the original INVITE
    /// let route_uri = Uri::from_str("sip:proxy.example.com;lr").unwrap();
    /// // Create a simple route - simplified for doc test
    /// let mut route = Route::new(vec![]);
    /// route.add_uri(route_uri);
    ///
    /// // Create a CANCEL request for an INVITE
    /// let cancel_request = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887asdfgh"))
    ///     .max_forwards(70)
    ///     // Use same route as INVITE
    ///     .header(TypedHeader::Route(route))
    ///     .build();
    /// ```
    ///
    /// ## CANCEL Request with Reason Header
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::builder::headers::ReasonBuilderExt;
    ///
    /// // Create a CANCEL request with a reason for termination
    /// let cancel_request = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     .via("192.168.1.2:5060", "UDP", Some("z9hG4bK887asdfgh"))
    ///     .max_forwards(70)
    ///     .reason_busy()  // Indicate the call is being canceled because user is busy
    ///     .build();
    ///
    /// // Alternatively, use reason_terminated() for standard request termination
    /// let cancel_standard = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1)
    ///     .reason_terminated()  // Standard reason for CANCEL (487 Request Terminated)
    ///     .build();
    /// ```
    pub fn cancel(uri: &str) -> Result<Self> {
        Self::new(Method::Cancel, uri)
    }
    
    /// Create a PUBLISH request builder
    ///
    /// This is a convenience constructor for creating a PUBLISH request as specified
    /// in [RFC 3903](https://datatracker.ietf.org/doc/html/rfc3903).
    /// PUBLISH requests are used to publish event state to an Event State Compositor.
    ///
    /// # Parameters
    /// - `uri`: The target URI of the Event State Compositor
    /// - `event`: The event package name (e.g., "presence")
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Initial PUBLISH Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let publish_request = SimpleRequestBuilder::publish("sip:alice@example.com", "presence").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .call_id("a84b4c76e66710@pc33.atlanta.com")
    ///     .cseq(1)
    ///     .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .expires(3600)
    ///     .content_type("application/pidf+xml")
    ///     .body("<presence>...</presence>")
    ///     .build();
    /// ```
    ///
    /// ## Refresh PUBLISH with SIP-If-Match
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let refresh_request = SimpleRequestBuilder::publish("sip:alice@example.com", "presence").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Alice", "sip:alice@example.com", None)
    ///     .call_id("a84b4c76e66710@pc33.atlanta.com")
    ///     .cseq(2)
    ///     .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .sip_if_match("abc123xyz")  // Entity tag from previous response
    ///     .expires(3600)
    ///     .content_type("application/pidf+xml")
    ///     .body("<presence>...</presence>")
    ///     .build();
    /// ```
    pub fn publish(uri: &str, event: &str) -> Result<Self> {
        let mut builder = Self::new(Method::Publish, uri)?;
        builder = builder.event(event);
        Ok(builder)
    }
    
    /// Create a SUBSCRIBE request builder
    ///
    /// This is a convenience constructor for creating a SUBSCRIBE request as specified
    /// in [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
    /// SUBSCRIBE requests are used to subscribe to event notifications.
    ///
    /// # Parameters
    /// - `uri`: The target URI to subscribe to
    /// - `event`: The event package name (e.g., "presence", "dialog")
    /// - `expires`: Subscription duration in seconds (0 to unsubscribe)
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Initial SUBSCRIBE Request
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let subscribe_request = SimpleRequestBuilder::subscribe("sip:bob@example.com", "presence", 3600).unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", None)  // No To tag for initial SUBSCRIBE
    ///     .call_id("a84b4c76e66710@pc33.atlanta.com")
    ///     .cseq(1)
    ///     .via("192.168.1.10:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .contact("sip:alice@192.168.1.10:5060", None)
    ///     .build();
    /// ```
    pub fn subscribe(uri: &str, event: &str, expires: u32) -> Result<Self> {
        let mut builder = Self::new(Method::Subscribe, uri)?;
        builder = builder.event(event).expires(expires);
        Ok(builder)
    }
    
    /// Create a NOTIFY request builder
    ///
    /// This is a convenience constructor for creating a NOTIFY request as specified
    /// in [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
    /// NOTIFY requests are used to send event notifications to subscribers.
    ///
    /// # Parameters
    /// - `uri`: The target URI of the subscriber
    /// - `event`: The event package name
    /// - `state`: The subscription state (e.g., "active;expires=3600", "terminated")
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    ///
    /// # Examples
    ///
    /// ## Active Notification with Presence Data
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let notify_request = SimpleRequestBuilder::notify(
    ///     "sip:alice@192.168.1.10:5060",
    ///     "presence",
    ///     "active;expires=3599"
    /// ).unwrap()
    ///     .from("Bob", "sip:bob@example.com", Some("xyz987"))
    ///     .to("Alice", "sip:alice@example.com", Some("1928301774"))  // Must have To tag
    ///     .call_id("a84b4c76e66710@pc33.atlanta.com")
    ///     .cseq(1)
    ///     .via("192.168.1.20:5060", "UDP", Some("z9hG4bK776branch"))
    ///     .max_forwards(70)
    ///     .contact("sip:bob@192.168.1.20:5060", None)
    ///     .content_type("application/pidf+xml")
    ///     .body("<presence>...</presence>")
    ///     .build();
    /// ```
    pub fn notify(uri: &str, event: &str, state: &str) -> Result<Self> {
        let mut builder = Self::new(Method::Notify, uri)?;
        builder = builder.event(event).subscription_state(state);
        Ok(builder)
    }

    /// Get the method of the request being built
    ///
    /// # Returns
    /// A reference to the Method
    pub fn method(&self) -> &Method {
        &self.request.method
    }
    
    /// Add a From header with optional tag parameter
    ///
    /// Creates and adds a From header as specified in [RFC 3261 Section 20.20](https://datatracker.ietf.org/doc/html/rfc3261#section-20.20).
    /// The From header indicates the logical identity of the initiator of the request.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header (e.g., "Alice")
    /// - `uri`: The URI for the From header (e.g., "sip:alice@example.com")
    /// - `tag`: Optional tag parameter for dialog identification
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"));
    /// ```
    pub fn from(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        use crate::builder::headers::FromBuilderExt;
        FromBuilderExt::from(self, display_name, uri, tag)
    }
    
    /// Add a To header with optional tag parameter
    ///
    /// Creates and adds a To header as specified in [RFC 3261 Section 20.39](https://datatracker.ietf.org/doc/html/rfc3261#section-20.39).
    /// The To header specifies the logical recipient of the request.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header (e.g., "Bob")
    /// - `uri`: The URI for the To header (e.g., "sip:bob@example.com")
    /// - `tag`: Optional tag parameter for dialog identification
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .to("Bob", "sip:bob@example.com", None);
    /// ```
    pub fn to(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        use crate::builder::headers::ToBuilderExt;
        ToBuilderExt::to(self, display_name, uri, tag)
    }
    
    /// Add a Call-ID header
    ///
    /// Creates and adds a Call-ID header as specified in [RFC 3261 Section 20.8](https://datatracker.ietf.org/doc/html/rfc3261#section-20.8).
    /// The Call-ID header uniquely identifies a particular invitation or all registrations of a particular client.
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value (e.g., "f81d4fae-7dec-11d0-a765-00a0c91e6bf6@example.com")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com");
    /// ```
    pub fn call_id(self, call_id: &str) -> Self {
        use crate::builder::headers::CallIdBuilderExt;
        CallIdBuilderExt::call_id(self, call_id)
    }
    
    /// Add a CSeq header for requests
    ///
    /// Creates and adds a CSeq header as specified in [RFC 3261 Section 20.16](https://datatracker.ietf.org/doc/html/rfc3261#section-20.16).
    /// The CSeq header serves as a way to identify and order transactions.
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .cseq(1);
    /// ```
    pub fn cseq(self, seq: u32) -> Self {
        use crate::builder::headers::CSeqBuilderExt;
        CSeqBuilderExt::cseq(self, seq)
    }
    
    /// Add a Via header with optional branch parameter
    ///
    /// Creates and adds a Via header as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
    /// The Via header indicates the path taken by the request so far and helps route responses back.
    ///
    /// # Parameters
    /// - `host`: The host or IP address (e.g., "192.168.1.1" or "example.com:5060")
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be prefixed with z9hG4bK per RFC 3261)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"));
    /// ```
    pub fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        use crate::builder::headers::ViaBuilderExt;
        ViaBuilderExt::via(self, host, transport, branch)
    }
    
    /// Add a Max-Forwards header
    ///
    /// Creates and adds a Max-Forwards header as specified in [RFC 3261 Section 20.22](https://datatracker.ietf.org/doc/html/rfc3261#section-20.22).
    /// The Max-Forwards header limits the number of hops a request can transit.
    ///
    /// # Parameters
    /// - `value`: The Max-Forwards value (typically 70)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .max_forwards(70);
    /// ```
    pub fn max_forwards(self, value: u32) -> Self {
        use crate::builder::headers::MaxForwardsBuilderExt;
        MaxForwardsBuilderExt::max_forwards(self, value)
    }
    
    /// Add a Contact header
    ///
    /// Creates and adds a Contact header as specified in [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
    /// The Contact header provides a URI that can be used to directly contact the user agent.
    ///
    /// # Parameters
    /// - `uri`: The contact URI as a string (e.g., "sip:alice@192.168.1.1:5060")
    /// - `display_name`: Optional display name (e.g., "Alice")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .contact("sip:alice@192.168.1.1:5060", Some("Alice"));
    /// ```
    pub fn contact(self, uri: &str, display_name: Option<&str>) -> Self {
        use crate::builder::headers::ContactBuilderExt;
        ContactBuilderExt::contact(self, uri, display_name)
    }
    
    /// Add a Content-Type header
    ///
    /// Creates and adds a Content-Type header as specified in [RFC 3261 Section 20.15](https://datatracker.ietf.org/doc/html/rfc3261#section-20.15).
    /// The Content-Type header indicates the media type of the message body.
    ///
    /// # Parameters
    /// - `content_type`: The content type (e.g., "application/sdp")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .content_type("application/sdp");
    /// ```
    pub fn content_type(mut self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.request = self.request.with_header(TypedHeader::ContentType(ct));
            },
            Err(_) => {
                // Silently fail - content-type is not critical
            }
        }
        self
    }
    
    /// Add an Event header
    ///
    /// Sets the Event header for SUBSCRIBE, NOTIFY, and PUBLISH requests as specified
    /// in [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
    ///
    /// # Parameters
    /// - `event_type`: The event package name (e.g., "presence", "dialog", "message-summary")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
    ///     .event("presence");
    /// ```
    pub fn event(mut self, event_type: &str) -> Self {
        use crate::types::event::{Event, EventType};
        let event = Event::new(EventType::Token(event_type.to_string()));
        self.request = self.request.with_header(TypedHeader::Event(event));
        self
    }
    
    /// Add a SIP-If-Match header
    ///
    /// Sets the SIP-If-Match header for conditional PUBLISH requests as specified
    /// in [RFC 3903](https://datatracker.ietf.org/doc/html/rfc3903).
    ///
    /// # Parameters
    /// - `etag`: The entity tag value to match
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Publish, "sip:alice@example.com").unwrap()
    ///     .event("presence")
    ///     .sip_if_match("abc123xyz");
    /// ```
    pub fn sip_if_match(mut self, etag: &str) -> Self {
        use crate::types::sip_if_match::SipIfMatch;
        self.request = self.request.with_header(TypedHeader::SipIfMatch(SipIfMatch::new(etag)));
        self
    }
    
    /// Add a Subscription-State header
    ///
    /// Sets the Subscription-State header for NOTIFY requests as specified
    /// in [RFC 6665](https://datatracker.ietf.org/doc/html/rfc6665).
    ///
    /// # Parameters
    /// - `state`: The subscription state (e.g., "active;expires=3600", "terminated;reason=timeout")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Notify, "sip:alice@192.168.1.10").unwrap()
    ///     .event("presence")
    ///     .subscription_state("active;expires=3599");
    /// ```
    pub fn subscription_state(mut self, state: &str) -> Self {
        use std::str::FromStr;
        use crate::types::subscription_state::SubscriptionState as SubscriptionStateType;

        if let Ok(sub_state) = SubscriptionStateType::from_str(state) {
            self.request = self.request.with_header(TypedHeader::SubscriptionState(sub_state));
        }
        self
    }
    
    /// Add an Expires header
    ///
    /// Sets the Expires header for REGISTER, SUBSCRIBE, and PUBLISH requests.
    ///
    /// # Parameters
    /// - `seconds`: The expiration time in seconds (0 means remove/unsubscribe)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Subscribe, "sip:bob@example.com").unwrap()
    ///     .event("presence")
    ///     .expires(3600);
    /// ```
    pub fn expires(mut self, seconds: u32) -> Self {
        use crate::types::expires::Expires;
        // Use the builder's header method which properly handles single-value headers
        self.header(TypedHeader::Expires(Expires::new(seconds)))
    }
    
    /// Add a generic header
    ///
    /// Allows adding any supported SIP header type using the [`TypedHeader`][`crate::types::TypedHeader`] enum.
    /// This is useful for headers that don't have a dedicated method in the builder.
    ///
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader};
    /// use rvoip_sip_core::types::user_agent::UserAgent;
    ///
    /// let user_agent = UserAgent::single("RVOIP/1.0");
    /// let products = vec!["RVOIP/1.0".to_string()]; // Convert to Vec<String> for TypedHeader::UserAgent
    /// 
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .header(TypedHeader::UserAgent(products));
    /// ```
    pub fn header(mut self, header: TypedHeader) -> Self {
        let new_header_name = header.name();

        match &header {
            // Single-value headers: Remove existing headers of the same name before adding the new one.
            TypedHeader::Expires(_) |
            TypedHeader::From(_) |
            TypedHeader::To(_) |
            TypedHeader::CallId(_) |
            TypedHeader::CSeq(_) |
            TypedHeader::MaxForwards(_) |
            TypedHeader::ContentType(_) |
            TypedHeader::ContentLength(_) |
            TypedHeader::SessionExpires(_) |
            TypedHeader::UserAgent(_) |
            TypedHeader::Server(_) |
            TypedHeader::Organization(_) |
            TypedHeader::Subject(_) |
            TypedHeader::Priority(_) |
            TypedHeader::MimeVersion(_) |
            TypedHeader::ReferTo(_) |
            TypedHeader::ReferredBy(_) |
            TypedHeader::RetryAfter(_) |
            TypedHeader::MinExpires(_) |
            TypedHeader::Reason(_) |
            TypedHeader::Timestamp(_) |
            TypedHeader::Event(_) |
            TypedHeader::SubscriptionState(_) |
            TypedHeader::MinSE(_) |
            TypedHeader::Date(_) |
            TypedHeader::RSeq(_) |
            TypedHeader::SipETag(_) |        // Single-value header for entity tags
            TypedHeader::SipIfMatch(_) => {  // Single-value header for conditional requests
                 self.request.headers.retain(|h| h.name() != new_header_name);
            }
            // Appendable headers: These headers can appear multiple times.
            TypedHeader::Route(_) |
            TypedHeader::RecordRoute(_) |
            TypedHeader::Via(_) |
            TypedHeader::Contact(_) |
            TypedHeader::Warning(_) |
            TypedHeader::Accept(_) |
            TypedHeader::AcceptEncoding(_) |
            TypedHeader::AcceptLanguage(_) |
            TypedHeader::Allow(_) |
            TypedHeader::AllowEvents(_) |  // Allow-Events can appear multiple times
            TypedHeader::Supported(_) |
            TypedHeader::Unsupported(_) |
            TypedHeader::Require(_) |
            TypedHeader::ProxyRequire(_) |
            TypedHeader::AlertInfo(_) |
            TypedHeader::CallInfo(_) |
            TypedHeader::ErrorInfo(_) |
            TypedHeader::InReplyTo(_) |
            TypedHeader::ContentEncoding(_) |
            TypedHeader::ContentLanguage(_) |
            TypedHeader::ContentDisposition(_) |
            TypedHeader::WwwAuthenticate(_) |
            TypedHeader::Authorization(_) |
            TypedHeader::ProxyAuthenticate(_) |
            TypedHeader::ProxyAuthorization(_) |
            TypedHeader::AuthenticationInfo(_) |
            TypedHeader::ReplyTo(_) |
            TypedHeader::Path(_) => {  // Path header for Path service
                // For appendable headers, no special action is needed before pushing.
            }
            TypedHeader::Other(name, _value) => {
                if *name == HeaderName::ReferTo { 
                    self.request.headers.retain(|h| h.name() != HeaderName::ReferTo);
                }
                // For other headers, default to single-value behavior (replace existing)
                // unless it's a known appendable header
                let is_known_appendable = matches!(name,
                    HeaderName::Via | HeaderName::Route | HeaderName::RecordRoute |
                    HeaderName::Contact | HeaderName::Warning | HeaderName::CallInfo |
                    HeaderName::Supported | HeaderName::Unsupported | HeaderName::Require |
                    HeaderName::ProxyRequire | HeaderName::Allow | HeaderName::AllowEvents |
                    HeaderName::Accept | HeaderName::AcceptEncoding | HeaderName::AcceptLanguage |
                    HeaderName::AlertInfo | HeaderName::ErrorInfo | HeaderName::ContentEncoding |
                    HeaderName::ContentLanguage | HeaderName::ContentDisposition |
                    HeaderName::InReplyTo | HeaderName::Path | HeaderName::Authorization |
                    HeaderName::ProxyAuthorization | HeaderName::WwwAuthenticate | 
                    HeaderName::ProxyAuthenticate | HeaderName::AuthenticationInfo
                );
                if !is_known_appendable {
                    self.request.headers.retain(|h| h.name() != *name);
                }
            }
        };

        self.request.headers.push(header); // Removed .clone()
        self
    }
    
    /// Add body content and update Content-Length
    ///
    /// Adds a message body to the request and automatically sets the Content-Length header
    /// as specified in [RFC 3261 Section 20.14](https://datatracker.ietf.org/doc/html/rfc3261#section-20.14).
    ///
    /// # Parameters
    /// - `body`: The body content (e.g., SDP for session description)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let sdp_body = "v=0\r\no=alice 2890844526 2890844526 IN IP4 127.0.0.1\r\ns=Session\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
    ///
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .content_type("application/sdp")
    ///     .body(sdp_body);
    /// ```
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.request = self.request.with_body(body);
        self
    }
    
    /// Build the final Request
    ///
    /// Finalizes the request construction and returns the complete SIP request.
    ///
    /// # Returns
    /// The constructed [`Request`][`crate::types::sip_request::Request`]
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("a84b4c76e66710")
    ///     .cseq(1)
    ///     .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
    ///     .max_forwards(70)
    ///     .build();
    /// ```
    pub fn build(self) -> Request {
        self.request
    }
} 