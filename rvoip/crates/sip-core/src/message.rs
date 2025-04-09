use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::header::{Header, HeaderName, HeaderValue};
use crate::method::Method;
use crate::uri::Uri;
use crate::version::Version;

/// SIP status codes as defined in RFC 3261 and extensions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum StatusCode {
    // 1xx: Provisional
    /// 100 Trying
    Trying = 100,
    /// 180 Ringing
    Ringing = 180,
    /// 181 Call Is Being Forwarded
    CallIsBeingForwarded = 181,
    /// 182 Queued
    Queued = 182,
    /// 183 Session Progress
    SessionProgress = 183,
    
    // 2xx: Success
    /// 200 OK
    Ok = 200,
    /// 202 Accepted
    Accepted = 202,
    
    // 3xx: Redirection
    /// 300 Multiple Choices
    MultipleChoices = 300,
    /// 301 Moved Permanently
    MovedPermanently = 301,
    /// 302 Moved Temporarily
    MovedTemporarily = 302,
    /// 305 Use Proxy
    UseProxy = 305,
    /// 380 Alternative Service
    AlternativeService = 380,
    
    // 4xx: Client Error
    /// 400 Bad Request
    BadRequest = 400,
    /// 401 Unauthorized
    Unauthorized = 401,
    /// 402 Payment Required
    PaymentRequired = 402,
    /// 403 Forbidden
    Forbidden = 403,
    /// 404 Not Found
    NotFound = 404,
    /// 405 Method Not Allowed
    MethodNotAllowed = 405,
    /// 406 Not Acceptable
    NotAcceptable = 406,
    /// 407 Proxy Authentication Required
    ProxyAuthenticationRequired = 407,
    /// 408 Request Timeout
    RequestTimeout = 408,
    /// 410 Gone
    Gone = 410,
    /// 413 Request Entity Too Large
    RequestEntityTooLarge = 413,
    /// 414 Request-URI Too Long
    RequestUriTooLong = 414,
    /// 415 Unsupported Media Type
    UnsupportedMediaType = 415,
    /// 416 Unsupported URI Scheme
    UnsupportedUriScheme = 416,
    /// 420 Bad Extension
    BadExtension = 420,
    /// 421 Extension Required
    ExtensionRequired = 421,
    /// 423 Interval Too Brief
    IntervalTooBrief = 423,
    /// 480 Temporarily Unavailable
    TemporarilyUnavailable = 480,
    /// 481 Call/Transaction Does Not Exist
    CallOrTransactionDoesNotExist = 481,
    /// 482 Loop Detected
    LoopDetected = 482,
    /// 483 Too Many Hops
    TooManyHops = 483,
    /// 484 Address Incomplete
    AddressIncomplete = 484,
    /// 485 Ambiguous
    Ambiguous = 485,
    /// 486 Busy Here
    BusyHere = 486,
    /// 487 Request Terminated
    RequestTerminated = 487,
    /// 488 Not Acceptable Here
    NotAcceptableHere = 488,
    /// 491 Request Pending
    RequestPending = 491,
    /// 493 Undecipherable
    Undecipherable = 493,
    
    // 5xx: Server Error
    /// 500 Server Internal Error
    ServerInternalError = 500,
    /// 501 Not Implemented
    NotImplemented = 501,
    /// 502 Bad Gateway
    BadGateway = 502,
    /// 503 Service Unavailable
    ServiceUnavailable = 503,
    /// 504 Server Time-out
    ServerTimeout = 504,
    /// 505 Version Not Supported
    VersionNotSupported = 505,
    /// 513 Message Too Large
    MessageTooLarge = 513,
    
    // 6xx: Global Failure
    /// 600 Busy Everywhere
    BusyEverywhere = 600,
    /// 603 Decline
    Decline = 603,
    /// 604 Does Not Exist Anywhere
    DoesNotExistAnywhere = 604,
    /// 606 Not Acceptable
    NotAcceptable606 = 606,
    
    /// Custom status code (with value)
    Custom(u16),
}

