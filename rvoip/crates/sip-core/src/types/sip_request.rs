//! # SIP Request Message
//!
//! This module provides the SIP Request message type.
//!
//! The Request struct represents SIP request messages sent from clients to servers,
//! containing a method, target URI, headers, and optional body.
//!
//! ## Request Structure
//!
//! A SIP request consists of:
//! - A request-line (containing the method, request URI, and SIP version)
//! - A set of headers providing metadata
//! - An empty line separating headers from the body
//! - An optional message body
//!
//! ## RFC Compliance
//!
//! This implementation follows [RFC 3261](https://tools.ietf.org/html/rfc3261),
//! which defines the Session Initiation Protocol.
//!
//! ## Common Request Methods
//!
//! - `INVITE`: Initiates a session
//! - `ACK`: Acknowledges final responses to INVITE
//! - `BYE`: Terminates a session
//! - `CANCEL`: Cancels a pending request
//! - `REGISTER`: Registers contact information
//! - `OPTIONS`: Queries capabilities
//! - `REFER`: Asks recipient to issue request
//! - `SUBSCRIBE`: Requests notification of an event
//! - `NOTIFY`: Provides information about an event
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
//! ```

use std::fmt;
use std::collections::HashSet;
use std::str::FromStr;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::types::header::{HeaderName, TypedHeader, TypedHeaderTrait};
use crate::types::uri::Uri;
use crate::types::version::Version;
use crate::types::method::Method;
use crate::types;
use crate::types::to::To;
use crate::types::from::From;
use crate::types::via::{Via, ViaHeader};
use crate::types::headers::HeaderAccess;
use crate::error::{Error, Result};
use crate::types::CallId;
use crate::types::CSeq;

