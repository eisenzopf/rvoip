use std::str::FromStr;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::{
    Method, 
    StatusCode, 
    Version,
    sip_request::Request,
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
    max_forwards::MaxForwards,
};

/// A simplified builder for SIP requests with improved method chaining.
///
/// This builder approach avoids returning different builder types for specific headers,
/// allowing for more straightforward method chaining and making it easier to use in macros.
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(314159)
///     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
///     .max_forwards(70)
///     .contact("sip:alice@pc33.atlanta.com", None)
///     .content_type("application/sdp")
///     .body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n")
///     .build();
/// ```
pub struct SimpleRequestBuilder {
    request: Request,
}

impl SimpleRequestBuilder {
    /// Create a new SimpleRequestBuilder with the specified method and URI
    ///
    /// # Parameters
    /// - `method`: The SIP method (INVITE, REGISTER, etc.)
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the SimpleRequestBuilder or an error if the URI is invalid
    pub fn new(method: Method, uri: &str) -> Result<Self> {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let request = Request::new(method, uri);
                Ok(Self { request })
            },
            Err(e) => Err(Error::InvalidUri(format!("Invalid URI: {}", e))),
        }
    }
    
    /// Create from an existing Request object
    pub fn from_request(request: Request) -> Self {
        Self { request }
    }
    
    /// Create an INVITE request
    pub fn invite(uri: &str) -> Result<Self> {
        Self::new(Method::Invite, uri)
    }
    
    /// Create a REGISTER request
    pub fn register(uri: &str) -> Result<Self> {
        Self::new(Method::Register, uri)
    }
    
    /// Create a BYE request
    pub fn bye(uri: &str) -> Result<Self> {
        Self::new(Method::Bye, uri)
    }
    
    /// Create an OPTIONS request
    pub fn options(uri: &str) -> Result<Self> {
        Self::new(Method::Options, uri)
    }
    
    /// Create an ACK request
    pub fn ack(uri: &str) -> Result<Self> {
        Self::new(Method::Ack, uri)
    }
    
    /// Create a CANCEL request
    pub fn cancel(uri: &str) -> Result<Self> {
        Self::new(Method::Cancel, uri)
    }

    /// Add a From header with optional tag parameter
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header
    /// - `uri`: The URI for the From header
    /// - `tag`: Optional tag parameter
    ///
    /// # Returns
    /// Self for method chaining
    pub fn from(mut self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.request = self.request.with_header(TypedHeader::From(From::new(address)));
                self
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.request = self.request.with_header(TypedHeader::From(From::new(address)));
                self
            }
        }
    }
    
    /// Add a To header with optional tag parameter
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header
    /// - `uri`: The URI for the To header
    /// - `tag`: Optional tag parameter
    ///
    /// # Returns
    /// Self for method chaining
    pub fn to(mut self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.request = self.request.with_header(TypedHeader::To(To::new(address)));
                self
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.request = self.request.with_header(TypedHeader::To(To::new(address)));
                self
            }
        }
    }
    
    /// Add a Call-ID header
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.request = self.request.with_header(TypedHeader::CallId(CallId::new(call_id)));
        self
    }
    
    /// Add a CSeq header for requests
    ///
    /// # Parameters
    /// - `seq`: The sequence number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn cseq(mut self, seq: u32) -> Self {
        let method = self.request.method.clone();
        self.request = self.request.with_header(
            TypedHeader::CSeq(CSeq::new(seq, method))
        );
        self
    }
    
    /// Add a Via header with optional branch parameter
    ///
    /// # Parameters
    /// - `host`: The host or IP address
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be prefixed with z9hG4bK per RFC 3261)
    ///
    /// # Returns
    /// Self for method chaining
    pub fn via(mut self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        let mut params = Vec::new();
        
        // Add branch parameter if provided
        if let Some(branch_value) = branch {
            params.push(Param::branch(branch_value));
        }
        
        // Parse host to separate hostname and port
        let (hostname, port) = if host.contains(':') {
            let parts: Vec<&str> = host.split(':').collect();
            if parts.len() == 2 {
                if let Ok(port_num) = parts[1].parse::<u16>() {
                    (parts[0].to_string(), Some(port_num))
                } else {
                    (host.to_string(), None)
                }
            } else {
                (host.to_string(), None)
            }
        } else {
            (host.to_string(), None)
        };
        
        // Create Via header
        if let Ok(via) = Via::new("SIP", "2.0", transport, &hostname, port, params) {
            self.request = self.request.with_header(TypedHeader::Via(via));
        }
        
        self
    }
    
    /// Add a Max-Forwards header
    ///
    /// # Parameters
    /// - `value`: The Max-Forwards value (typically 70)
    ///
    /// # Returns
    /// Self for method chaining
    pub fn max_forwards(mut self, value: u32) -> Self {
        self.request = self.request.with_header(
            TypedHeader::MaxForwards(MaxForwards::new(value as u8))
        );
        self
    }
    
    /// Add a Contact header
    ///
    /// # Parameters
    /// - `uri`: The contact URI as a string
    /// - `display_name`: Optional display name
    ///
    /// # Returns
    /// Self for method chaining
    pub fn contact(mut self, uri: &str, display_name: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                // Create an address with or without display name
                let address = match display_name {
                    Some(name) => Address::new_with_display_name(name, uri),
                    None => Address::new(uri)
                };
                
                // Create a contact param with the address
                let contact_param = ContactParamInfo { address };
                let contact = Contact::new_params(vec![contact_param]);
                
                self.request = self.request.with_header(TypedHeader::Contact(contact));
            },
            Err(_) => {
                // Silently fail - contact is not critical
            }
        }
        self
    }
    
    /// Add a Content-Type header
    ///
    /// # Parameters
    /// - `content_type`: The content type (e.g., "application/sdp")
    ///
    /// # Returns
    /// Self for method chaining
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
    
    /// Add a generic header
    ///
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.request = self.request.with_header(header);
        self
    }
    
    /// Add body content and update Content-Length
    ///
    /// # Parameters
    /// - `body`: The body content
    ///
    /// # Returns
    /// Self for method chaining
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.request = self.request.with_body(body);
        self
    }
    
    /// Build the final Request
    ///
    /// # Returns
    /// The constructed Request
    pub fn build(self) -> Request {
        self.request
    }
}