impl StatusCode {
    /// Creates a status code from a raw u16 value
    pub fn from_u16(code: u16) -> Result<Self> {
        match code {
            100 => Ok(StatusCode::Trying),
            180 => Ok(StatusCode::Ringing),
            181 => Ok(StatusCode::CallIsBeingForwarded),
            182 => Ok(StatusCode::Queued),
            183 => Ok(StatusCode::SessionProgress),
            
            200 => Ok(StatusCode::Ok),
            202 => Ok(StatusCode::Accepted),
            
            300 => Ok(StatusCode::MultipleChoices),
            301 => Ok(StatusCode::MovedPermanently),
            302 => Ok(StatusCode::MovedTemporarily),
            305 => Ok(StatusCode::UseProxy),
            380 => Ok(StatusCode::AlternativeService),
            
            400 => Ok(StatusCode::BadRequest),
            401 => Ok(StatusCode::Unauthorized),
            402 => Ok(StatusCode::PaymentRequired),
            403 => Ok(StatusCode::Forbidden),
            404 => Ok(StatusCode::NotFound),
            405 => Ok(StatusCode::MethodNotAllowed),
            406 => Ok(StatusCode::NotAcceptable),
            407 => Ok(StatusCode::ProxyAuthenticationRequired),
            408 => Ok(StatusCode::RequestTimeout),
            410 => Ok(StatusCode::Gone),
            413 => Ok(StatusCode::RequestEntityTooLarge),
            414 => Ok(StatusCode::RequestUriTooLong),
            415 => Ok(StatusCode::UnsupportedMediaType),
            416 => Ok(StatusCode::UnsupportedUriScheme),
            420 => Ok(StatusCode::BadExtension),
            421 => Ok(StatusCode::ExtensionRequired),
            423 => Ok(StatusCode::IntervalTooBrief),
            480 => Ok(StatusCode::TemporarilyUnavailable),
            481 => Ok(StatusCode::CallOrTransactionDoesNotExist),
            482 => Ok(StatusCode::LoopDetected),
            483 => Ok(StatusCode::TooManyHops),
            484 => Ok(StatusCode::AddressIncomplete),
            485 => Ok(StatusCode::Ambiguous),
            486 => Ok(StatusCode::BusyHere),
            487 => Ok(StatusCode::RequestTerminated),
            488 => Ok(StatusCode::NotAcceptableHere),
            491 => Ok(StatusCode::RequestPending),
            493 => Ok(StatusCode::Undecipherable),
            
            500 => Ok(StatusCode::ServerInternalError),
            501 => Ok(StatusCode::NotImplemented),
            502 => Ok(StatusCode::BadGateway),
            503 => Ok(StatusCode::ServiceUnavailable),
            504 => Ok(StatusCode::ServerTimeout),
            505 => Ok(StatusCode::VersionNotSupported),
            513 => Ok(StatusCode::MessageTooLarge),
            
            600 => Ok(StatusCode::BusyEverywhere),
            603 => Ok(StatusCode::Decline),
            604 => Ok(StatusCode::DoesNotExistAnywhere),
            606 => Ok(StatusCode::NotAcceptable606),
            
            _ if code >= 100 && code < 700 => Ok(StatusCode::Custom(code)),
            _ => Err(Error::InvalidStatusCode(code)),
        }
    }