/// A SIP request message
///
/// Represents a SIP request sent from a client to a server, containing a method,
/// target URI, headers, and optional body. SIP requests are used to initiate actions,
/// such as establishing a session, registering a device, or terminating a call.
///
/// # Standard RFC Compliance
///
/// This implementation follows [RFC 3261](https://tools.ietf.org/html/rfc3261), 
/// which defines the Session Initiation Protocol.
///
/// # Fields
///
/// - `method`: The SIP method (INVITE, ACK, BYE, etc.)
/// - `uri`: The target URI (e.g., "sip:user@example.com")
/// - `version`: The SIP protocol version (typically SIP/2.0)
/// - `headers`: A list of message headers
/// - `body`: The message body (optional)
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a REGISTER request
    /// let uri = "sip:registrar.example.com".parse().unwrap();
    /// let request = Request::new(Method::Register, uri);
    ///
    /// assert_eq!(request.method, Method::Register);
    /// assert!(request.headers.is_empty());
    /// assert!(request.body.is_empty());
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")))
    ///     .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
    ///
    /// assert_eq!(request.headers.len(), 2);
    /// ```
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets all headers from a Vec<TypedHeader> (used by parser)
    ///
    /// This method replaces all existing headers with the provided ones.
    ///
    /// # Parameters
    /// - `headers`: Vector of typed headers to set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let mut request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap());
    /// let headers = vec![
    ///     TypedHeader::CallId(CallId::new("abc123")),
    ///     TypedHeader::MaxForwards(MaxForwards::new(70))
    /// ];
    ///
    /// request.set_headers(headers);
    /// assert_eq!(request.headers.len(), 2);
    /// ```
    pub fn set_headers(&mut self, headers: Vec<TypedHeader>) {
        self.headers = headers;
    }

    /// Sets the body of the request
    ///
    /// This method also automatically adds or updates the Content-Length header.
    ///
    /// # Parameters
    /// - `body`: The body content
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        
        // Add or update Content-Length header
        let content_length = TypedHeader::ContentLength(types::content_length::ContentLength(self.body.len() as u32));
        
        // Remove any existing Content-Length headers
        self.headers.retain(|h| h.name() != HeaderName::ContentLength);
        
        // Add the new Content-Length header
        self.headers.push(content_length);
        
        self
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
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")));
    ///
    /// let header = request.header(&HeaderName::CallId);
    /// assert!(header.is_some());
    /// ```
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    /// Returns the method of the request
    ///
    /// # Returns
    /// A clone of the request's method
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap());
    /// assert_eq!(request.method(), Method::Invite);
    /// ```
    pub fn method(&self) -> Method {
        self.method.clone()
    }
    
    /// Returns the URI of the request
    ///
    /// # Returns
    /// A reference to the request's URI
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let uri: Uri = "sip:bob@example.com".parse().unwrap();
    /// let request = Request::new(Method::Invite, uri.clone());
    /// assert_eq!(request.uri(), &uri);
    /// ```
    pub fn uri(&self) -> &Uri {
        &self.uri
    }
    
    /// Retrieves the first header with the specified type, if any.
    pub fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> 
    where 
        <T as TypedHeaderTrait>::Name: std::fmt::Debug,
        T: std::fmt::Debug
    {
        for header in &self.headers {
            if let Some(typed) = try_as_typed_header::<T>(header) {
                return Some(typed);
            }
        }
        None
    }

    /// Get the Call-ID header value, if present
    ///
    /// # Returns
    /// An optional reference to the CallId
    pub fn call_id(&self) -> Option<&types::CallId> {
        if let Some(h) = self.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(cid) = h {
                return Some(cid);
            }
        }
        None
    }
    
    /// Retrieves the From header value, if present
    ///
    /// # Returns
    /// An optional reference to the From header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use std::str::FromStr;
    ///
    /// // Create a URI for the address
    /// let uri = Uri::from_str("sip:alice@atlanta.com").unwrap();
    /// // Create an address with display name
    /// let address = Address::new_with_display_name("Alice", uri);
    /// // Create a From header with the address
    /// let mut from = From::new(address);
    /// // Add a tag parameter
    /// from.set_tag("1928301774");
    /// 
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::From(from.clone()));
    ///
    /// let retrieved = request.from();
    /// assert!(retrieved.is_some());
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let to = To::new(Address::new_with_display_name("Bob", "sip:bob@example.com".parse().unwrap()));
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::To(to.clone()));
    ///
    /// let retrieved = request.to();
    /// assert!(retrieved.is_some());
    /// ```
    pub fn to(&self) -> Option<&To> {
        if let Some(h) = self.header(&HeaderName::To) {
            if let TypedHeader::To(to) = h {
                return Some(to);
            }
        }
        None
    }
    
    /// Retrieves the CSeq header value, if present
    ///
    /// # Returns
    /// An optional reference to the CSeq header
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let cseq = CSeq::new(1, Method::Invite);
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::CSeq(cseq.clone()));
    ///
    /// let retrieved = request.cseq();
    /// assert!(retrieved.is_some());
    /// assert_eq!(retrieved.unwrap().method().clone(), Method::Invite);
    /// ```
    pub fn cseq(&self) -> Option<&CSeq> {
        if let Some(h) = self.header(&HeaderName::CSeq) {
            if let TypedHeader::CSeq(cseq) = h {
                return Some(cseq);
            }
        }
        None
    }

    /// Get all Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the request
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", Some(5060),
    ///     vec![Param::branch("z9hG4bK123456")]
    /// ).unwrap();
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::Via(via.clone()));
    ///
    /// let vias = request.via_headers();
    /// assert_eq!(vias.len(), 1);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new(
    ///     "SIP", "2.0", "UDP",
    ///     "example.com", Some(5060),
    ///     vec![Param::branch("z9hG4bK123456")]
    /// ).unwrap();
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::Via(via.clone()));
    ///
    /// let first_via = request.first_via();
    /// assert!(first_via.is_some());
    /// ```
    pub fn first_via(&self) -> Option<Via> {
        self.headers.iter().find_map(|h| {
             if let TypedHeader::Via(via_data) = h {
                 Some(via_data.clone())
             } else {
                 None
             }
        })
    }

    /// Gets the value of a specific header by name
    ///
    /// # Parameters
    /// - `name`: The header name to look for
    ///
    /// # Returns
    /// An optional string slice containing the header value
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use bytes::Bytes;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")))
    ///     .with_body(Bytes::from("test body"));
    ///
    /// let bytes = request.to_bytes_no_body();
    /// let content = String::from_utf8_lossy(&bytes);
    /// assert!(content.contains("INVITE sip:bob@example.com SIP/2.0"));
    /// assert!(content.contains("Call-ID: abc123"));
    /// assert!(!content.contains("test body"));
    /// ```
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

    /// Returns a reference to the body content
    ///
    /// # Returns
    /// A slice reference to the message body bytes
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use bytes::Bytes;
    ///
    /// let body_content = "test body";
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_body(Bytes::from(body_content));
    ///
    /// assert_eq!(request.body(), body_content.as_bytes());
    /// ```
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Creates a new request with all essential headers for a valid SIP message
    ///
    /// This is a convenience method that creates a request with:
    /// - From header
    /// - To header
    /// - Call-ID header
    /// - CSeq header
    /// - Max-Forwards header (default: 70)
    ///
    /// # Parameters
    /// - `method`: The SIP method to use
    /// - `to_uri`: The URI of the recipient
    /// - `from_uri`: The URI of the sender
    /// - `call_id_value`: A call ID string
    /// - `cseq_num`: The CSeq sequence number
    ///
    /// # Returns
    /// A well-formed Request with all required headers
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use rvoip_sip_core::types::headers::HeaderAccess;
    ///
    /// let to_uri = "sip:bob@example.com".parse().unwrap();
    /// let from_uri = "sip:alice@example.com".parse().unwrap();
    ///
    /// let request = Request::new_with_essentials(
    ///     Method::Invite,
    ///     to_uri,
    ///     from_uri,
    ///     "abc123@example.com",
    ///     1
    /// );
    ///
    /// assert!(request.has_header(&HeaderName::From));
    /// assert!(request.has_header(&HeaderName::To));
    /// assert!(request.has_header(&HeaderName::CallId));
    /// assert!(request.has_header(&HeaderName::CSeq));
    /// assert!(request.has_header(&HeaderName::MaxForwards));
    /// ```
    pub fn new_with_essentials(
        method: Method,
        to_uri: Uri,
        from_uri: Uri,
        call_id_value: &str,
        cseq_num: u32,
    ) -> Self {
        let from_addr = types::Address::new(from_uri);
        let to_addr = types::Address::new(to_uri.clone());
        
        Request::new(method.clone(), to_uri)
            .with_header(TypedHeader::From(From::new(from_addr)))
            .with_header(TypedHeader::To(To::new(to_addr)))
            .with_header(TypedHeader::CallId(types::CallId::new(call_id_value)))
            .with_header(TypedHeader::CSeq(types::CSeq::new(cseq_num, method)))
            .with_header(TypedHeader::MaxForwards(types::MaxForwards::new(70)))
    }

    /// Returns the SIP version
    ///
    /// # Returns
    /// A clone of the request's SIP version
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap());
    /// assert_eq!(request.version(), Version::sip_2_0());
    /// ```
    pub fn version(&self) -> Version {
        self.version.clone()
    }
    
    /// Returns a reference to the request headers
    ///
    /// # Returns
    /// A slice of all TypedHeader objects in the request
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")));
    ///
    /// assert_eq!(request.all_headers().len(), 1);
    /// ```
    pub fn all_headers(&self) -> &[TypedHeader] {
        &self.headers
    }
    
    /// Returns a reference to the request body as Bytes
    ///
    /// # Returns
    /// A reference to the request body Bytes
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    /// use bytes::Bytes;
    ///
    /// let body_content = "test body";
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_body(Bytes::from(body_content));
    ///
    /// assert_eq!(request.body_bytes(), &Bytes::from(body_content));
    /// ```
    pub fn body_bytes(&self) -> &Bytes {
        &self.body
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Write request line: METHOD URI SIP/2.0
        write!(f, "{} {} {}", self.method, self.uri, self.version)?;
        
        // Write headers
        for header in &self.headers {
            write!(f, "\r\n{}", header)?;
        }
        
        // Write separator
        write!(f, "\r\n")?;
        
        // Write body if present
        if !self.body.is_empty() {
            write!(f, "\r\n")?;
            
            // Try to decode body as UTF-8, fall back to hex representation
            match std::str::from_utf8(&self.body) {
                Ok(body_str) => write!(f, "{}", body_str)?,
                Err(_) => {
                    // Limit to first 100 bytes for display
                    let display_len = std::cmp::min(self.body.len(), 100);
                    for b in &self.body[..display_len] {
                        write!(f, "{:02x}", b)?;
                    }
                    if self.body.len() > 100 {
                        write!(f, "... [truncated, {} bytes total]", self.body.len())?;
                    }
                }
            }
        }
        
        Ok(())
    }
}

