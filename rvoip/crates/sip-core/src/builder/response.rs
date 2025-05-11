use std::str::FromStr;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::{
    Method,
    StatusCode,
    Version,
    sip_response::Response,
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
    HeaderName,
    HeaderValue,
};

/// # SIP Response Builder
///
/// The SimpleResponseBuilder provides a streamlined approach to creating SIP response messages
/// as defined in [RFC 3261](https://datatracker.ietf.org/doc/html/rfc3261).
///
/// ## SIP Response Overview
///
/// SIP (Session Initiation Protocol) responses are messages sent by servers to clients in response 
/// to client requests. Each response contains a status code indicating the outcome, a reason phrase, 
/// and various headers providing additional information.
///
/// A typical SIP response looks like:
///
/// ```text
/// SIP/2.0 200 OK
/// Via: SIP/2.0/UDP server10.example.com;branch=z9hG4bK4b43c2ff8.1;received=192.0.2.3
/// Via: SIP/2.0/UDP client.example.com:5060;branch=z9hG4bK74bf9;received=192.0.2.201
/// From: Alice <sip:alice@example.com>;tag=9fxced76sl
/// To: Bob <sip:bob@example.com>;tag=8321234356
/// Call-ID: 3848276298220188511@client.example.com
/// CSeq: 1 INVITE
/// Contact: <sip:bob@client.example.com>
/// Content-Type: application/sdp
/// Content-Length: 147
///
/// [SDP message body]
/// ```
///
/// ## SIP Response Status Codes
///
/// SIP defines several categories of status codes:
///
/// - **1xx (Provisional)**: Request received and being processed (e.g., 100 Trying, 180 Ringing)
/// - **2xx (Success)**: Request was successfully received, understood, and accepted (e.g., 200 OK)
/// - **3xx (Redirection)**: Further action needs to be taken to complete the request (e.g., 302 Moved Temporarily)
/// - **4xx (Client Error)**: Request contains bad syntax or cannot be fulfilled at this server (e.g., 404 Not Found)
/// - **5xx (Server Error)**: Server failed to fulfill an apparently valid request (e.g., 500 Server Internal Error)
/// - **6xx (Global Failure)**: Request cannot be fulfilled at any server (e.g., 603 Decline)
///
/// ## Key SIP Response Headers
///
/// - **Via**: Indicates the path taken by the request (copied from request with additional parameters)
/// - **From**: Identifies the logical initiator of the request (copied from request)
/// - **To**: Identifies the logical recipient of the request (copied from request, tag added if needed)
/// - **Call-ID**: Unique identifier for this call or registration (copied from request)
/// - **CSeq**: Sequence number and method for ordering requests (copied from request)
/// - **Contact**: Direct URI at which the responder can be reached (for dialog-establishing responses)
/// - **Content-Type/Content-Length**: Describes the message body, if present
///
/// ## Dialog Creation and Maintenance
///
/// Certain responses help establish and maintain dialogs between SIP user agents:
///
/// - **2xx responses to INVITE**: Establish dialogs and must include Contact headers
/// - **101-199 responses with to-tag**: Establish early dialogs when responding to INVITE
/// - **Responses within dialogs**: Must maintain dialog state with proper tags and CSeq values
///
/// ## Transaction Model
///
/// SIP responses work within transactions to provide reliability:
///
/// - **INVITE transaction responses**: May include provisional (1xx) responses, followed by final responses
/// - **Non-INVITE transaction responses**: Typically have single final response
/// - **2xx responses to INVITE**: Establish separate transactions for reliability through ACK
///
/// ## Benefits of Using SimpleResponseBuilder
///
/// The SimpleResponseBuilder provides several advantages:
///
/// - **Ergonomic API**: Fluent interface with method chaining
/// - **Default Handling**: Sets reasonable defaults for many fields
/// - **RFC Compliance**: Ensures compliance with SIP standards and conventions
/// - **Header Management**: Properly formats and validates SIP headers
/// - **Status-Specific Builders**: Convenience constructors for common responses
/// - **Type Safety**: Leverages Rust's type system to prevent invalid messages
///
/// The examples below demonstrate how to create various types of SIP responses
/// for common scenarios.

