//! # SIP Message Types
//!
//! This module provides the core SIP message type: [`Message`], which can be either a [`Request`] or [`Response`].
//!
//! The Message type forms the foundation of the SIP protocol implementation, allowing you to:
//! - Parse incoming SIP messages
//! - Create and modify SIP requests and responses
//! - Access and manipulate headers and message bodies
//! - Serialize messages for transmission
//!
//! ## Message Structure
//! 
//! A SIP message consists of:
//! - A start line (either a request line or a status line)
//! - Headers (metadata about the message)
//! - An empty line (signaling the end of headers)
//! - An optional message body
//!
//! ## Examples
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//! use bytes::Bytes;
//!
//! // Create a basic SIP INVITE request
//! let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
//!     .with_header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", "sip:alice@example.com".parse().unwrap()))))
//!     .with_header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", "sip:bob@example.com".parse().unwrap()))))
//!     .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
//!     .with_header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)))
//!     .with_body(Bytes::from("SDP body content here"));
//!
//! // Convert to a Message enum for unified handling
//! let message: Message = request.into();
//!
//! // Working with a response
//! let response = Response::new(StatusCode::Ok)
//!     .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")));
//! let message: Message = response.into();
//! ```

use std::fmt;
use std::collections::HashSet;
use std::str::FromStr;
use std::convert::From as StdFrom;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::types::header::{HeaderName, TypedHeader, TypedHeaderTrait};
use crate::types::method::Method;
use crate::types::StatusCode;
use crate::types;
use crate::types::uri::{Uri, Host, Scheme};
use crate::types::via::Via;
use crate::types::headers::HeaderAccess;
// Import the request and response types directly
use crate::types::sip_request::Request;
use crate::types::sip_response::Response;
use crate::types::to::To;
use crate::types::from::From;

/// Represents either a SIP request or response
///
/// This enum provides a unified interface for working with SIP messages,
/// allowing code to handle both requests and responses through a common API.
/// The SIP protocol uses a request-response model where clients send requests
/// and servers respond with responses, both represented by this type.
///
/// # Standard RFC Compliance
///
/// This implementation follows [RFC 3261](https://tools.ietf.org/html/rfc3261), 
/// which defines the Session Initiation Protocol.
///
/// # Type Parameters
///
/// - `Request`: A SIP request message (e.g., INVITE, REGISTER)
/// - `Response`: A SIP response message (e.g., 200 OK, 404 Not Found)
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
/// use bytes::Bytes;
///
/// // Parse a raw message
/// let data = Bytes::from("SIP/2.0 200 OK\r\n\r\n");
/// let message = parse_message(&data).unwrap();
///
/// // Check message type and access contents
/// match &message {
///     Message::Request(req) => {
///         println!("Request method: {}", req.method);
///     },
///     Message::Response(resp) => {
///         println!("Response status: {}", resp.status);
///     }
/// }
///
/// // Or use helper methods
/// if message.is_response() {
///     if let Some(status) = message.status() {
///         println!("Status code: {}", status);
///     }
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// SIP request
    Request(Request),
    /// SIP response
    Response(Response),
}