// Implement HeaderAccess for Request
impl HeaderAccess for Request {
    fn typed_headers<T: TypedHeaderTrait + 'static>(&self) -> Vec<&T> 
    where 
        <T as TypedHeaderTrait>::Name: std::fmt::Debug,
        T: std::fmt::Debug
    {
        use crate::types::headers::collect_typed_headers;
        collect_typed_headers::<T>(&self.headers)
    }

    fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> 
    where 
        <T as TypedHeaderTrait>::Name: std::fmt::Debug,
        T: std::fmt::Debug
    {
        self.typed_headers::<T>().into_iter().next()
    }

    fn headers(&self, name: &HeaderName) -> Vec<&TypedHeader> {
        self.headers.iter()
            .filter(|h| h.name() == *name)
            .collect()
    }

    fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    fn headers_by_name(&self, name: &str) -> Vec<&TypedHeader> {
        match HeaderName::from_str(name) {
            Ok(header_name) => self.headers(&header_name),
            Err(_) => Vec::new(), // Return empty vec for invalid names
        }
    }

    fn raw_header_value(&self, name: &HeaderName) -> Option<String> {
        self.header(name).and_then(|h| {
            match h.to_string().split_once(':') {
                Some((_, value)) => Some(value.trim().to_string()),
                None => None,
            }
        })
    }