/// The SimpleResponseBuilder provides a streamlined approach to creating SIP response messages.
///
/// # Examples
///
/// ## Creating a Basic SIP Response
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::{Method, StatusCode};
///
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
///     .build();
///
/// assert_eq!(response.status_code(), 200);
/// ```
///
/// ## Using Convenience Constructors
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::Method;
///
/// // 200 OK response
/// let ok = SimpleResponseBuilder::ok()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .build();
///
/// // 100 Trying response
/// let trying = SimpleResponseBuilder::trying()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .build();
///
/// // 180 Ringing response
/// let ringing = SimpleResponseBuilder::ringing()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .build();
/// ```
///
/// ## Adding SDP Content
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::{Method, StatusCode};
/// use rvoip_sip_core::sdp::SdpBuilder;
/// use rvoip_sip_core::sdp::attributes::MediaDirection;
///
/// // Create an SDP body using the SdpBuilder pattern
/// let sdp_body = SdpBuilder::new("Session")
///     .origin("bob", "2890844527", "2890844527", "IN", "IP4", "127.0.0.1")
///     .connection("IN", "IP4", "127.0.0.1")
///     .time("0", "0")
///     .media_audio(49172, "RTP/AVP")
///         .formats(&["0"])
///         .rtpmap("0", "PCMU/8000")
///         .done()
///     .build()
///     .expect("Valid SDP");
///
/// let response = SimpleResponseBuilder::ok()
///     .from("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .to("Alice", "sip:alice@example.com", Some("1928301774"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .content_type("application/sdp")
///     .body(sdp_body.to_string())
///     .build();
/// ```
///
/// ## Working with Contact Headers
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::{Method, StatusCode};
///
/// let response = SimpleResponseBuilder::ok()
///     .from("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .to("Alice", "sip:alice@example.com", Some("1928301774"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .contact("sip:bob@192.168.1.2:5060", Some("Bob"))
///     .build();
/// ```
///
/// ## Creating Responses from Existing Ones
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleResponseBuilder;
/// use rvoip_sip_core::types::{Response, StatusCode};
///
/// // Create or get a response from somewhere
/// let response = Response::new(StatusCode::Ok);
///
/// // Create a builder from the existing response
/// let modified_response = SimpleResponseBuilder::from_response(response)
///     .from("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .to("Alice", "sip:alice@example.com", Some("1928301774"))
///     .build();
/// ```
pub struct SimpleResponseBuilder {
    response: Response,
}

impl SimpleResponseBuilder {
    /// Create a new SimpleResponseBuilder with the specified status code and optional reason phrase
    ///
    /// This is the main entry point for creating a SIP response builder. Status codes are defined
    /// in [RFC 3261 Section 21](https://datatracker.ietf.org/doc/html/rfc3261#section-21).
    ///
    /// # Parameters
    /// - `status`: The SIP status code (e.g., StatusCode::Ok (200), StatusCode::NotFound (404))
    /// - `reason`: Optional custom reason phrase (if None, the default for the status code will be used)
    ///
    /// # Returns
    /// A new SimpleResponseBuilder
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// // Create with default reason phrase
    /// let ok_builder = SimpleResponseBuilder::new(StatusCode::Ok, None);
    ///
    /// // Create with custom reason phrase
    /// let not_found_builder = SimpleResponseBuilder::new(StatusCode::NotFound, Some("User Not Available"));
    /// ```
    pub fn new(status: StatusCode, reason: Option<&str>) -> Self {
        let mut response = Response::new(status);
        
        if let Some(reason_text) = reason {
            response = response.with_reason(reason_text);
        }
        
        Self { response }
    }
    