    /// Returns the numeric value of this status code
    pub fn as_u16(&self) -> u16 {
        match self {
            StatusCode::Trying => 100,
            StatusCode::Ringing => 180,
            StatusCode::CallIsBeingForwarded => 181,
            StatusCode::Queued => 182,
            StatusCode::SessionProgress => 183,
            
            StatusCode::Ok => 200,
            StatusCode::Accepted => 202,
            
            StatusCode::MultipleChoices => 300,
            StatusCode::MovedPermanently => 301,
            StatusCode::MovedTemporarily => 302,
            StatusCode::UseProxy => 305,
            StatusCode::AlternativeService => 380,
            
            StatusCode::BadRequest => 400,
            StatusCode::Unauthorized => 401,
            StatusCode::PaymentRequired => 402,
            StatusCode::Forbidden => 403,
            StatusCode::NotFound => 404,
            StatusCode::MethodNotAllowed => 405,
            StatusCode::NotAcceptable => 406,
            StatusCode::ProxyAuthenticationRequired => 407,
            StatusCode::RequestTimeout => 408,
            StatusCode::Gone => 410,
            StatusCode::RequestEntityTooLarge => 413,
            StatusCode::RequestUriTooLong => 414,
            StatusCode::UnsupportedMediaType => 415,
            StatusCode::UnsupportedUriScheme => 416,
            StatusCode::BadExtension => 420,
            StatusCode::ExtensionRequired => 421,
            StatusCode::IntervalTooBrief => 423,
            StatusCode::TemporarilyUnavailable => 480,
            StatusCode::CallOrTransactionDoesNotExist => 481,
            StatusCode::LoopDetected => 482,
            StatusCode::TooManyHops => 483,
            StatusCode::AddressIncomplete => 484,
            StatusCode::Ambiguous => 485,
            StatusCode::BusyHere => 486,
            StatusCode::RequestTerminated => 487,
            StatusCode::NotAcceptableHere => 488,
            StatusCode::RequestPending => 491,
            StatusCode::Undecipherable => 493,
            
            StatusCode::ServerInternalError => 500,
            StatusCode::NotImplemented => 501,
            StatusCode::BadGateway => 502,
            StatusCode::ServiceUnavailable => 503,
            StatusCode::ServerTimeout => 504,
            StatusCode::VersionNotSupported => 505,
            StatusCode::MessageTooLarge => 513,
            
            StatusCode::BusyEverywhere => 600,
            StatusCode::Decline => 603,
            StatusCode::DoesNotExistAnywhere => 604,
            StatusCode::NotAcceptable606 => 606,
            
            StatusCode::Custom(code) => *code,
        }
    }