    fn raw_headers(&self, name: &HeaderName) -> Vec<Vec<u8>> {
        self.headers(name)
            .iter()
            .filter_map(|h| {
                match h.to_string().split_once(':') {
                    Some((_, value)) => Some(value.trim().as_bytes().to_vec()),
                    None => None,
                }
            })
            .collect()
    }

    fn header_names(&self) -> Vec<HeaderName> {
        let mut names = HashSet::new();
        for header in &self.headers {
            names.insert(header.name());
        }
        names.into_iter().collect()
    }

    fn has_header(&self, name: &HeaderName) -> bool {
        self.headers.iter().any(|h| h.name() == *name)
    }
}

// Helper function to try casting a TypedHeader to a specific type
fn try_as_typed_header<'a, T: TypedHeaderTrait + 'static>(header: &'a TypedHeader) -> Option<&'a T> 
where 
    <T as TypedHeaderTrait>::Name: std::fmt::Debug,
    T: std::fmt::Debug
{
    header.as_typed_ref::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentLength;
    use crate::types::MaxForwards;
    use crate::types::CallId;
    use crate::types::CSeq;

    #[test]
    fn test_request_creation() {
        let address = types::Address::new("sip:alice@example.com".parse().unwrap());
        
        let request = Request::new(Method::Invite, Uri::sip("bob@example.com"))
            .with_header(TypedHeader::From(From::new(address)))
            .with_header(TypedHeader::To(To::new(types::Address::new("sip:bob@example.com".parse().unwrap()))))
            .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.example.com")))
            .with_header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .with_header(TypedHeader::Via(Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via")))
            .with_body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n");

        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri.to_string(), "sip:bob@example.com");
        assert_eq!(request.version, Version::new(2, 0));
        
        // Check for the main headers
        assert!(request.has_header(&HeaderName::From));
        assert!(request.has_header(&HeaderName::To));
        assert!(request.has_header(&HeaderName::CallId));
        assert!(request.has_header(&HeaderName::CSeq));
        assert!(request.has_header(&HeaderName::Via));
        assert!(request.has_header(&HeaderName::ContentLength));
        
        // Get content length directly from the headers
        let cl = request.header(&HeaderName::ContentLength).unwrap();
        if let TypedHeader::ContentLength(cl) = cl {
            assert_eq!(*cl, types::content_length::ContentLength(56));
        } else {
            panic!("Expected ContentLength header");
        }
    }

    #[test]
    fn test_request_with_header() {
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        assert_eq!(request.headers.len(), 2);
        assert!(request.header(&HeaderName::CallId).is_some());
        assert!(request.header(&HeaderName::MaxForwards).is_some());
    }

    #[test]
    fn test_request_with_body() {
        let body_content = "test body content";
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_body(Bytes::from(body_content));
        
        assert_eq!(request.body, Bytes::from(body_content));
        assert_eq!(request.body(), body_content.as_bytes());
    }

    #[test]
    fn test_set_headers() {
        let mut request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap());
        let headers = vec![
            TypedHeader::CallId(types::CallId::new("test-id")),
            TypedHeader::MaxForwards(MaxForwards::new(70))
        ];
        
        request.set_headers(headers);
        
        assert_eq!(request.headers.len(), 2);
        assert!(request.header(&HeaderName::CallId).is_some());
        assert!(request.header(&HeaderName::MaxForwards).is_some());
    }

    #[test]
    fn test_typed_header_access() {
        let call_id = types::CallId::new("test-id");
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let retrieved = request.typed_header::<types::CallId>();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value(), call_id.value());
        
        // Test for a header that doesn't exist
        let non_existent = request.typed_header::<ContentLength>();
        assert!(non_existent.is_none());
    }

    #[test]
    fn test_call_id_access() {
        let call_id = types::CallId::new("test-id");
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let retrieved = request.call_id();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value(), call_id.value());
    }

    #[test]
    fn test_via_headers() {
        let via = Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via");
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::Via(via.clone()));
        
        let vias = request.via_headers();
        assert_eq!(vias.len(), 1);
        
        let first_via = request.first_via();
        assert!(first_via.is_some());
    }

    #[test]
    fn test_to_bytes_no_body() {
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_body(Bytes::from("test body"));
        
        let bytes = request.to_bytes_no_body();
        let content = String::from_utf8_lossy(&bytes);
        
        assert!(content.contains("INVITE sip:bob@example.com SIP/2.0"));
        assert!(content.contains("Call-ID: test-id"));
        assert!(!content.contains("test body"));
    }

    #[test]
    fn test_display() {
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_body(Bytes::from("test body"));
        
        let display = format!("{}", request);
        
        assert!(display.contains("INVITE sip:bob@example.com SIP/2.0"));
        assert!(display.contains("Call-ID: test-id"));
        assert!(display.contains("test body"));
    }

    #[test]
    fn test_header_access_trait() {
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        // Test HeaderAccess implementation
        assert!(request.has_header(&HeaderName::CallId));
        assert!(request.has_header(&HeaderName::MaxForwards));
        assert!(!request.has_header(&HeaderName::To));
        
        let call_id_headers = request.headers(&HeaderName::CallId);
        assert_eq!(call_id_headers.len(), 1);
        
        let by_name = request.headers_by_name("Call-ID");
        assert_eq!(by_name.len(), 1);
        
        let names = request.header_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&HeaderName::CallId));
        assert!(names.contains(&HeaderName::MaxForwards));
        
        let raw_value = request.raw_header_value(&HeaderName::CallId);
        assert!(raw_value.is_some());
        assert!(raw_value.unwrap().contains("test-id"));
    }

    #[test]
    fn test_new_with_essentials() {
        let to_uri = "sip:bob@example.com".parse().unwrap();
        let from_uri = "sip:alice@example.com".parse().unwrap();
        
        let request = Request::new_with_essentials(
            Method::Invite,
            to_uri,
            from_uri,
            "abc123@example.com",
            1
        );
        
        assert_eq!(request.method, Method::Invite);
        assert!(request.has_header(&HeaderName::From));
        assert!(request.has_header(&HeaderName::To));
        assert!(request.has_header(&HeaderName::CallId));
        assert!(request.has_header(&HeaderName::CSeq));
        assert!(request.has_header(&HeaderName::MaxForwards));
        
        let call_id = request.call_id();
        assert!(call_id.is_some());
        assert_eq!(call_id.unwrap().value(), "abc123@example.com");
    }
} 