impl Message {
    /// Returns true if this message is a request
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap());
    /// let message: Message = request.into();
    ///
    /// assert!(message.is_request());
    /// assert!(!message.is_response());
    /// ```
    pub fn is_request(&self) -> bool {
        matches!(self, Message::Request(_))
    }

    /// Returns true if this message is a response
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok);
    /// let message: Message = response.into();
    ///
    /// assert!(message.is_response());
    /// assert!(!message.is_request());
    /// ```
    pub fn is_response(&self) -> bool {
        matches!(self, Message::Response(_))
    }

    /// Returns the request if this is a request message, None otherwise
    ///
    /// # Returns
    /// - `Some(&Request)` if this is a request message
    /// - `None` if this is a response message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap());
    /// let message: Message = request.into();
    ///
    /// assert!(message.as_request().is_some());
    /// assert!(message.as_response().is_none());
    /// ```
    pub fn as_request(&self) -> Option<&Request> {
        match self {
            Message::Request(req) => Some(req),
            _ => None,
        }
    }

    /// Returns the response if this is a response message, None otherwise
    ///
    /// # Returns
    /// - `Some(&Response)` if this is a response message
    /// - `None` if this is a request message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok);
    /// let message: Message = response.into();
    ///
    /// assert!(message.as_response().is_some());
    /// assert!(message.as_request().is_none());
    /// ```
    pub fn as_response(&self) -> Option<&Response> {
        match self {
            Message::Response(resp) => Some(resp),
            _ => None,
        }
    }

    /// Returns the method if this is a request, None otherwise
    ///
    /// # Returns
    /// - `Some(Method)` if this is a request message (clone of the method)
    /// - `None` if this is a response message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap());
    /// let message: Message = request.into();
    ///
    /// assert_eq!(message.method(), Some(Method::Invite));
    /// ```
    pub fn method(&self) -> Option<Method> {
        self.as_request().map(|req| req.method.clone())
    }

    /// Returns the status if this is a response, None otherwise
    ///
    /// # Returns
    /// - `Some(StatusCode)` if this is a response message
    /// - `None` if this is a request message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok);
    /// let message: Message = response.into();
    ///
    /// assert_eq!(message.status(), Some(StatusCode::Ok));
    /// ```
    pub fn status(&self) -> Option<StatusCode> {
        self.as_response().map(|resp| resp.status)
    }

    /// Returns the headers of the message
    ///
    /// # Returns
    /// A slice of all TypedHeader objects in the message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")));
    /// let message: Message = response.into();
    ///
    /// assert_eq!(message.headers().len(), 1);
    /// ```
    pub fn headers(&self) -> &[TypedHeader] {
        match self {
            Message::Request(req) => &req.headers,
            Message::Response(resp) => &resp.headers,
        }
    }

    /// Returns the body of the message
    ///
    /// # Returns
    /// A reference to the message body bytes
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use bytes::Bytes;
    ///
    /// let body = Bytes::from("test body");
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_body(body.clone());
    /// let message: Message = response.into();
    ///
    /// assert_eq!(message.body(), &body);
    /// ```
    pub fn body(&self) -> &Bytes {
        match self {
            Message::Request(req) => &req.body,
            Message::Response(resp) => &resp.body,
        }
    }

    /// Retrieves the first typed header with the specified name, if any
    ///
    /// # Parameters
    /// - `name`: The header name to look for
    ///
    /// # Returns
    /// An optional reference to the first matching header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")));
    /// let message: Message = response.into();
    ///
    /// let header = message.header(&HeaderName::CallId);
    /// assert!(header.is_some());
    /// ```
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers().iter().find(|h| h.name() == *name)
    }

    /// Retrieves a strongly-typed header value
    ///
    /// # Type Parameters
    /// - `T`: The expected header type
    ///
    /// # Returns
    /// The typed header if found and correctly typed, or None
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_id = CallId::new("abc123");
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(call_id.clone()));
    /// let message: Message = response.into();
    ///
    /// let retrieved = message.typed_header::<CallId>();
    /// assert!(retrieved.is_some());
    /// assert_eq!(retrieved.unwrap().value(), "abc123");
    /// ```
    pub fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> {
        // First check if the header name matches what we expect
        if let Some(h) = self.header(&T::header_name().into()) {
            // Use the centralized as_typed_ref method
            return h.as_typed_ref::<T>();
        }
        None
    }

    /// Retrieves the Call-ID header value, if present
    ///
    /// # Returns
    /// An optional reference to the CallId
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let call_id = CallId::new("abc123");
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(call_id.clone()));
    /// let message: Message = response.into();
    ///
    /// let retrieved = message.call_id();
    /// assert!(retrieved.is_some());
    /// assert_eq!(retrieved.unwrap().value(), "abc123");
    /// ```
    pub fn call_id(&self) -> Option<&types::CallId> {
        if let Some(h) = self.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(call_id) = h {
                return Some(call_id);
            }
        }
        None
    }
    
    /// Retrieves the From header value, if present
    ///
    /// # Returns
    /// An optional reference to the From header
    pub fn from(&self) -> Option<&From> {
        if let Some(h) = self.header(&HeaderName::From) {
            if let TypedHeader::From(from) = h {
                return Some(from);
            }
        }
        None
    }
    
    /// Retrieves the To header value, if present
    ///
    /// # Returns
    /// An optional reference to the To header
    pub fn to(&self) -> Option<&To> {
        if let Some(h) = self.header(&HeaderName::To) {
            if let TypedHeader::To(to) = h {
                return Some(to);
            }
        }
        None
    }

    /// Get Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new("UDP", "example.com", 5060);
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::Via(via.clone()));
    /// let message: Message = response.into();
    ///
    /// let vias = message.via_headers();
    /// assert_eq!(vias.len(), 1);
    /// ```
    pub fn via_headers(&self) -> Vec<Via> {
        let headers_vec = match self {
            Message::Request(req) => req.headers_by_name("Via"),
            Message::Response(resp) => resp.headers_by_name("Via"),
        };
        
        let mut vias = Vec::with_capacity(headers_vec.len());
        
        for h in headers_vec {
            if let TypedHeader::Via(via) = h {
                vias.push(via.clone());
            }
        }
        
        vias
    }

    /// Get the first Via header as a structured Via object
    ///
    /// # Returns
    /// The first Via header if present, or None
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new("UDP", "example.com", 5060);
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::Via(via.clone()));
    /// let message: Message = response.into();
    ///
    /// let first_via = message.first_via();
    /// assert!(first_via.is_some());
    /// ```
    pub fn first_via(&self) -> Option<Via> {
        if let Some(h) = self.header(&HeaderName::Via) {
            if let TypedHeader::Via(via) = h {
                return Some(via.clone());
            }
        }
        None
    }

    /// Convert the message to bytes
    ///
    /// Serializes the message into a binary representation following the SIP protocol
    /// specification, ready for transmission over the network.
    ///
    /// # Returns
    /// A vector of bytes containing the complete serialized message
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok);
    /// let message: Message = response.into();
    ///
    /// let bytes = message.to_bytes();
    /// assert!(!bytes.is_empty());
    /// assert!(String::from_utf8_lossy(&bytes).contains("SIP/2.0 200 OK"));
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        
        match self {
            Message::Request(request) => {
                // Add request line: METHOD URI SIP/2.0\r\n
                bytes.extend_from_slice(format!("{} {} {}\r\n", 
                    request.method, request.uri, request.version).as_bytes());
                
                // Add headers
                for header in &request.headers {
                    bytes.extend_from_slice(format!("{}\r\n", header).as_bytes());
                }
                
                // Add empty line to separate headers from body
                bytes.extend_from_slice(b"\r\n");
                
                // Add body if any
                bytes.extend_from_slice(&request.body);
            },
            Message::Response(response) => {
                // Add status line: SIP/2.0 CODE REASON\r\n
                bytes.extend_from_slice(format!("{} {} {}\r\n", 
                    response.version, 
                    response.status.as_u16(), 
                    response.reason_phrase()).as_bytes());
                
                // Add headers
                for header in &response.headers {
                    bytes.extend_from_slice(format!("{}\r\n", header).as_bytes());
                }
                
                // Add empty line to separate headers from body
                bytes.extend_from_slice(b"\r\n");
                
                // Add body if any
                bytes.extend_from_slice(&response.body);
            }
        }
        
        bytes
    }

    // Note: The parse method is intentionally omitted here.
    // Parsing should be handled by the parser module.
    // pub fn parse(data: &[u8]) -> Result<Self> { ... }

    /// Add a typed header to the message
    ///
    /// This is a convenience method that updates the appropriate header
    /// collection based on whether this is a Request or Response.
    ///
    /// # Parameters
    ///
    /// - `header`: The typed header to add
    ///
    /// # Returns
    ///
    /// The updated message with the header added
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Add a header to a request message
    /// let request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
    /// let message = Message::from(request)
    ///     .with_header(TypedHeader::CallId(CallId::new("test-call-id")));
    /// ```
    pub fn with_header(self, header: TypedHeader) -> Self {
        match self {
            Message::Request(mut req) => {
                req.headers.push(header);
                Message::Request(req)
            },
            Message::Response(mut resp) => {
                resp.headers.push(header);
                Message::Response(resp)
            }
        }
    }
    
    /// Create a new SIP Message (request)
    ///
    /// This is a convenience method that creates a new Message with a Request.
    /// For a response, use Message::from(Response::new(...))
    ///
    /// # Returns
    ///
    /// A new Message containing a minimal valid request
    pub fn new() -> Self {
        Message::Request(Request::new(Method::Invite, Uri::sip("example.com")))
    }

    /// Sets the body of the message.
    ///
    /// This method automatically sets the Content-Length header to match the body size.
    ///
    /// # Parameters
    ///
    /// - `body`: The body content as bytes or a string-like type
    ///
    /// # Returns
    ///
    /// The updated message with the body set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a message with an SDP body
    /// let msg = Message::new()
    ///     .with_header(TypedHeader::From(From::new("Alice", "sip:alice@example.com").with_tag("1928301774")))
    ///     .with_header(TypedHeader::To(To::new("Bob", "sip:bob@example.com")))
    ///     .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.example.com")))
    ///     .with_header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
    ///     .with_header(TypedHeader::Via(Via::new_simple("UDP", "example.com", 5060).expect("Failed to create Via")))
    ///     .with_body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n");
    ///
    /// // The Content-Length header should be automatically set
    /// let content_length = msg.content_length();
    /// assert!(content_length.is_some());
    /// ```
    pub fn with_body(self, body: impl Into<Bytes>) -> Self {
        match self {
            Message::Request(req) => Message::Request(req.with_body(body)),
            Message::Response(resp) => Message::Response(resp.with_body(body)),
        }
    }
    
    /// Returns the content length of the message, if present.
    pub fn content_length(&self) -> Option<u64> {
        if let Some(h) = self.header(&HeaderName::ContentLength) {
            if let TypedHeader::ContentLength(cl) = h {
                return Some(cl.0 as u64);
            }
        }
        None
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Request(req) => write!(f, "{}", req),
            Message::Response(resp) => write!(f, "{}", resp),
        }
    }
}

