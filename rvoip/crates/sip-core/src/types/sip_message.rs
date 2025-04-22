use std::collections::HashMap;
use std::fmt;
use bytes::Bytes;
// use serde::{Deserialize, Serialize}; // Commented out Serde

use crate::types::header::{Header, HeaderName, TypedHeader};
use crate::types::uri::Uri;
use crate::types::version::Version;
// Use types from the current crate's types module
use crate::types::Method;
use crate::types::StatusCode;
use crate::error::{Error, Result};
// Use types from the current crate's types module
use crate::types::{Via};
use crate::types; // Add import
use crate::method::Method;
use crate::uri::Uri;
use crate::version::Version;
use crate::header::Header;
use crate::header::HeaderName;
use crate::header::TypedHeader;
use crate::types::via::Via; // Import Via specifically

/// A SIP request message
#[derive(Debug, Clone, PartialEq)]
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
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets all headers from a Vec<TypedHeader> (used by parser)
    pub fn set_headers(&mut self, headers: Vec<TypedHeader>) {
        self.headers = headers;
    }

    /// Sets the body of the request
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Retrieves the first typed header with the specified name, if any
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
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
    /// Note: This relies on the Via parser being available where called.
    /// TODO: Refactor to return `Result<Vec<Via>>` or use a dedicated typed header getter.
    pub fn via_headers(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in &self.headers {
            // Directly match the TypedHeader::Via variant
            if let TypedHeader::Via(via_data) = header {
                // via_data should be of type types::Via which might contain Vec<ViaHeader> or similar
                // Assuming types::Via can be directly used or converted
                // If types::Via holds a Vec<ViaHeader>, we might need to clone/extend
                // Let's assume types::Via can be cloned directly for now.
                // If it holds a Vec<ViaHeader>, we'd use result.extend(via_data.0.clone());
                result.push(via_data.clone()); // Adjust based on actual types::Via structure
            }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    /// TODO: Refactor similar to via_headers.
    pub fn first_via(&self) -> Option<Via> {
        self.headers.iter().find_map(|h| {
             if let TypedHeader::Via(via_data) = h {
                 Some(via_data.clone()) // Clone if necessary
             } else {
                 None
             }
        })
        // Original implementation: self.via_headers().into_iter().next() - less efficient
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
                 // Adjust cloning/extending based on actual types::Via structure
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
                 Some(via_data.clone()) // Clone if necessary
             } else {
                 None
             }
        })
        // Original implementation: self.via_headers_no_body().into_iter().next()
    }

    /// Convert the message to bytes
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
#[derive(Debug, Clone, PartialEq)]
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
    pub fn trying() -> Self {
        Response::new(StatusCode::Trying)
    }

    /// Creates a SIP 180 Ringing response
    pub fn ringing() -> Self {
        Response::new(StatusCode::Ringing)
    }

    /// Creates a SIP 200 OK response
    pub fn ok() -> Self {
        Response::new(StatusCode::Ok)
    }

    /// Adds a typed header to the response
    pub fn with_header(mut self, header: TypedHeader) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets all headers from a Vec<TypedHeader> (used by parser)
    pub fn set_headers(&mut self, headers: Vec<TypedHeader>) {
        self.headers = headers;
    }

    /// Sets a custom reason phrase for the response
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Sets the body of the response
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Retrieves the first typed header with the specified name, if any
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers.iter().find(|h| h.name() == *name)
    }

    /// Gets the reason phrase for this response (either the custom one or the default)
    pub fn reason_phrase(&self) -> &str {
        self.reason.as_deref().unwrap_or_else(|| self.status.reason_phrase())
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
    /// Note: This relies on the Via parser being available where called.
    /// TODO: Refactor to return `Result<Vec<Via>>` or use a dedicated typed header getter.
    pub fn via_headers(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in &self.headers {
            // Directly match the TypedHeader::Via variant
            if let TypedHeader::Via(via_data) = header {
                 // Adjust cloning/extending based on actual types::Via structure
                 result.push(via_data.clone()); 
            }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    /// TODO: Refactor similar to via_headers.
    pub fn first_via(&self) -> Option<Via> {
         self.headers.iter().find_map(|h| {
              if let TypedHeader::Via(via_data) = h {
                  Some(via_data.clone()) // Clone if necessary
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
    pub fn header(&self, name: &HeaderName) -> Option<&TypedHeader> {
        self.headers().iter().find(|h| h.name() == *name)
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
    /// Note: This relies on the Via parser being available where called.
    /// TODO: Refactor to return `Result<Vec<Via>>` or use a dedicated typed header getter.
    pub fn via_headers(&self) -> Vec<Via> {
        let mut result = Vec::new();
        for header in self.headers() {
             // Directly match the TypedHeader::Via variant
             if let TypedHeader::Via(via_data) = header {
                  // Adjust cloning/extending based on actual types::Via structure
                  result.push(via_data.clone()); 
             }
        }
        result
    }

    /// Get the first Via header as a structured Via object
    /// TODO: Refactor similar to via_headers.
    pub fn first_via(&self) -> Option<Via> {
         self.headers().iter().find_map(|h| {
              if let TypedHeader::Via(via_data) = h {
                  Some(via_data.clone()) // Clone if necessary
              } else {
                  None
              }
         })
    }

    /// Convert the message to bytes
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