    /// Create from an existing Response object
    ///
    /// This allows you to modify an existing response by using the builder pattern.
    ///
    /// # Parameters
    /// - `response`: An existing SIP Response object
    ///
    /// # Returns
    /// A SimpleResponseBuilder initialized with the provided response
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Response, StatusCode};
    ///
    /// // Create a response or get it from somewhere
    /// let response = Response::new(StatusCode::Ok);
    ///
    /// // Create a builder from the existing response
    /// let builder = SimpleResponseBuilder::from_response(response);
    /// ```
    pub fn from_response(response: Response) -> Self {
        Self { response }
    }
    
    /// Create a 200 OK response
    ///
    /// This is a convenience constructor for creating a 200 OK response as specified
    /// in [RFC 3261 Section 21.2.1](https://datatracker.ietf.org/doc/html/rfc3261#section-21.2.1).
    /// 200 OK responses indicate the request was successful.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 200 OK status
    ///
    /// # Examples
    ///
    /// ## Basic 200 OK Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::ok();
    /// ```
    ///
    /// ## Complete 200 OK Response to an INVITE
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a 200 OK response accepting an INVITE request
    /// let ok_response = SimpleResponseBuilder::ok()
    ///     // Copy headers from the original request
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))  // Add local tag
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)  // Same CSeq and method as request
    ///     // Copy Via headers from request (in same order)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Contact header is required in 2xx responses to INVITE
    ///     .contact("sip:bob@192.168.1.2:5060", Some("Bob"))
    ///     .build();
    /// ```
    ///
    /// ## 200 OK Response with SDP Answer
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    /// use rvoip_sip_core::sdp::attributes::MediaDirection;
    /// 
    /// // Create an SDP answer using the SdpBuilder pattern
    /// let sdp_answer = SdpBuilder::new("SIP Call")
    ///     .origin("bob", "2890844527", "2890844527", "IN", "IP4", "192.168.1.2")
    ///     .connection("IN", "IP4", "192.168.1.2")
    ///     .time("0", "0")
    ///     .media_audio(49172, "RTP/AVP")
    ///         .formats(&["0", "8"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .rtpmap("8", "PCMA/8000")
    ///         .direction(MediaDirection::SendRecv)
    ///         .done()
    ///     .build()
    ///     .expect("Valid SDP");
    ///
    /// // Create a 200 OK response with SDP answer to INVITE
    /// let ok_with_sdp = SimpleResponseBuilder::ok()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .contact("sip:bob@192.168.1.2:5060", None)
    ///     // Add SDP body with selected codecs
    ///     .content_type("application/sdp")
    ///     .body(sdp_answer.to_string())
    ///     .build();
    /// ```
    pub fn ok() -> Self {
        let response = Response::ok();
        Self { response }
    }
    
    /// Create a 100 Trying response
    ///
    /// This is a convenience constructor for creating a 100 Trying response as specified
    /// in [RFC 3261 Section 21.1.1](https://datatracker.ietf.org/doc/html/rfc3261#section-21.1.1).
    /// 100 Trying responses indicate the request has been received and the server is working on it.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 100 Trying status
    ///
    /// # Examples
    ///
    /// ## Basic 100 Trying Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::trying();
    /// ```
    ///
    /// ## Complete 100 Trying Response to an INVITE
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a 100 Trying response to acknowledge receipt of an INVITE
    /// // This is typically sent by a proxy or UAS while processing the request
    /// let trying_response = SimpleResponseBuilder::trying()
    ///     // Copy headers from the original request (no tags added for provisional responses)
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)  // No To tag in initial provisional response
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     // Copy all Via headers from request (in same order)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     .build();
    /// ```
    ///
    /// ## 100 Trying with Server Information
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader};
    ///
    /// // Create a 100 Trying response with Server header
    /// let trying_with_server = SimpleResponseBuilder::trying()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     // Add Server header to identify the server software
    ///     .header(TypedHeader::Server(vec![
    ///         "RVoIP-Proxy/1.0".to_string(),
    ///         "(Linux)".to_string()
    ///     ]))
    ///     .build();
    /// ```
    pub fn trying() -> Self {
        let response = Response::trying();
        Self { response }
    }
    
    /// Create a 180 Ringing response
    ///
    /// This is a convenience constructor for creating a 180 Ringing response as specified
    /// in [RFC 3261 Section 21.1.2](https://datatracker.ietf.org/doc/html/rfc3261#section-21.1.2).
    /// 180 Ringing responses indicate the user agent has located the callee and is alerting them.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 180 Ringing status
    ///
    /// # Examples
    ///
    /// ## Basic 180 Ringing Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::ringing();
    /// ```
    ///
    /// ## Complete 180 Ringing Response for an INVITE
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a 180 Ringing response to indicate the destination is being alerted
    /// let ringing_response = SimpleResponseBuilder::ringing()
    ///     // Copy headers from the original request
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))  // Add To tag to establish early dialog
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     // Copy Via headers from request (in same order)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Contact header in provisional responses helps with early media
    ///     .contact("sip:bob@192.168.1.2:5060", None)
    ///     .build();
    /// ```
    ///
    /// ## 180 Ringing with Early Media (SDP)
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    /// use rvoip_sip_core::sdp::attributes::MediaDirection;
    ///
    /// // Create an SDP for early media (ringback tone) using SdpBuilder pattern
    /// let early_media_sdp = SdpBuilder::new("Ringback Tone")
    ///     .origin("bob", "2890844527", "2890844527", "IN", "IP4", "192.168.1.2")
    ///     .connection("IN", "IP4", "192.168.1.2")
    ///     .time("0", "0")
    ///     .media_audio(49172, "RTP/AVP")
    ///         .formats(&["0"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .direction(MediaDirection::SendOnly)  // SendOnly for ringback tone
    ///         .done()
    ///     .build()
    ///     .expect("Valid SDP");
    ///
    /// // Create a 180 Ringing response with early media (ringback tone)
    /// let ringing_with_media = SimpleResponseBuilder::ringing()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", Some("b5qt9xl3"))
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("proxy.example.com:5060", "UDP", Some("z9hG4bK776asdhds"))
    ///     .contact("sip:bob@192.168.1.2:5060", None)
    ///     // Add SDP body for early media (ringback tone)
    ///     .content_type("application/sdp")
    ///     .body(early_media_sdp.to_string())
    ///     .build();
    /// ```
    pub fn ringing() -> Self {
        let response = Response::ringing();
        Self { response }
    }
    
    /// Create a 400 Bad Request response
    ///
    /// This is a convenience constructor for creating a 400 Bad Request response as specified
    /// in [RFC 3261 Section 21.4.1](https://datatracker.ietf.org/doc/html/rfc3261#section-21.4.1).
    /// 400 Bad Request responses indicate the server couldn't understand the request due to malformed syntax.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 400 Bad Request status
    ///
    /// # Examples
    ///
    /// ## Basic 400 Bad Request Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::bad_request();
    /// ```
    ///
    /// ## Complete 400 Bad Request Response with Reason
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader, Warning};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create warning with proper Uri (not string)
    /// let agent_uri = Uri::from_str("sip:proxy.example.com").unwrap();
    /// let warning = Warning::new(399, agent_uri, "Malformed Contact header");
    ///
    /// // Create a 400 Bad Request response for a malformed request
    /// let bad_request_response = SimpleResponseBuilder::bad_request()
    ///     // Include original headers when possible
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)  // No To tag for error responses
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     // Copy topmost Via header only for error responses
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Add a Warning header with more information about the error
    ///     .header(TypedHeader::Warning(vec![warning]))
    ///     .build();
    /// ```
    ///
    /// ## 400 Bad Request with Detailed Error Message
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader, Warning};
    ///
    /// // Create an explanation of the syntax error in plain text
    /// let error_body = 
    ///     "The request contained a malformed header:\r\n\
    ///     Contact: <sip:bob@example.com missing-bracket\r\n\
    ///     \r\n\
    ///     The correct format should be:\r\n\
    ///     Contact: <sip:bob@example.com>";
    ///
    /// // Create a 400 Bad Request with a text explanation
    /// let detailed_response = SimpleResponseBuilder::bad_request()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Add a detailed explanation in the body
    ///     .content_type("text/plain")
    ///     .body(error_body)
    ///     .build();
    /// ```
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BadRequest, None)
    }
    
    /// Create a 404 Not Found response
    ///
    /// This is a convenience constructor for creating a 404 Not Found response as specified
    /// in [RFC 3261 Section 21.4.4](https://datatracker.ietf.org/doc/html/rfc3261#section-21.4.4).
    /// 404 Not Found responses indicate the server has definitive information that the user does not exist at the domain specified in the Request-URI.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 404 Not Found status
    ///
    /// # Examples
    ///
    /// ## Basic 404 Not Found Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::not_found();
    /// ```
    ///
    /// ## Complete 404 Not Found Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    ///
    /// // Create a 404 Not Found response to an INVITE
    /// let not_found_response = SimpleResponseBuilder::not_found()
    ///     // Include original headers from the request
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Unknown", "sip:unknown@example.com", None)  // No To tag for error responses
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     // Only include the top Via header in error responses
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     .build();
    /// ```
    ///
    /// ## 404 Not Found with Alternative Destination
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader};
    ///
    /// // Create a 404 Not Found with alternative contact information
    /// let alternative_response = SimpleResponseBuilder::not_found()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@old-domain.example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Add a Contact header with alternative location
    ///     .contact("sip:bob@new-domain.example.com", Some("Bob"))
    ///     // Note: Reason header commented out until implemented
    ///     /*
    ///     // Add a Reason header explaining the change
    ///     .header(TypedHeader::Reason(
    ///         Reason::new("SIP", 301, Some("User moved permanently"))
    ///     ))
    ///     */
    ///     .build();
    /// ```
    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound, None)
    }
    
    /// Create a 500 Server Error response
    ///
    /// This is a convenience constructor for creating a 500 Server Internal Error response as specified
    /// in [RFC 3261 Section 21.5.1](https://datatracker.ietf.org/doc/html/rfc3261#section-21.5.1).
    /// 500 Server Internal Error responses indicate the server encountered an unexpected condition that prevented it from fulfilling the request.
    ///
    /// # Returns
    /// A SimpleResponseBuilder with a 500 Server Internal Error status
    ///
    /// # Examples
    ///
    /// ## Basic 500 Server Error Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::server_error();
    /// ```
    ///
    /// ## Complete 500 Server Error Response
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::Method;
    /// 
    /// // Create a 500 Server Error response for an INVITE
    /// let server_error_response = SimpleResponseBuilder::server_error()
    ///     // Include original headers from the request
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)  // No To tag for error responses
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     // Only include the top Via header in error responses
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Note: RetryAfter header commented out until implemented
    ///     /*
    ///     // Include a Retry-After header suggesting when to retry
    ///     .header(TypedHeader::RetryAfter(RetryAfter::new(300)))  // Retry after 5 minutes
    ///     */
    ///     .build();
    /// ```
    ///
    /// ## 500 Server Error with Detailed Warning
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{Method, TypedHeader, Warning};
    /// use rvoip_sip_core::types::uri::Uri;
    /// use std::str::FromStr;
    ///
    /// // Create warning URIs
    /// let agent_uri = Uri::from_str("sip:sip.example.com").unwrap();
    ///
    /// // Create a 500 Server Error with detailed warning information
    /// let detailed_error = SimpleResponseBuilder::server_error()
    ///     .from("Alice", "sip:alice@example.com", Some("a73kszlfl"))
    ///     .to("Bob", "sip:bob@example.com", None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@192.168.1.2")
    ///     .cseq(1, Method::Invite)
    ///     .via("alice-pc.example.com:5060", "UDP", Some("z9hG4bK123abcde"))
    ///     // Add informative headers
    ///     .header(TypedHeader::Warning(vec![
    ///         Warning::new(399, agent_uri.clone(), "Database connection timeout"),
    ///         Warning::new(399, agent_uri, "Server is under heavy load")
    ///     ]))
    ///     // Note: RetryAfter and Server headers commented out until implemented
    ///     /*
    ///     .header(TypedHeader::RetryAfter(RetryAfter::new(120)))  // Retry after 2 minutes
    ///     .header(TypedHeader::Server(vec![
    ///         "SIP-Core/1.0".to_string(),
    ///         "(Maintenance Mode)".to_string()
    ///     ]))
    ///     */
    ///     .build();
    /// ```
    pub fn server_error() -> Self {
        Self::new(StatusCode::ServerInternalError, None)
    }

    /// Add a From header with optional tag parameter
    ///
    /// Creates and adds a From header as specified in [RFC 3261 Section 20.20](https://datatracker.ietf.org/doc/html/rfc3261#section-20.20).
    /// In responses, the From header is copied from the request.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header (e.g., "Alice")
    /// - `uri`: The URI for the From header (e.g., "sip:alice@example.com")
    /// - `tag`: Optional tag parameter (should be the same as in the request)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"));
    /// ```
    pub fn from(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        use crate::builder::headers::FromBuilderExt;
        FromBuilderExt::from(self, display_name, uri, tag)
    }
    
    /// Add a To header with optional tag parameter
    ///
    /// Creates and adds a To header as specified in [RFC 3261 Section 20.39](https://datatracker.ietf.org/doc/html/rfc3261#section-20.39).
    /// In responses, the To header is copied from the request and a tag is added if it didn't already have one.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header (e.g., "Bob")
    /// - `uri`: The URI for the To header (e.g., "sip:bob@example.com")
    /// - `tag`: Optional tag parameter (should be added by UAS for dialogs)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"));
    /// ```
    pub fn to(self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        use crate::builder::headers::ToBuilderExt;
        ToBuilderExt::to(self, display_name, uri, tag)
    }
    
    /// Add a Call-ID header
    ///
    /// Creates and adds a Call-ID header as specified in [RFC 3261 Section 20.8](https://datatracker.ietf.org/doc/html/rfc3261#section-20.8).
    /// In responses, the Call-ID is copied from the request.
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
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .call_id("f81d4fae-7dec-11d0-a765-00a0c91e6bf6@host.example.com");
    /// ```
    pub fn call_id(self, call_id: &str) -> Self {
        use crate::builder::headers::CallIdBuilderExt;
        CallIdBuilderExt::call_id(self, call_id)
    }
    
    /// Add a CSeq header for responses (requires method)
    ///
    /// Creates and adds a CSeq header as specified in [RFC 3261 Section 20.16](https://datatracker.ietf.org/doc/html/rfc3261#section-20.16).
    /// In responses, the CSeq is copied from the request, including both the sequence number and method.
    ///
    /// # Parameters
    /// - `seq`: The sequence number (e.g., 1, 2, 3)
    /// - `method`: The method in the CSeq header (e.g., Method::Invite)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{StatusCode, Method};
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .cseq(1, Method::Invite);
    /// ```
    pub fn cseq(self, seq: u32, method: Method) -> Self {
        use crate::builder::headers::CSeqBuilderExt;
        CSeqBuilderExt::cseq_with_method(self, seq, method)
    }
    
    /// Add a Via header with optional branch parameter
    ///
    /// Creates and adds a Via header as specified in [RFC 3261 Section 20.42](https://datatracker.ietf.org/doc/html/rfc3261#section-20.42).
    /// In responses, the Via headers are copied from the request in the same order.
    ///
    /// # Parameters
    /// - `host`: The host or IP address (e.g., "192.168.1.1" or "example.com:5060")
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be the same as in the request)
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .via("192.168.1.1:5060", "UDP", Some("z9hG4bK776asdhds"));
    /// ```
    pub fn via(self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        use crate::builder::headers::ViaBuilderExt;
        ViaBuilderExt::via(self, host, transport, branch)
    }
    
    /// Add a Contact header
    ///
    /// Creates and adds a Contact header as specified in [RFC 3261 Section 20.10](https://datatracker.ietf.org/doc/html/rfc3261#section-20.10).
    /// In responses, the Contact header provides a URI for subsequent requests in the dialog.
    ///
    /// # Parameters
    /// - `uri`: The contact URI as a string (e.g., "sip:bob@192.168.1.2:5060")
    /// - `display_name`: Optional display name (e.g., "Bob")
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .contact("sip:bob@192.168.1.2:5060", Some("Bob"));
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
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .content_type("application/sdp");
    /// ```
    pub fn content_type(mut self, content_type: &str) -> Self {
        match ContentType::from_str(content_type) {
            Ok(ct) => {
                self.response = self.response.with_header(TypedHeader::ContentType(ct));
            },
            Err(_) => {
                // Silently fail - content-type is not critical
            }
        }
        self
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
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{StatusCode, TypedHeader};
    /// use rvoip_sip_core::types::server::ServerInfo;
    ///
    /// // Create a vector of server product tokens
    /// let server_products = vec!["SIPCore/2.1".to_string(), "(High Performance Edition)".to_string()];
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .header(TypedHeader::Server(server_products));
    /// ```
    pub fn header(mut self, header: TypedHeader) -> Self {
        let new_header_name = header.name();

        match &header {
            // Single-value headers: Remove existing headers of the same name before adding the new one.
            TypedHeader::From(_) |
            TypedHeader::To(_) |
            TypedHeader::CallId(_) |
            TypedHeader::CSeq(_) |
            TypedHeader::MaxForwards(_) | // Though MaxForwards is less common in responses, treat as single if set directly.
            TypedHeader::ContentType(_) |
            TypedHeader::ContentLength(_) |
            TypedHeader::Expires(_) |
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
            TypedHeader::ReplyTo(_) |
            TypedHeader::Event(_) |
            TypedHeader::SubscriptionState(_) |
            TypedHeader::MinExpires(_) |
            TypedHeader::Date(_) |
            TypedHeader::Timestamp(_) => {
                self.response.headers.retain(|h| h.name() != new_header_name);
            }
            
            // Appendable headers: These headers can appear multiple times.
            TypedHeader::Via(_) |
            TypedHeader::Route(_) |         // Less common in responses but possible
            TypedHeader::RecordRoute(_) |
            TypedHeader::Contact(_) |
            TypedHeader::Warning(_) |
            TypedHeader::CallInfo(_) |
            TypedHeader::Supported(_) |
            TypedHeader::Unsupported(_) |
            TypedHeader::Require(_) |
            TypedHeader::ProxyRequire(_) |  // Less common in responses
            TypedHeader::Allow(_) |
            TypedHeader::Accept(_) |
            TypedHeader::AcceptEncoding(_) |
            TypedHeader::AcceptLanguage(_) |
            TypedHeader::AlertInfo(_) |
            TypedHeader::ErrorInfo(_) |
            TypedHeader::ContentEncoding(_) |
            TypedHeader::ContentLanguage(_) |
            TypedHeader::ContentDisposition(_) |
            TypedHeader::InReplyTo(_) |     // Less common in responses
            TypedHeader::Path(_) |          // For Path service
            TypedHeader::Authorization(_) | // Typically in requests, but handle if set
            TypedHeader::ProxyAuthorization(_) | // Typically in requests
            TypedHeader::WwwAuthenticate(_) |
            TypedHeader::ProxyAuthenticate(_) |
            TypedHeader::AuthenticationInfo(_) |
            TypedHeader::Reason(_) => {
                // No retain logic, these headers are appended.
            }

            // Handle 'Other' separately to decide based on its actual name
            TypedHeader::Other(name, _value_ref) => {
                let is_known_appendable_by_name = matches!(name,
                    HeaderName::Via | HeaderName::Route | HeaderName::RecordRoute |
                    HeaderName::Contact | HeaderName::Warning | HeaderName::CallInfo |
                    HeaderName::Supported | HeaderName::Unsupported | HeaderName::Require |
                    HeaderName::ProxyRequire | HeaderName::Allow | HeaderName::Accept |
                    HeaderName::AcceptEncoding | HeaderName::AcceptLanguage |
                    HeaderName::AlertInfo | HeaderName::ErrorInfo | HeaderName::ContentEncoding |
                    HeaderName::ContentLanguage | HeaderName::ContentDisposition |
                    HeaderName::InReplyTo | HeaderName::Path | HeaderName::Authorization |
                    HeaderName::ProxyAuthorization | HeaderName::WwwAuthenticate | HeaderName::ProxyAuthenticate |
                    HeaderName::AuthenticationInfo | HeaderName::Reason
                );
                if !is_known_appendable_by_name {
                    self.response.headers.retain(|h| h.name() != *name);
                }
            }
        };

        if let TypedHeader::Other(name, value) = &header {
            if *name == HeaderName::ReferTo {
                if let HeaderValue::ReferTo(refer_to_val) = value {
                    let typed_refer_to = TypedHeader::ReferTo(refer_to_val.clone());
                    self.response.headers.push(typed_refer_to);
                    return self;
                }
            }
            // Add other similar conversions here if needed
        }

        self.response.headers.push(header);
        self
    }
    
    /// Add body content and update Content-Length
    ///
    /// Adds a message body to the response and automatically sets the Content-Length header
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
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::StatusCode;
    /// use rvoip_sip_core::sdp::SdpBuilder;
    /// use rvoip_sip_core::sdp::attributes::MediaDirection;
    ///
    /// // Create an SDP body using the SdpBuilder pattern
    /// let sdp_body = SdpBuilder::new("Session")
    ///     .origin("bob", "2890844527", "2890844527", "IN", "IP4", "127.0.0.1")
    ///     .connection("IN", "IP4", "127.0.0.1")
    ///     .time("0", "0")
    ///     .media_audio(49172, "RTP/AVP")
    ///         .formats(&["0"])
    ///         .rtpmap("0", "PCMU/8000")
    ///         .done()
    ///     .build()
    ///     .expect("Valid SDP");
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .content_type("application/sdp")
    ///     .body(sdp_body.to_string());
    /// ```
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.response = self.response.with_body(body);
        self
    }
    
    /// Build the final Response
    ///
    /// Finalizes the response construction and returns the complete SIP response.
    ///
    /// # Returns
    /// The constructed [`Response`][`crate::types::sip_response::Response`]
    ///
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// use rvoip_sip_core::types::{StatusCode, Method};
    ///
    /// let response = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
    ///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
    ///     .call_id("a84b4c76e66710")
    ///     .cseq(1, Method::Invite)
    ///     .via("example.com", "UDP", Some("z9hG4bK776asdhds"))
    ///     .build();
    /// ```
    pub fn build(self) -> Response {
        self.response
    }
}