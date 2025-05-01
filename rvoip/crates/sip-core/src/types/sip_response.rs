//! # SIP Response Message
//!
//! This module provides the SIP Response message type.
//!
//! The Response struct represents SIP response messages sent from servers to clients,
//! containing a status code, reason phrase, headers, and optional body.
//!
//! ## Response Structure
//!
//! A SIP response consists of:
//! - A status line (containing the SIP version, status code, and reason phrase)
//! - A set of headers providing metadata
//! - An empty line separating headers from the body
//! - An optional message body
//!
//! ## RFC Compliance
//!
//! This implementation follows [RFC 3261](https://tools.ietf.org/html/rfc3261),
//! which defines the Session Initiation Protocol.
//!
//! ## Status Code Classes
//!
//! SIP response status codes are grouped into classes:
//!
//! - 1xx (Provisional): Request received and being processed
//! - 2xx (Success): Action successfully received, understood, and accepted
//! - 3xx (Redirection): Further action needs to be taken to complete the request
//! - 4xx (Client Error): Request contains bad syntax or cannot be fulfilled at this server
//! - 5xx (Server Error): Server failed to fulfill an apparently valid request
//! - 6xx (Global Failure): Request cannot be fulfilled at any server
//!
//! ## Examples
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

use std::fmt;
use std::collections::HashSet;
use std::str::FromStr;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::types::header::{HeaderName, TypedHeader, TypedHeaderTrait};
use crate::types::version::Version;
use crate::types::method::Method;
use crate::types::StatusCode;
use crate::types;
use crate::types::via::{Via, ViaHeader};
use crate::types::headers::HeaderAccess;
use crate::error::{Error, Result};
use crate::types::to::To;
use crate::types::sip_request::Request;
use crate::types::from::From;

