//! # SIP Message Types
//!
//! This module provides the core SIP message types: [`Request`], [`Response`], and [`Message`].
//!
//! These types form the foundation of the SIP protocol implementation, allowing you to:
//! - Parse incoming SIP messages
//! - Create and modify SIP requests and responses
//! - Access and manipulate headers and message bodies
//! - Serialize messages for transmission
//!
//! ## Examples
//!
//! ### Creating a SIP request
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
//! ```
//!
//! ### Creating a SIP response
//!
//! ```rust
//! use rvoip_sip_core::prelude::*;
//!
//! // Create a SIP 200 OK response
//! let response = Response::new(StatusCode::Ok)
//!     .with_header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", "sip:alice@example.com".parse().unwrap()))))
//!     .with_header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", "sip:bob@example.com".parse().unwrap()))))
//!     .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.com")))
//!     .with_header(TypedHeader::CSeq(CSeq::new(314159, Method::Invite)));
//! ```

use std::collections::HashMap;
use std::fmt;
use bytes::Bytes;
use serde::{Deserialize, Serialize}; // Uncommented Serde

use crate::types::header::{Header, HeaderName, TypedHeader, TypedHeaderTrait};
use crate::types::uri::Uri;
use crate::types::version::Version;
// Use types from the current crate's types module
use crate::types::method::Method;
use crate::types::StatusCode;
use crate::error::{Error, Result};
// Use types from the current crate's types module
use crate::types; // Add import
use crate::types::multipart::{MultipartBody, MimePart, ParsedBody};
use crate::types::sdp::SdpSession; // Assuming SdpSession is in types::sdp
use crate::types::via::{Via, ViaHeader};

/// A SIP request message
///
/// Represents a SIP request sent from a client to a server, containing a method,
/// target URI, headers, and optional body.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a basic INVITE request
/// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
///     .with_header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", "sip:alice@example.com".parse().unwrap()))))
///     .with_header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", "sip:bob@example.com".parse().unwrap()))));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The method of the request
    pub method: Method,
    /// The request URI
    pub uri: Uri,
    /// The SIP version
    pub version: Version,
    /// The headers of the request (now typed)
    pub headers: Vec<TypedHeader>,
    /// The body of the request
    pub body: Bytes,
}