/// A simplified builder for SIP responses with improved method chaining.
///
/// This builder approach avoids returning different builder types for specific headers,
/// allowing for more straightforward method chaining and making it easier to use in macros.
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
///     .contact("sip:bob@192.168.1.2", None)
///     .content_type("application/sdp")
///     .body("v=0\r\no=bob 123 456 IN IP4 192.168.1.2\r\ns=A call\r\nt=0 0\r\n")
///     .build();
/// ```
pub struct SimpleResponseBuilder {
    response: Response,
}

impl SimpleResponseBuilder {
    /// Create a new SimpleResponseBuilder with the specified status code and optional reason phrase
    ///
    /// # Parameters
    /// - `status`: The SIP status code
    /// - `reason`: Optional custom reason phrase (if None, the default for the status code will be used)
    ///
    /// # Returns
    /// A new SimpleResponseBuilder
    pub fn new(status: StatusCode, reason: Option<&str>) -> Self {
        let mut response = Response::new(status);
        
        if let Some(reason_text) = reason {
            response = response.with_reason(reason_text);
        }
        
        Self { response }
    }
    
    /// Create from an existing Response object
    pub fn from_response(response: Response) -> Self {
        Self { response }
    }
    
    /// Create a 200 OK response
    pub fn ok() -> Self {
        let response = Response::ok();
        Self { response }
    }
    
    /// Create a 100 Trying response
    pub fn trying() -> Self {
        let response = Response::trying();
        Self { response }
    }
    
    /// Create a 180 Ringing response
    pub fn ringing() -> Self {
        let response = Response::ringing();
        Self { response }
    }
    