// Implement From<Request> for Message
impl StdFrom<Request> for Message {
    fn from(req: Request) -> Self {
        Message::Request(req)
    }
}

// Implement From<Response> for Message
impl StdFrom<Response> for Message {
    fn from(resp: Response) -> Self {
        Message::Response(resp)
    }
}

// Implement HeaderAccess for Message
impl HeaderAccess for Message {
    fn typed_headers<T: TypedHeaderTrait + 'static>(&self) -> Vec<&T> {
        use crate::types::headers::collect_typed_headers;
        
        // Get the headers from the message
        let headers = match self {
            Message::Request(req) => &req.headers,
            Message::Response(resp) => &resp.headers,
        };
        
        // Use our safer collect_typed_headers implementation
        collect_typed_headers::<T>(headers)
    }

    fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> {
        self.typed_headers::<T>().into_iter().next()
    }

    fn headers(&self, name: &HeaderName) -> Vec<&TypedHeader> {
        match self {
            Message::Request(req) => req.headers(name),
            Message::Response(resp) => resp.headers(name),
        }
    }

    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        match self {
            Message::Request(req) => req.header(name),
            Message::Response(resp) => resp.header(name),
        }
    }

    fn headers_by_name(&self, name: &str) -> Vec<&TypedHeader> {
        match self {
            Message::Request(req) => req.headers_by_name(name),
            Message::Response(resp) => resp.headers_by_name(name),
        }
    }

    fn raw_header_value(&self, name: &HeaderName) -> Option<String> {
        match self {
            Message::Request(req) => req.raw_header_value(name),
            Message::Response(resp) => resp.raw_header_value(name),
        }
    }

    fn raw_headers(&self, name: &HeaderName) -> Vec<Vec<u8>> {
        match self {
            Message::Request(req) => req.raw_headers(name),
            Message::Response(resp) => resp.raw_headers(name),
        }
    }

    fn header_names(&self) -> Vec<HeaderName> {
        match self {
            Message::Request(req) => req.header_names(),
            Message::Response(resp) => resp.header_names(),
        }
    }

    fn has_header(&self, name: &HeaderName) -> bool {
        match self {
            Message::Request(req) => req.has_header(name),
            Message::Response(resp) => resp.has_header(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::types::headers::HeaderAccess;

    #[test]
    fn test_message_from_request() {
        let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(types::CallId::new("test-call-id")));
        
        let message: Message = request.clone().into();
        
        assert!(message.is_request());
        assert!(!message.is_response());
        assert_eq!(message.method(), Some(Method::Invite));
        assert_eq!(message.status(), None);
        
        let request_ref = message.as_request().unwrap();
        assert_eq!(request_ref.method, request.method);
        assert_eq!(request_ref.uri, request.uri);
        assert_eq!(request_ref.headers.len(), request.headers.len());
    }

    #[test]
    fn test_message_from_response() {
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(types::CallId::new("test-call-id")));
        
        let message: Message = response.clone().into();
        
        assert!(!message.is_request());
        assert!(message.is_response());
        assert_eq!(message.method(), None);
        assert_eq!(message.status(), Some(StatusCode::Ok));
        
        let response_ref = message.as_response().unwrap();
        assert_eq!(response_ref.status, response.status);
        assert_eq!(response_ref.headers.len(), response.headers.len());
    }

    #[test]
    fn test_header_access() {
        let call_id = types::CallId::new("test-call-id");
        let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let message: Message = request.into();
        
        // Test direct header access
        let header = message.header(&HeaderName::CallId);
        assert!(header.is_some());
        
        // Test for multiple headers with the same name
        let call_id_headers = message.headers_by_name("Call-ID");
        assert_eq!(call_id_headers.len(), 1);
        
        // We know typed_header will return None in the current implementation,
        // so we won't test it
        
        // Get all headers
        let all_headers = message.headers();
        assert_eq!(all_headers.len(), 1);
        
        // Test call_id convenience method
        let call_id_header = message.call_id();
        assert!(call_id_header.is_some());
        assert_eq!(call_id_header.unwrap().value(), call_id.value());
    }

    #[test]
    fn test_via_headers() {
        let via = Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via");
        let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap())
            .with_header(TypedHeader::Via(via.clone()));
        
        let message: Message = request.into();
        
        let vias = message.via_headers();
        assert_eq!(vias.len(), 1);
        
        let first_via = message.first_via();
        assert!(first_via.is_some());
    }

    #[test]
    fn test_to_bytes() {
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(types::CallId::new("test-call-id")))
            .with_body(Bytes::from("test body"));
        
        let message: Message = response.into();
        
        let bytes = message.to_bytes();
        let content = String::from_utf8_lossy(&bytes);
        
        assert!(content.contains("SIP/2.0 200 OK"));
        assert!(content.contains("Call-ID: test-call-id"));
        assert!(content.contains("test body"));
    }

    #[test]
    fn test_headeraccess_trait() {
        let call_id = types::CallId::new("test-call-id");
        let request = Request::new(Method::Invite, "sip:alice@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let message: Message = request.into();
        
        // Test HeaderAccess implementation - only test methods that don't rely on typed_header
        assert!(message.has_header(&HeaderName::CallId));
        assert!(!message.has_header(&HeaderName::To));
        
        let headers = message.headers_by_name("Call-ID");
        assert_eq!(headers.len(), 1);
        
        let names = message.header_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&HeaderName::CallId));
        
        let value = message.raw_header_value(&HeaderName::CallId);
        assert!(value.is_some());
        assert!(value.unwrap().contains("test-call-id"));
    }

    #[test]
    fn test_message_creation() {
        use crate::types::from::From;
        use crate::types::to::To;
        use crate::types::CallId;
        use crate::types::CSeq;
        
        let address = types::Address::new("sip:alice@example.com".parse().unwrap());
        
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::From(From::new(address)))
            .with_header(TypedHeader::To(To::new(types::Address::new("sip:bob@example.com".parse().unwrap()))))
            .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.example.com")))
            .with_header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .with_header(TypedHeader::Via(Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via")))
            .with_body("Hello World".to_string());
        
        let message: Message = request.into();
        
        // Now we should be able to successfully get typed headers
        assert!(message.typed_header::<CallId>().is_some());
        let call_id = message.typed_header::<CallId>().unwrap();
        assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.example.com");
        
        // Check the From header
        assert!(message.typed_header::<From>().is_some());
        let from = message.typed_header::<From>().unwrap();
        assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
        
        // Check CSeq
        assert!(message.typed_header::<CSeq>().is_some());
        let cseq = message.typed_header::<CSeq>().unwrap();
        assert_eq!(cseq.sequence(), 1);
        assert_eq!(*cseq.method(), Method::Invite);
        
        // Check body
        assert_eq!(message.body(), "Hello World");
    }
} 