    /// Returns the canonical reason phrase for this status code
    pub fn reason_phrase(&self) -> &'static str {
        match self {
            StatusCode::Trying => "Trying",
            StatusCode::Ringing => "Ringing",
            StatusCode::CallIsBeingForwarded => "Call Is Being Forwarded",
            StatusCode::Queued => "Queued",
            StatusCode::SessionProgress => "Session Progress",
            
            StatusCode::Ok => "OK",
            StatusCode::Accepted => "Accepted",
            
            StatusCode::MultipleChoices => "Multiple Choices",
            StatusCode::MovedPermanently => "Moved Permanently",
            StatusCode::MovedTemporarily => "Moved Temporarily",
            StatusCode::UseProxy => "Use Proxy",
            StatusCode::AlternativeService => "Alternative Service",
            
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Unauthorized => "Unauthorized",
            StatusCode::PaymentRequired => "Payment Required",
            StatusCode::Forbidden => "Forbidden",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::NotAcceptable => "Not Acceptable",
            StatusCode::ProxyAuthenticationRequired => "Proxy Authentication Required",
            StatusCode::RequestTimeout => "Request Timeout",
            StatusCode::Gone => "Gone",
            StatusCode::RequestEntityTooLarge => "Request Entity Too Large",
            StatusCode::RequestUriTooLong => "Request-URI Too Long",
            StatusCode::UnsupportedMediaType => "Unsupported Media Type",
            StatusCode::UnsupportedUriScheme => "Unsupported URI Scheme",
            StatusCode::BadExtension => "Bad Extension",
            StatusCode::ExtensionRequired => "Extension Required",
            StatusCode::IntervalTooBrief => "Interval Too Brief",
            StatusCode::TemporarilyUnavailable => "Temporarily Unavailable",
            StatusCode::CallOrTransactionDoesNotExist => "Call/Transaction Does Not Exist",
            StatusCode::LoopDetected => "Loop Detected",
            StatusCode::TooManyHops => "Too Many Hops",
            StatusCode::AddressIncomplete => "Address Incomplete",
            StatusCode::Ambiguous => "Ambiguous",
            StatusCode::BusyHere => "Busy Here",
            StatusCode::RequestTerminated => "Request Terminated",
            StatusCode::NotAcceptableHere => "Not Acceptable Here",
            StatusCode::RequestPending => "Request Pending",
            StatusCode::Undecipherable => "Undecipherable",
            
            StatusCode::ServerInternalError => "Server Internal Error",
            StatusCode::NotImplemented => "Not Implemented",
            StatusCode::BadGateway => "Bad Gateway",
            StatusCode::ServiceUnavailable => "Service Unavailable",
            StatusCode::ServerTimeout => "Server Time-out",
            StatusCode::VersionNotSupported => "Version Not Supported",
            StatusCode::MessageTooLarge => "Message Too Large",
            
            StatusCode::BusyEverywhere => "Busy Everywhere",
            StatusCode::Decline => "Decline",
            StatusCode::DoesNotExistAnywhere => "Does Not Exist Anywhere",
            StatusCode::NotAcceptable606 => "Not Acceptable",
            
            StatusCode::Custom(_) => "Unknown",
        }
    }

    /// Returns true if this status code is provisional (1xx)
    pub fn is_provisional(&self) -> bool {
        let code = self.as_u16();
        code >= 100 && code < 200
    }

    /// Returns true if this status code is success (2xx)
    pub fn is_success(&self) -> bool {
        let code = self.as_u16();
        code >= 200 && code < 300
    }

    /// Returns true if this status code is redirection (3xx)
    pub fn is_redirection(&self) -> bool {
        let code = self.as_u16();
        code >= 300 && code < 400
    }

    /// Returns true if this status code is client error (4xx)
    pub fn is_client_error(&self) -> bool {
        let code = self.as_u16();
        code >= 400 && code < 500
    }

    /// Returns true if this status code is server error (5xx)
    pub fn is_server_error(&self) -> bool {
        let code = self.as_u16();
        code >= 500 && code < 600
    }

    /// Returns true if this status code is global failure (6xx)
    pub fn is_global_failure(&self) -> bool {
        let code = self.as_u16();
        code >= 600 && code < 700
    }

    /// Returns true if this status code indicates an error (4xx, 5xx, 6xx)
    pub fn is_error(&self) -> bool {
        let code = self.as_u16();
        code >= 400 && code < 700
    }

    /// Get the textual reason phrase for the status code
    pub fn as_reason(&self) -> &'static str {
        match self {
            Self::Trying => "Trying",
            Self::Ringing => "Ringing",
            Self::CallIsBeingForwarded => "Call Is Being Forwarded",
            Self::Queued => "Queued",
            Self::SessionProgress => "Session Progress",
            Self::Ok => "OK",
            Self::Accepted => "Accepted",
            Self::MultipleChoices => "Multiple Choices",
            Self::MovedPermanently => "Moved Permanently",
            Self::MovedTemporarily => "Moved Temporarily",
            Self::UseProxy => "Use Proxy",
            Self::AlternativeService => "Alternative Service",
            Self::BadRequest => "Bad Request",
            Self::Unauthorized => "Unauthorized",
            Self::PaymentRequired => "Payment Required",
            Self::Forbidden => "Forbidden",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::NotAcceptable => "Not Acceptable",
            Self::ProxyAuthenticationRequired => "Proxy Authentication Required",
            Self::RequestTimeout => "Request Timeout",
            Self::Gone => "Gone",
            Self::RequestEntityTooLarge => "Request Entity Too Large",
            Self::RequestUriTooLong => "Request-URI Too Long",
            Self::UnsupportedMediaType => "Unsupported Media Type",
            Self::UnsupportedUriScheme => "Unsupported URI Scheme",
            Self::BadExtension => "Bad Extension",
            Self::ExtensionRequired => "Extension Required",
            Self::IntervalTooBrief => "Interval Too Brief",
            Self::TemporarilyUnavailable => "Temporarily Unavailable",
            Self::CallOrTransactionDoesNotExist => "Call/Transaction Does Not Exist",
            Self::LoopDetected => "Loop Detected",
            Self::TooManyHops => "Too Many Hops",
            Self::AddressIncomplete => "Address Incomplete",
            Self::Ambiguous => "Ambiguous",
            Self::BusyHere => "Busy Here",
            Self::RequestTerminated => "Request Terminated",
            Self::NotAcceptableHere => "Not Acceptable Here",
            Self::RequestPending => "Request Pending",
            Self::Undecipherable => "Undecipherable",
            Self::ServerInternalError => "Server Internal Error",
            Self::NotImplemented => "Not Implemented",
            Self::BadGateway => "Bad Gateway",
            Self::ServiceUnavailable => "Service Unavailable",
            Self::ServerTimeout => "Server Time-out",
            Self::VersionNotSupported => "Version Not Supported",
            Self::MessageTooLarge => "Message Too Large",
            Self::BusyEverywhere => "Busy Everywhere",
            Self::Decline => "Decline",
            Self::DoesNotExistAnywhere => "Does Not Exist Anywhere",
            Self::NotAcceptable606 => "Not Acceptable",
            Self::Custom(_) => "Custom Status Code",
        }
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.as_u16(), self.reason_phrase())
    }
}