impl Request {
    /// Creates a new SIP request with the specified method and URI
    ///
    /// This initializes a request with SIP/2.0 version, empty headers, and empty body.
    ///
    /// # Parameters
    /// - `method`: The SIP method (INVITE, ACK, BYE, etc.)
    /// - `uri`: The target URI (e.g., "sip:user@example.com")
    ///
    /// # Returns
    /// A new `Request` instance
    pub fn new(method: Method, uri: Uri) -> Self {
        Request {
            method,
            uri,
            version: Version::sip_2_0(),
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    /// Adds a typed header to the request
    ///
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets all headers from a Vec<TypedHeader> (used by parser)
    ///
    /// # Parameters
    /// - `headers`: Vector of typed headers to set
    pub fn set_headers(&mut self, headers: Vec<TypedHeader>) {
        self.headers = headers;
    }

    /// Sets the body of the request
    ///
    /// # Parameters
    /// - `body`: The request body
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Retrieves the first typed header with the specified name, if any
    ///
    /// # Parameters
    /// - `name`: The header name to look for
    ///
    /// # Returns
    /// An optional reference to the first matching header
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    /// Returns the method of the request
    pub fn method(&self) -> Method {
        self.method.clone()
    }
    
    /// Returns the URI of the request
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
    
    /// Retrieves a strongly-typed header value
    ///
    /// # Type Parameters
    /// - `T`: The expected header type
    ///
    /// # Returns
    /// The typed header if found and correctly typed, or None
    pub fn typed_header<T: TypedHeaderTrait>(&self) -> Option<&T> {
        for header in &self.headers {
            if let Some(typed) = try_as_typed_header::<T>(header) {
                return Some(typed);
            }
        }
        None
    }

    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&types::CallId> {
        self.header(&HeaderName::CallId).and_then(|h| 
            if let TypedHeader::CallId(cid) = h { Some(cid) } else { None }
        )
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        None // Placeholder
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        None // Placeholder
    }
    
    /// Retrieves the CSeq header value, if present
    pub fn cseq(&self) -> Option<&str> {
        None // Placeholder
    }

    /// Get all Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the request
    pub fn via_headers(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in &self.headers {
            // Directly match the TypedHeader::Via variant
            if let TypedHeader::Via(via_data) = header {
                // Via is already a Vec<ViaHeader> wrapper
                result.push(via_data.clone());
            }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    ///
    /// # Returns
    /// The first Via header if present, or None
    pub fn first_via(&self) -> Option<Via> {
        self.headers.iter().find_map(|h| {
             if let TypedHeader::Via(via_data) = h {
                 Some(via_data.clone())
             } else {
                 None
             }
        })
    }

    pub fn get_header_value(&self, name: &HeaderName) -> Option<&str> {
        None // Placeholder
    }

    /// Get Via headers as structured Via objects
    /// Note: This relies on the Via parser being available where called.
    /// TODO: Refactor to return `Result<Vec<Via>>` or use a dedicated typed header getter.
    pub fn via_headers_no_body(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in &self.headers {
            // Directly match the TypedHeader::Via variant (similar to via_headers)
            if let TypedHeader::Via(via_data) = header {
                 // Via is already a Vec<ViaHeader> wrapper
                 result.push(via_data.clone());
            }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    /// TODO: Refactor similar to via_headers.
    pub fn first_via_no_body(&self) -> Option<Via> {
        self.headers.iter().find_map(|h| {
             if let TypedHeader::Via(via_data) = h {
                 Some(via_data.clone())
             } else {
                 None
             }
        })
    }

    /// Convert the message to bytes without including the body
    ///
    /// # Returns
    /// A vector of bytes containing the request line and headers
    pub fn to_bytes_no_body(&self) -> Vec<u8> {
        let mut buffer = Vec::new();
        
        // Add request line: METHOD URI SIP/2.0\r\n
        buffer.extend_from_slice(format!("{} {} {}\r\n", 
            self.method, self.uri, self.version).as_bytes());
        
        // Add headers
        for header in &self.headers {
            buffer.extend_from_slice(format!("{}\r\n", header).as_bytes());
        }
        
        // Add empty line to separate headers from body
        buffer.extend_from_slice(b"\r\n");
        
        buffer
    }

    // Note: The parse method is intentionally omitted here.
    // Parsing should be handled by the parser module.
    // pub fn parse(data: &[u8]) -> Result<Self> { ... }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Request line
        write!(f, "{} {} {}\r\n", self.method, self.uri, self.version)?;
        
        // Headers
        for header in &self.headers {
            write!(f, "{}\r\n", header)?;
        }
        
        // Blank line and body (if any)
        write!(f, "\r\n")?;
        if !self.body.is_empty() {
            write!(f, "{}", String::from_utf8_lossy(&self.body))?;
        }
        
        Ok(())
    }
}

/// A SIP response message
///
/// Represents a SIP response sent from a server to a client, containing
/// a status code, reason phrase, headers, and optional body.
///
/// # Examples
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// // Create a 200 OK response
/// let response = Response::new(StatusCode::Ok)
///     .with_header(TypedHeader::From(From::new(Address::new_with_display_name("Alice", "sip:alice@example.com".parse().unwrap()))))
///     .with_header(TypedHeader::To(To::new(Address::new_with_display_name("Bob", "sip:bob@example.com".parse().unwrap()))));
///
/// // Or use a convenience method for common responses
/// let trying = Response::trying();
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The SIP version
    pub version: Version,
    /// The status code
    pub status: StatusCode,
    /// Custom reason phrase (overrides the default for the status code)
    pub reason: Option<String>,
    /// The headers of the response (now typed)
    pub headers: Vec<TypedHeader>,
    /// The body of the response
    pub body: Bytes,
}

impl Response {
    /// Creates a new SIP response with the specified status code
    ///
    /// This initializes a response with SIP/2.0 version, the given status code,
    /// default reason phrase, empty headers, and empty body.
    ///
    /// # Parameters
    /// - `status`: The SIP status code
    ///
    /// # Returns
    /// A new `Response` instance
    pub fn new(status: StatusCode) -> Self {
        Response {
            version: Version::sip_2_0(),
            status,
            reason: None,
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    /// Creates a SIP 100 Trying response
    ///
    /// # Returns
    /// A new `Response` with 100 Trying status
    pub fn trying() -> Self {
        Response::new(StatusCode::Trying)
    }

    /// Creates a SIP 180 Ringing response
    ///
    /// # Returns
    /// A new `Response` with 180 Ringing status
    pub fn ringing() -> Self {
        Response::new(StatusCode::Ringing)
    }

    /// Creates a SIP 200 OK response
    ///
    /// # Returns
    /// A new `Response` with 200 OK status
    pub fn ok() -> Self {
        Response::new(StatusCode::Ok)
    }

    /// Adds a typed header to the response
    ///
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets all headers from a Vec<TypedHeader> (used by parser)
    ///
    /// # Parameters
    /// - `headers`: Vector of typed headers to set
    pub fn set_headers(&mut self, headers: Vec<TypedHeader>) {
        self.headers = headers;
    }

    /// Sets a custom reason phrase for the response
    ///
    /// # Parameters
    /// - `reason`: The custom reason phrase
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the body of the response
    ///
    /// # Parameters
    /// - `body`: The response body
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Retrieves the first typed header with the specified name, if any
    ///
    /// # Parameters
    /// - `name`: The header name to look for
    ///
    /// # Returns
    /// An optional reference to the first matching header
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    /// Gets the reason phrase for this response (either the custom one or the default)
    ///
    /// # Returns
    /// The reason phrase as a string slice
    pub fn reason_phrase(&self) -> &str {
        self.reason.as_deref().unwrap_or_else(|| self.status.reason_phrase())
    }
    
    /// Retrieves a strongly-typed header value
    ///
    /// # Type Parameters
    /// - `T`: The expected header type
    ///
    /// # Returns
    /// The typed header if found and correctly typed, or None
    pub fn typed_header<T: TypedHeaderTrait>(&self) -> Option<&T> {
        for header in &self.headers {
            if let Some(typed) = try_as_typed_header::<T>(header) {
                return Some(typed);
            }
        }
        None
    }
    
    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&types::CallId> {
        self.header(&HeaderName::CallId).and_then(|h| 
            if let TypedHeader::CallId(cid) = h { Some(cid) } else { None }
        )
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        None // Placeholder
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        None // Placeholder
    }

    /// Get all Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the response
    pub fn via_headers(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in &self.headers {
            // Directly match the TypedHeader::Via variant
            if let TypedHeader::Via(via_data) = header {
                // Via is already a Vec<ViaHeader> wrapper
                result.push(via_data.clone());
            }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    ///
    /// # Returns
    /// The first Via header if present, or None
    pub fn first_via(&self) -> Option<Via> {
        self.headers.iter().find_map(|h| {
             if let TypedHeader::Via(via_data) = h {
                 Some(via_data.clone())
             } else {
                 None
             }
        })
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Status line
        write!(
            f, 
            "{} {} {}\r\n", 
            self.version, 
            self.status.as_u16(), 
            self.reason_phrase()
        )?;
        
        // Headers
        for header in &self.headers {
            write!(f, "{}\r\n", header)?;
        }
        
        // Blank line and body (if any)
        write!(f, "\r\n")?;
        if !self.body.is_empty() {
            write!(f, "{}", String::from_utf8_lossy(&self.body))?;
        }
        
        Ok(())
    }
}

/// Represents either a SIP request or response
///
/// This enum provides a unified interface for working with SIP messages,
/// allowing code to handle both requests and responses through a common API.
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
    pub fn is_request(&self) -> bool {
        matches!(self, Message::Request(_))
    }

    /// Returns true if this message is a response
    pub fn is_response(&self) -> bool {
        matches!(self, Message::Response(_))
    }

    /// Returns the request if this is a request message, None otherwise
    pub fn as_request(&self) -> Option<&Request> {
        match self {
            Message::Request(req) => Some(req),
            _ => None,
        }
    }

    /// Returns the response if this is a response message, None otherwise
    pub fn as_response(&self) -> Option<&Response> {
        match self {
            Message::Response(resp) => Some(resp),
            _ => None,
        }
    }

    /// Returns the method if this is a request, None otherwise
    pub fn method(&self) -> Option<Method> {
        self.as_request().map(|req| req.method.clone())
    }

    /// Returns the status if this is a response, None otherwise
    pub fn status(&self) -> Option<StatusCode> {
        self.as_response().map(|resp| resp.status)
    }

    /// Returns the headers of the message
    pub fn headers(&self) -> &[TypedHeader] {
        match self {
            Message::Request(req) => &req.headers,
            Message::Response(resp) => &resp.headers,
        }
    }

    /// Returns the body of the message
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
    pub fn typed_header<T: TypedHeaderTrait>(&self) -> Option<&T> {
        for header in self.headers() {
            if let Some(typed) = try_as_typed_header::<T>(header) {
                return Some(typed);
            }
        }
        None
    }

    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&types::CallId> {
        self.header(&HeaderName::CallId).and_then(|h| 
            if let TypedHeader::CallId(cid) = h { Some(cid) } else { None }
        )
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        None // Placeholder
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        None // Placeholder
    }

    /// Get Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the message
    pub fn via_headers(&self) -> Vec<Via> {
        match self {
            Message::Request(req) => req.via_headers(),
            Message::Response(resp) => resp.via_headers(),
        }
    }

    /// Get the first Via header as a structured Via object
    ///
    /// # Returns
    /// The first Via header if present, or None
    pub fn first_via(&self) -> Option<Via> {
        match self {
            Message::Request(req) => req.first_via(),
            Message::Response(resp) => resp.first_via(),
        }
    }

    /// Convert the message to bytes
    ///
    /// # Returns
    /// A vector of bytes containing the complete serialized message
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
impl From<Request> for Message {
    fn from(req: Request) -> Self {
        Message::Request(req)
    }
}

// Implement From<Response> for Message
impl From<Response> for Message {
    fn from(resp: Response) -> Self {
        Message::Response(resp)
    }
}

// Helper function to try casting a TypedHeader to a specific type
fn try_as_typed_header<T: TypedHeaderTrait>(header: &TypedHeader) -> Option<&T> {
    if header.name() == T::header_name().into() {
        // This is unsafe, but necessary for downcasting
        // The safety is maintained by checking the header name first
        unsafe {
            let ptr = header as *const TypedHeader;
            let ptr_any = ptr as *const dyn std::any::Any;
            let ptr_t = ptr_any as *const T;
            return Some(&*ptr_t);
        }
    }
    None
} 