    /// Create a 400 Bad Request response
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BadRequest, None)
    }
    
    /// Create a 404 Not Found response
    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound, None)
    }
    
    /// Create a 500 Server Error response
    pub fn server_error() -> Self {
        Self::new(StatusCode::ServerInternalError, None)
    }

    /// Add a From header with optional tag parameter
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header
    /// - `uri`: The URI for the From header
    /// - `tag`: Optional tag parameter
    ///
    /// # Returns
    /// Self for method chaining
    pub fn from(mut self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.response = self.response.with_header(TypedHeader::From(From::new(address)));
                self
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.response = self.response.with_header(TypedHeader::From(From::new(address)));
                self
            }
        }
    }
    
    /// Add a To header with optional tag parameter
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header
    /// - `uri`: The URI for the To header
    /// - `tag`: Optional tag parameter
    ///
    /// # Returns
    /// Self for method chaining
    pub fn to(mut self, display_name: &str, uri: &str, tag: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut address = Address::new_with_display_name(display_name, uri);
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.response = self.response.with_header(TypedHeader::To(To::new(address)));
                self
            },
            Err(_) => {
                // Best effort - if URI parsing fails, still try to continue with a simple string
                let uri_str = uri.to_string();
                let mut address = Address::new_with_display_name(display_name, Uri::custom(&uri_str));
                
                // Add tag if provided
                if let Some(tag_value) = tag {
                    address.params.push(Param::tag(tag_value));
                }
                
                self.response = self.response.with_header(TypedHeader::To(To::new(address)));
                self
            }
        }
    }
    
    /// Add a Call-ID header
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.response = self.response.with_header(TypedHeader::CallId(CallId::new(call_id)));
        self
    }
    
    /// Add a CSeq header for responses (requires method)
    ///
    /// # Parameters
    /// - `seq`: The sequence number
    /// - `method`: The method in the CSeq header
    ///
    /// # Returns
    /// Self for method chaining
    pub fn cseq(mut self, seq: u32, method: Method) -> Self {
        self.response = self.response.with_header(
            TypedHeader::CSeq(CSeq::new(seq, method))
        );
        self
    }
    
    /// Add a Via header with optional branch parameter
    ///
    /// # Parameters
    /// - `host`: The host or IP address
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    /// - `branch`: Optional branch parameter (should be prefixed with z9hG4bK per RFC 3261)
    ///
    /// # Returns
    /// Self for method chaining
    pub fn via(mut self, host: &str, transport: &str, branch: Option<&str>) -> Self {
        let mut params = Vec::new();
        
        // Add branch parameter if provided
        if let Some(branch_value) = branch {
            params.push(Param::branch(branch_value));
        }
        
        // Parse host to separate hostname and port
        let (hostname, port) = if host.contains(':') {
            let parts: Vec<&str> = host.split(':').collect();
            if parts.len() == 2 {
                if let Ok(port_num) = parts[1].parse::<u16>() {
                    (parts[0].to_string(), Some(port_num))
                } else {
                    (host.to_string(), None)
                }
            } else {
                (host.to_string(), None)
            }
        } else {
            (host.to_string(), None)
        };
        
        // Create Via header
        if let Ok(via) = Via::new("SIP", "2.0", transport, &hostname, port, params) {
            self.response = self.response.with_header(TypedHeader::Via(via));
        }
        
        self
    }
    
    /// Add a Contact header
    ///
    /// # Parameters
    /// - `uri`: The contact URI as a string
    /// - `display_name`: Optional display name
    ///
    /// # Returns
    /// Self for method chaining
    pub fn contact(mut self, uri: &str, display_name: Option<&str>) -> Self {
        match Uri::from_str(uri) {
            Ok(uri) => {
                // Create an address with or without display name
                let address = match display_name {
                    Some(name) => Address::new_with_display_name(name, uri),
                    None => Address::new(uri)
                };
                
                // Create a contact param with the address
                let contact_param = ContactParamInfo { address };
                let contact = Contact::new_params(vec![contact_param]);
                
                self.response = self.response.with_header(TypedHeader::Contact(contact));
            },
            Err(_) => {
                // Silently fail - contact is not critical
            }
        }
        self
    }
    
    /// Add a Content-Type header
    ///
    /// # Parameters
    /// - `content_type`: The content type (e.g., "application/sdp")
    ///
    /// # Returns
    /// Self for method chaining
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
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.response = self.response.with_header(header);
        self
    }
    
    /// Add body content and update Content-Length
    ///
    /// # Parameters
    /// - `body`: The body content
    ///
    /// # Returns
    /// Self for method chaining
    pub fn body(mut self, body: impl Into<Bytes>) -> Self {
        self.response = self.response.with_body(body);
        self
    }
    
    /// Build the final Response
    ///
    /// # Returns
    /// The constructed Response
    pub fn build(self) -> Response {
        self.response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_request_builder() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("1928301774"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("a84b4c76e66710@pc33.atlanta.com")
            .cseq(314159)
            .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
            .max_forwards(70)
            .build();
            
        assert_eq!(request.method, Method::Invite);
        assert_eq!(request.uri.to_string(), "sip:bob@example.com");
        
        // Check From header
        let from = request.from().unwrap();
        assert_eq!(from.address().display_name(), Some("Alice"));
        assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
        assert_eq!(from.tag(), Some("1928301774"));
        
        // Check To header
        let to = request.to().unwrap();
        assert_eq!(to.address().display_name(), Some("Bob"));
        assert_eq!(to.address().uri.to_string(), "sip:bob@example.com");
        assert_eq!(to.tag(), None);
        
        // Check Call-ID header
        let call_id = request.call_id().unwrap();
        assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.com");
        
        // Check CSeq header
        let cseq = request.cseq().unwrap();
        assert_eq!(cseq.sequence(), 314159);
        assert_eq!(*cseq.method(), Method::Invite);
        
        // Check Via header
        let via = request.first_via().unwrap();
        assert_eq!(via.0[0].sent_protocol.transport, "UDP");
        assert_eq!(via.0[0].sent_by_host.to_string(), "pc33.atlanta.com");
        assert!(via.branch().is_some());
        assert_eq!(via.branch().unwrap(), "z9hG4bK776asdhds");
        
        // Check Max-Forwards header
        let max_forwards = request.typed_header::<MaxForwards>().unwrap();
        assert_eq!(max_forwards.0, 70);
    }
    
    #[test]
    fn test_simple_response_builder() {
        let response = SimpleResponseBuilder::new(StatusCode::Ok, Some("OK"))
            .from("Alice", "sip:alice@example.com", Some("1928301774"))
            .to("Bob", "sip:bob@example.com", Some("a6c85cf"))
            .call_id("a84b4c76e66710@pc33.atlanta.com")
            .cseq(1, Method::Invite)
            .via("pc33.atlanta.com", "UDP", Some("z9hG4bK776asdhds"))
            .build();
            
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.reason, Some("OK".to_string()));
        
        // Check From header
        let from = response.from().unwrap();
        assert_eq!(from.address().display_name(), Some("Alice"));
        assert_eq!(from.address().uri.to_string(), "sip:alice@example.com");
        assert_eq!(from.tag(), Some("1928301774"));
        
        // Check To header
        let to = response.to().unwrap();
        assert_eq!(to.address().display_name(), Some("Bob"));
        assert_eq!(to.address().uri.to_string(), "sip:bob@example.com");
        assert_eq!(to.tag(), Some("a6c85cf"));
        
        // Check Call-ID header
        let call_id = response.call_id().unwrap();
        assert_eq!(call_id.value(), "a84b4c76e66710@pc33.atlanta.com");
        
        // Check CSeq header
        let cseq = response.cseq().unwrap();
        assert_eq!(cseq.sequence(), 1);
        assert_eq!(*cseq.method(), Method::Invite);
        
        // Check Via header
        let via = response.first_via().unwrap();
        assert_eq!(via.0[0].sent_protocol.transport, "UDP");
        assert_eq!(via.0[0].sent_by_host.to_string(), "pc33.atlanta.com");
        assert!(via.branch().is_some());
        assert_eq!(via.branch().unwrap(), "z9hG4bK776asdhds");
    }
    
    #[test]
    fn test_with_body_and_content_type() {
        let request = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
            .content_type("application/sdp")
            .body("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n")
            .build();
            
        // Check Content-Type header
        let content_type = request.typed_header::<ContentType>().unwrap();
        assert_eq!(content_type.to_string(), "application/sdp");
        
        // Check body
        assert_eq!(
            String::from_utf8_lossy(&request.body),
            "v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\ns=A call\r\nt=0 0\r\n"
        );
        
        // Check Content-Length header
        let content_length = request.typed_header::<ContentLength>().unwrap();
        assert_eq!(content_length.0 as usize, request.body.len());
    }
    
    #[test]
    fn test_uri_parsing_error_handling() {
        // Test with invalid URI
        let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .from("Alice", "invalid-uri", Some("1928301774"))
            .to("Bob", "another-invalid-uri", None)
            .build();
            
        // The builder should still create headers with best effort parsing
        assert!(request.from().is_some());
        assert!(request.to().is_some());
    }
} 