/// A SIP response message
///
/// Represents a SIP response sent from a server to a client, containing
/// a status code, reason phrase, headers, and optional body. SIP responses
/// acknowledge requests and provide information about their processing.
///
/// # Standard RFC Compliance
///
/// This implementation follows [RFC 3261](https://tools.ietf.org/html/rfc3261), 
/// which defines the Session Initiation Protocol.
///
/// # Fields
///
/// - `version`: The SIP protocol version (typically SIP/2.0)
/// - `status`: The response status code (e.g., 200 for OK, 404 for Not Found)
/// - `reason`: Optional custom reason phrase (overrides the default for the status code)
/// - `headers`: A list of message headers
/// - `body`: The message body (optional)
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Create a 404 Not Found response
    /// let response = Response::new(StatusCode::NotFound);
    ///
    /// assert_eq!(response.status, StatusCode::NotFound);
    /// assert_eq!(response.reason_phrase(), "Not Found");
    /// assert!(response.headers.is_empty());
    /// assert!(response.body.is_empty());
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::trying();
    /// assert_eq!(response.status, StatusCode::Trying);
    /// ```
    pub fn trying() -> Self {
        Response::new(StatusCode::Trying)
    }

    /// Creates a SIP 180 Ringing response
    ///
    /// # Returns
    /// A new `Response` with 180 Ringing status
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::ringing();
    /// assert_eq!(response.status, StatusCode::Ringing);
    /// ```
    pub fn ringing() -> Self {
        Response::new(StatusCode::Ringing)
    }

    /// Creates a SIP 200 OK response
    ///
    /// # Returns
    /// A new `Response` with 200 OK status
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::ok();
    /// assert_eq!(response.status, StatusCode::Ok);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")))
    ///     .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
    ///
    /// assert_eq!(response.headers.len(), 2);
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
    /// let mut response = Response::new(StatusCode::Ok);
    /// let headers = vec![
    ///     TypedHeader::CallId(CallId::new("abc123")),
    ///     TypedHeader::MaxForwards(MaxForwards::new(70))
    /// ];
    ///
    /// response.set_headers(headers);
    /// assert_eq!(response.headers.len(), 2);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_reason("Everything is Awesome");
    ///
    /// assert_eq!(response.reason_phrase(), "Everything is Awesome");
    /// ```
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the body of the response
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
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::CallId(CallId::new("abc123")));
    ///
    /// let header = response.header(&HeaderName::CallId);
    /// assert!(header.is_some());
    /// ```
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    /// Gets the reason phrase for this response (either the custom one or the default)
    ///
    /// # Returns
    /// The reason phrase as a string slice
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// // Default reason phrase
    /// let response = Response::new(StatusCode::NotFound);
    /// assert_eq!(response.reason_phrase(), "Not Found");
    ///
    /// // Custom reason phrase
    /// let response = Response::new(StatusCode::NotFound)
    ///     .with_reason("Resource Not Available");
    /// assert_eq!(response.reason_phrase(), "Resource Not Available");
    /// ```
    pub fn reason_phrase(&self) -> &str {
        self.reason.as_deref().unwrap_or_else(|| self.status.reason_phrase())
    }
    
    /// Retrieves the first header with the specified type, if any.
    pub fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> {
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
    /// An optional string reference to the From header value
    pub fn from(&self) -> Option<&str> {
        None // Placeholder
    }
    
    /// Retrieves the To header value, if present
    ///
    /// # Returns
    /// An optional string reference to the To header value
    pub fn to(&self) -> Option<&str> {
        None // Placeholder
    }

    /// Get all Via headers as structured Via objects
    ///
    /// # Returns
    /// A vector of all Via headers in the response
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let via = Via::new("UDP", "example.com", 5060);
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::Via(via.clone()));
    ///
    /// let vias = response.via_headers();
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
    /// let via = Via::new("UDP", "example.com", 5060);
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_header(TypedHeader::Via(via.clone()));
    ///
    /// let first_via = response.first_via();
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

    /// Returns the status code of the response
    ///
    /// # Returns
    /// The response status code
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let response = Response::new(StatusCode::Ok);
    /// assert_eq!(response.status(), StatusCode::Ok);
    /// ```
    pub fn status(&self) -> StatusCode {
        self.status
    }
    
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
    /// let response = Response::new(StatusCode::Ok)
    ///     .with_body(Bytes::from(body_content));
    ///
    /// assert_eq!(response.body(), body_content.as_bytes());
    /// ```
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Creates a new response with all essential headers copied from a request
    ///
    /// This is a convenience method that creates a response with:
    /// - From header (copied from request)
    /// - To header (copied from request)
    /// - Call-ID header (copied from request)
    /// - CSeq header (copied from request)
    /// - Via headers (copied from request)
    ///
    /// # Parameters
    /// - `status`: The status code for the response
    /// - `request`: The request to copy headers from
    ///
    /// # Returns
    /// A well-formed Response with all required headers
    ///
    /// # Examples
    ///
    /// ```rust
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
    ///     .with_header(TypedHeader::From(types::from::From::new(types::Address::new("sip:alice@example.com".parse().unwrap()))))
    ///     .with_header(TypedHeader::To(types::To::new(types::Address::new("sip:bob@example.com".parse().unwrap()))))
    ///     .with_header(TypedHeader::CallId(types::CallId::new("abc123")))
    ///     .with_header(TypedHeader::CSeq(types::CSeq::new(1, Method::Invite)));
    ///
    /// let response = Response::from_request(StatusCode::Ok, &request);
    ///
    /// assert_eq!(response.status, StatusCode::Ok);
    /// assert!(response.has_header(&HeaderName::From));
    /// assert!(response.has_header(&HeaderName::To));
    /// assert!(response.has_header(&HeaderName::CallId));
    /// assert!(response.has_header(&HeaderName::CSeq));
    /// ```
    pub fn from_request(status: StatusCode, request: &crate::types::sip_request::Request) -> Self {
        let mut response = Response::new(status);
        
        // Copy essential headers
        for header in &request.headers {
            match header {
                TypedHeader::From(_) | 
                TypedHeader::To(_) | 
                TypedHeader::CallId(_) | 
                TypedHeader::CSeq(_) |
                TypedHeader::Via(_) => {
                    response.headers.push(header.clone());
                },
                _ => {} // Skip other headers
            }
        }
        
        response
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Status line
        write!(f, "{} {} {}\r\n", 
            self.version, 
            self.status.as_u16(), 
            self.reason_phrase())?;
        
        // Headers
        for header in &self.headers {
            write!(f, "{}\r\n", header)?;
        }
        
        // Blank line and body (if any)
        write!(f, "\r\n")?;
        if !self.body.is_empty() {
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

// Implement HeaderAccess for Response
impl HeaderAccess for Response {
    fn typed_headers<T: TypedHeaderTrait + 'static>(&self) -> Vec<&T> {
        use crate::types::headers::collect_typed_headers;
        collect_typed_headers::<T>(&self.headers)
    }

    fn typed_header<T: TypedHeaderTrait + 'static>(&self) -> Option<&T> {
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
fn try_as_typed_header<'a, T: TypedHeaderTrait + 'static>(header: &'a TypedHeader) -> Option<&'a T> {
    header.as_typed_ref::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ContentLength;
    use crate::types::MaxForwards;
    use crate::types::CallId;
    use crate::types::CSeq;
    use crate::types::sip_request::Request;
    
    #[test]
    fn test_response_creation() {
        let address = types::Address::new("sip:alice@example.com".parse().unwrap());
        
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::From(From::new(address)))
            .with_header(TypedHeader::To(To::new(types::Address::new("sip:bob@example.com".parse().unwrap())).with_tag("a6c85cf")))
            .with_header(TypedHeader::CallId(CallId::new("a84b4c76e66710@pc33.atlanta.example.com")))
            .with_header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .with_header(TypedHeader::Via(Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via")))
            .with_body("v=0\r\no=bob 2890844527 2890844527 IN IP4 example.com\r\ns=\r\nt=0 0\r\n");

        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.version, Version::new(2, 0));
        
        assert_eq!(response.headers.len(), 6); // 5 headers + Content-Length
        assert!(response.has_header(&HeaderName::From));
        assert!(response.has_header(&HeaderName::To));
        assert!(response.has_header(&HeaderName::CallId));
        assert!(response.has_header(&HeaderName::CSeq));
        assert!(response.has_header(&HeaderName::Via));
        assert!(response.has_header(&HeaderName::ContentLength));
        
        // Check Content-Length was set correctly
        let cl = response.header(&HeaderName::ContentLength).unwrap();
        if let TypedHeader::ContentLength(cl) = cl {
            assert_eq!(*cl, types::content_length::ContentLength(64));
        } else {
            panic!("Expected ContentLength header");
        }
    }

    #[test]
    fn test_convenience_constructors() {
        let trying = Response::trying();
        assert_eq!(trying.status, StatusCode::Trying);
        
        let ringing = Response::ringing();
        assert_eq!(ringing.status, StatusCode::Ringing);
        
        let ok = Response::ok();
        assert_eq!(ok.status, StatusCode::Ok);
    }

    #[test]
    fn test_response_with_header() {
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        assert_eq!(response.headers.len(), 2);
        assert!(response.header(&HeaderName::CallId).is_some());
        assert!(response.header(&HeaderName::MaxForwards).is_some());
    }

    #[test]
    fn test_response_with_reason() {
        let custom_reason = "Everything is Awesome";
        let response = Response::new(StatusCode::Ok)
            .with_reason(custom_reason);
        
        assert_eq!(response.reason, Some(custom_reason.to_string()));
        assert_eq!(response.reason_phrase(), custom_reason);
        
        // Test default reason
        let response = Response::new(StatusCode::NotFound);
        assert_eq!(response.reason_phrase(), "Not Found");
    }

    #[test]
    fn test_response_with_body() {
        let body_content = "test body content";
        let response = Response::new(StatusCode::Ok)
            .with_body(Bytes::from(body_content));
        
        assert_eq!(response.body, Bytes::from(body_content));
        assert_eq!(response.body(), body_content.as_bytes());
    }

    #[test]
    fn test_set_headers() {
        let mut response = Response::new(StatusCode::Ok);
        let headers = vec![
            TypedHeader::CallId(types::CallId::new("test-id")),
            TypedHeader::MaxForwards(MaxForwards::new(70))
        ];
        
        response.set_headers(headers);
        
        assert_eq!(response.headers.len(), 2);
        assert!(response.header(&HeaderName::CallId).is_some());
        assert!(response.header(&HeaderName::MaxForwards).is_some());
    }

    #[test]
    fn test_typed_header_access() {
        let call_id = types::CallId::new("test-id");
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let retrieved = response.typed_header::<types::CallId>();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value(), call_id.value());
        
        // Test for a header that doesn't exist
        let non_existent = response.typed_header::<ContentLength>();
        assert!(non_existent.is_none());
    }

    #[test]
    fn test_call_id_access() {
        let call_id = types::CallId::new("test-id");
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(call_id.clone()));
        
        let retrieved = response.call_id();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().value(), call_id.value());
    }

    #[test]
    fn test_via_headers() {
        let via = Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via");
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::Via(via.clone()));
        
        let vias = response.via_headers();
        assert_eq!(vias.len(), 1);
        
        let first_via = response.first_via();
        assert!(first_via.is_some());
    }

    #[test]
    fn test_display() {
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_body(Bytes::from("test body"));
        
        let display = format!("{}", response);
        
        assert!(display.contains("SIP/2.0 200 OK"));
        assert!(display.contains("Call-ID: test-id"));
        assert!(display.contains("test body"));
    }

    #[test]
    fn test_header_access_trait() {
        let response = Response::new(StatusCode::Ok)
            .with_header(TypedHeader::CallId(types::CallId::new("test-id")))
            .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        // Test HeaderAccess implementation
        assert!(response.has_header(&HeaderName::CallId));
        assert!(response.has_header(&HeaderName::MaxForwards));
        assert!(!response.has_header(&HeaderName::To));
        
        let call_id_headers = response.headers(&HeaderName::CallId);
        assert_eq!(call_id_headers.len(), 1);
        
        let by_name = response.headers_by_name("Call-ID");
        assert_eq!(by_name.len(), 1);
        
        let names = response.header_names();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&HeaderName::CallId));
        assert!(names.contains(&HeaderName::MaxForwards));
        
        let raw_value = response.raw_header_value(&HeaderName::CallId);
        assert!(raw_value.is_some());
        assert!(raw_value.unwrap().contains("test-id"));
    }

    #[test]
    fn test_from_request() {
        let address = types::Address::new("sip:alice@example.com".parse().unwrap());
        
        let request = Request::new(Method::Invite, "sip:bob@example.com".parse().unwrap())
            .with_header(TypedHeader::From(From::new(address)))
            .with_header(TypedHeader::To(To::new(types::Address::new("sip:bob@example.com".parse().unwrap()))))
            .with_header(TypedHeader::CallId(types::CallId::new("abc123")))
            .with_header(TypedHeader::CSeq(types::CSeq::new(1, Method::Invite)))
            .with_header(TypedHeader::Via(Via::new_simple("SIP", "2.0", "UDP", "example.com", Some(5060), vec![]).expect("Failed to create Via")))
            .with_header(TypedHeader::ContentLength(ContentLength::new(0)));
            
        let response = Response::from_request(StatusCode::Ok, &request);
        
        assert_eq!(response.status, StatusCode::Ok);
        assert!(response.has_header(&HeaderName::From));
        assert!(response.has_header(&HeaderName::To));
        assert!(response.has_header(&HeaderName::CallId));
        assert!(response.has_header(&HeaderName::CSeq));
        assert!(response.has_header(&HeaderName::Via));
        
        // Content-Length should not be copied
        assert!(!response.has_header(&HeaderName::ContentLength));
    }
} 