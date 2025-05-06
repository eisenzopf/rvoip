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
};

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
///
/// let sdp_body = "v=0\r\no=bob 2890844527 2890844527 IN IP4 bob.example.com\r\ns=Session\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
///
/// let response = SimpleResponseBuilder::ok()
///     .from("Bob", "sip:bob@example.com", Some("a6c85cf"))
///     .to("Alice", "sip:alice@example.com", Some("1928301774"))
///     .call_id("a84b4c76e66710@pc33.atlanta.com")
///     .cseq(1, Method::Invite)
///     .content_type("application/sdp")
///     .body(sdp_body)
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::ok();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::trying();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::ringing();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::bad_request();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::not_found();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleResponseBuilder;
    ///
    /// let builder = SimpleResponseBuilder::server_error();
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
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.response = self.response.with_header(TypedHeader::CallId(CallId::new(call_id)));
        self
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
    pub fn cseq(mut self, seq: u32, method: Method) -> Self {
        self.response = self.response.with_header(
            TypedHeader::CSeq(CSeq::new(seq, method))
        );
        self
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
        self.response = self.response.with_header(header);
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
    ///
    /// let sdp_body = "v=0\r\no=bob 2890844527 2890844527 IN IP4 127.0.0.1\r\ns=Session\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
    ///
    /// let builder = SimpleResponseBuilder::new(StatusCode::Ok, None)
    ///     .content_type("application/sdp")
    ///     .body(sdp_body);
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