impl FromStr for StatusCode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let code = s.parse::<u16>().map_err(|_| Error::InvalidStatusCode(0))?;
        StatusCode::from_u16(code)
    }
}

/// A SIP request message
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The method of the request
    pub method: Method,
    /// The request URI
    pub uri: Uri,
    /// The SIP version
    pub version: Version,
    /// The headers of the request
    pub headers: Vec<Header>,
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

    /// Adds a header to the request
    pub fn with_header(mut self, header: Header) -> Self {
        self.headers.push(header);
        self
    }

    /// Sets the body of the request
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = body.into();
        self
    }

    /// Retrieves the first header with the specified name, if any
    pub fn header(&self, name: &HeaderName) -> Option<&Header> {
        self.headers.iter().find(|h| &h.name == name)
    }

    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&str> {
        self.header(&HeaderName::CallId).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        self.header(&HeaderName::From).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        self.header(&HeaderName::To).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the CSeq header value, if present
    pub fn cseq(&self) -> Option<&str> {
        self.header(&HeaderName::CSeq).and_then(|h| h.value.as_text())
    }
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The SIP version
    pub version: Version,
    /// The status code
    pub status: StatusCode,
    /// Custom reason phrase (overrides the default for the status code)
    pub reason: Option<String>,
    /// The headers of the response
    pub headers: Vec<Header>,
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

    /// Adds a header to the response
    pub fn with_header(mut self, header: Header) -> Self {
        self.headers.push(header);
        self
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

    /// Retrieves the first header with the specified name, if any
    pub fn header(&self, name: &HeaderName) -> Option<&Header> {
        self.headers.iter().find(|h| &h.name == name)
    }

    /// Gets the reason phrase for this response (either the custom one or the default)
    pub fn reason_phrase(&self) -> &str {
        self.reason.as_deref().unwrap_or_else(|| self.status.reason_phrase())
    }
    
    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&str> {
        self.header(&HeaderName::CallId).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        self.header(&HeaderName::From).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        self.header(&HeaderName::To).and_then(|h| h.value.as_text())
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
    pub fn headers(&self) -> &[Header] {
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

    /// Retrieves the first header with the specified name, if any
    pub fn header(&self, name: &HeaderName) -> Option<&Header> {
        self.headers().iter().find(|h| &h.name == name)
    }

    /// Retrieves the Call-ID header value, if present
    pub fn call_id(&self) -> Option<&str> {
        self.header(&HeaderName::CallId).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the From header value, if present
    pub fn from(&self) -> Option<&str> {
        self.header(&HeaderName::From).and_then(|h| h.value.as_text())
    }
    
    /// Retrieves the To header value, if present
    pub fn to(&self) -> Option<&str> {
        self.header(&HeaderName::To).and_then(|h| h.value.as_text())
    }

    /// Convert the message to bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        
        match self {
            Message::Request(request) => {
                // Add request line: METHOD URI SIP/2.0\r\n
                bytes.extend_from_slice(format!("{} {} SIP/2.0\r\n", 
                    request.method, request.uri).as_bytes());
                
                // Add headers
                for header in &request.headers {
                    bytes.extend_from_slice(format!("{}: {}\r\n", 
                        header.name, header.value).as_bytes());
                }
                
                // Add empty line to separate headers from body
                bytes.extend_from_slice(b"\r\n");
                
                // Add body if any
                bytes.extend_from_slice(&request.body);
            },
            Message::Response(response) => {
                // Add status line: SIP/2.0 CODE REASON\r\n
                bytes.extend_from_slice(format!("SIP/2.0 {} {}\r\n", 
                    response.status.as_u16(), response.status.reason_phrase()).as_bytes());
                
                // Add headers
                for header in &response.headers {
                    bytes.extend_from_slice(format!("{}: {}\r\n", 
                        header.name, header.value).as_bytes());
                }
                
                // Add empty line to separate headers from body
                bytes.extend_from_slice(b"\r\n");
                
                // Add body if any
                bytes.extend_from_slice(&response.body);
            }
        }
        
        bytes
    }

    /// Parse a SIP message from bytes
    pub fn parse(data: &[u8]) -> Result<Self> {
        // Convert bytes to string for parsing
        let text = match std::str::from_utf8(data) {
            Ok(text) => text,
            Err(e) => return Err(Error::ParseError(format!("Invalid UTF-8 data: {}", e))),
        };
        
        // Split the message into lines
        let mut lines = text.lines();
        
        // Parse the first line (request-line or status-line)
        let first_line = match lines.next() {
            Some(line) => line,
            None => return Err(Error::ParseError("Empty message".to_string())),
        };
        
        // Check if it's a request or response
        if first_line.starts_with("SIP/") {
            // It's a response
            Self::parse_response(first_line, lines, data)
        } else {
            // It's a request
            Self::parse_request(first_line, lines, data)
        }
    }
    
    // Parse a SIP request
    fn parse_request<'a, I>(request_line: &str, lines: I, data: &[u8]) -> Result<Self> 
    where
        I: Iterator<Item = &'a str>,
    {
        // Parse request line
        let parts: Vec<&str> = request_line.split_whitespace().collect();
        if parts.len() != 3 {
            return Err(Error::ParseError(format!("Invalid request line: {}", request_line)));
        }
        
        // Parse method
        let method = Method::from_str(parts[0])?;
        
        // Parse URI
        let uri = Uri::from_str(parts[1])?;
        
        // Parse version
        let version = Version::from_str(parts[2])?;
        
        // Parse headers and body
        let (headers, body) = Self::parse_headers_and_body(lines, data)?;
        
        Ok(Message::Request(Request {
            method,
            uri,
            version,
            headers,
            body,
        }))
    }
    
    // Parse a SIP response
    fn parse_response<'a, I>(status_line: &str, lines: I, data: &[u8]) -> Result<Self> 
    where
        I: Iterator<Item = &'a str>,
    {
        // Parse status line
        let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
        if parts.len() < 3 {
            return Err(Error::ParseError(format!("Invalid status line: {}", status_line)));
        }
        
        // Parse version
        let version = Version::from_str(parts[0])?;
        
        // Parse status code
        let status_code = parts[1].parse::<u16>()
            .map_err(|_| Error::ParseError(format!("Invalid status code: {}", parts[1])))?;
        let status = StatusCode::from_u16(status_code)?;
        
        // Parse reason phrase
        let reason = Some(parts[2].to_string());
        
        // Parse headers and body
        let (headers, body) = Self::parse_headers_and_body(lines, data)?;
        
        Ok(Message::Response(Response {
            version,
            status,
            reason,
            headers,
            body,
        }))
    }
    
    // Parse headers and body
    fn parse_headers_and_body<'a, I>(mut lines: I, data: &[u8]) -> Result<(Vec<Header>, Bytes)> 
    where
        I: Iterator<Item = &'a str>,
    {
        let mut headers = Vec::new();
        let mut header_lines = Vec::new();
        
        // Collect header lines
        for line in &mut lines {
            if line.is_empty() {
                // Empty line marks the end of headers
                break;
            }
            header_lines.push(line);
        }
        
        // Parse headers
        for line in header_lines {
            if let Some(pos) = line.find(':') {
                let name = &line[..pos].trim();
                let value = &line[pos+1..].trim();
                
                let header_name = HeaderName::from_str(name)?;
                let header_value = HeaderValue::text(value.to_string());
                
                headers.push(Header::new(header_name, header_value));
            } else {
                return Err(Error::ParseError(format!("Invalid header line: {}", line)));
            }
        }
        
        // Find body in original data
        let body = if let Some(body_pos) = find_body_position(data) {
            Bytes::copy_from_slice(&data[body_pos..])
        } else {
            Bytes::new()
        };
        
        Ok((headers, body))
    }
}

// Helper function to find the start of the body in the raw data
fn find_body_position(data: &[u8]) -> Option<usize> {
    // Look for double CRLF which marks the end of headers
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == b'\r' && data[i+1] == b'\n' && data[i+2] == b'\r' && data[i+3] == b'\n' {
            return Some(i + 4);
        }
        i += 1;
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_code_properties() {
        assert!(StatusCode::Trying.is_provisional());
        assert!(!StatusCode::Trying.is_success());
        
        assert!(StatusCode::Ok.is_success());
        assert!(!StatusCode::Ok.is_error());
        
        assert!(StatusCode::BadRequest.is_client_error());
        assert!(StatusCode::BadRequest.is_error());
        
        assert!(StatusCode::ServerInternalError.is_server_error());
        assert!(StatusCode::ServerInternalError.is_error());
        
        assert!(StatusCode::Decline.is_global_failure());
        assert!(StatusCode::Decline.is_error());
    }

    #[test]
    fn test_request_creation() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let request = Request::new(Method::Invite, uri.clone())
            .with_header(Header::text(HeaderName::From, "sip:alice@example.com"))
            .with_header(Header::text(HeaderName::To, "sip:bob@example.com"))
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"))
            .with_header(Header::text(HeaderName::CSeq, "1 INVITE"));
            
        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri, uri);
        assert_eq!(request.version, Version::sip_2_0());
        assert_eq!(request.headers.len(), 4);
        
        assert_eq!(request.call_id(), Some("abc123@example.com"));
        assert_eq!(request.from(), Some("sip:alice@example.com"));
        assert_eq!(request.to(), Some("sip:bob@example.com"));
        assert_eq!(request.cseq(), Some("1 INVITE"));
    }

    #[test]
    fn test_response_creation() {
        let response = Response::ok()
            .with_header(Header::text(HeaderName::From, "sip:alice@example.com"))
            .with_header(Header::text(HeaderName::To, "sip:bob@example.com;tag=789"))
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"))
            .with_header(Header::text(HeaderName::CSeq, "1 INVITE"));
            
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.version, Version::sip_2_0());
        assert_eq!(response.headers.len(), 4);
        assert_eq!(response.reason_phrase(), "OK");
        
        // Custom reason phrase
        let response = Response::new(StatusCode::Ok)
            .with_reason("Everything is fine");
        assert_eq!(response.reason_phrase(), "Everything is fine");
    }

    #[test]
    fn test_message_enum() {
        let uri = Uri::from_str("sip:bob@example.com").unwrap();
        let request = Request::new(Method::Invite, uri)
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"));
            
        let response = Response::ok()
            .with_header(Header::text(HeaderName::CallId, "abc123@example.com"));
            
        let req_msg = Message::Request(request);
        let resp_msg = Message::Response(response);
        
        assert!(req_msg.is_request());
        assert!(!req_msg.is_response());
        assert!(resp_msg.is_response());
        assert!(!resp_msg.is_request());
        
        assert_eq!(req_msg.method(), Some(Method::Invite));
        assert_eq!(resp_msg.status(), Some(StatusCode::Ok));
        
        assert_eq!(req_msg.call_id(), Some("abc123@example.com"));
        assert_eq!(resp_msg.call_id(), Some("abc123@example.com"));
    }
} 