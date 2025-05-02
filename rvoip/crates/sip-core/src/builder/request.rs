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
};

/// The SimpleRequestBuilder provides a streamlined approach to creating SIP request messages.
///
/// # Examples
///
/// ## Creating a Basic INVITE Request
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
///
/// // Create a basic INVITE request
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
///     .unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("a84b4c76e66710")
///     .cseq(1)
///     .via("alice.example.com", "UDP", Some("z9hG4bK776asdhds"))
///     .max_forwards(70)
///     .build();
///
/// assert_eq!(request.method(), Method::Invite);
/// ```
///
/// ## Creating Method-Specific Requests
///
/// You can use convenience constructors for common methods:
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
///
/// // INVITE request
/// let invite = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("a84b4c76e66710")
///     .build();
///
/// // REGISTER request
/// let register = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Alice", "sip:alice@example.com", None)
///     .call_id("register-call-id")
///     .contact("sip:alice@192.168.0.2:5060", None)
///     .build();
///
/// // BYE request
/// let bye = SimpleRequestBuilder::bye("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", Some("bob-tag"))
///     .call_id("a84b4c76e66710")
///     .build();
///
/// // OPTIONS request
/// let options = SimpleRequestBuilder::options("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("options-call-id")
///     .build();
/// ```
///
/// ## Adding a Message Body with Content-Type
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::Method;
///
/// let sdp_body = "v=0\r\no=alice 2890844526 2890844526 IN IP4 alice.example.org\r\ns=SIP Call\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
///
/// let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
///     .unwrap()
///     .content_type("application/sdp")
///     .body(sdp_body)
///     .build();
///
/// assert_eq!(request.method(), Method::Invite);
/// ```
///
/// ## Creating a REGISTER Request with Contact and Expires
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::TypedHeader;
/// use rvoip_sip_core::types::expires::Expires;
///
/// let request = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Alice", "sip:alice@example.com", None)
///     .call_id("register-call-id")
///     .contact("sip:alice@192.168.0.2:5060", None)
///     .header(TypedHeader::Expires(Expires::new(3600)))
///     .build();
/// ```
///
/// ## Creating a Request with Authentication
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::TypedHeader;
/// use rvoip_sip_core::types::auth::{Authorization, AuthScheme};
/// use rvoip_sip_core::types::Uri;
/// use std::str::FromStr;
///
/// // Create an Authorization header
/// let uri = Uri::from_str("sip:registrar.example.com").unwrap();
/// let auth = Authorization::new(
///     AuthScheme::Digest,
///     "alice",
///     "example.com",
///     "dcd98b7102dd2f0e8b11d0f600bfb0c093",
///     uri,
///     "a2ea68c230e5fea1ca715740fb14db97"
/// );
///
/// // Add it to a REGISTER request
/// let request = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Alice", "sip:alice@example.com", None)
///     .call_id("register-call-id")
///     .header(TypedHeader::Authorization(auth))
///     .build();
/// ```
///
/// ## Adding Headers for Features and Capabilities
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::{Method, TypedHeader};
/// use rvoip_sip_core::types::allow::Allow;
/// use rvoip_sip_core::types::supported::Supported;
///
/// // Create an Allow header properly with methods added after construction
/// let mut allow = Allow::new();
/// allow.add_method(Method::Invite);
/// allow.add_method(Method::Ack);
/// allow.add_method(Method::Cancel);
/// allow.add_method(Method::Bye);
/// allow.add_method(Method::Options);
///
/// let request = SimpleRequestBuilder::new(Method::Options, "sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .call_id("options-call-id")
///     .header(TypedHeader::Allow(allow))
///     .header(TypedHeader::Supported(Supported::new(vec![
///         "path".to_string(), "outbound".to_string(), "gruu".to_string()
///     ])))
///     .build();
/// ```
///
/// ## Creating from an Existing Request
///
/// ```rust
/// use rvoip_sip_core::builder::SimpleRequestBuilder;
/// use rvoip_sip_core::types::{Method, Uri, Request};
/// use std::str::FromStr;
///
/// // Create or get a request from somewhere
/// let uri = Uri::from_str("sip:bob@example.com").unwrap();
/// let existing_request = Request::new(Method::Invite, uri);
///
/// // Build on top of it
/// let modified_request = SimpleRequestBuilder::from_request(existing_request)
///     .from("Alice", "sip:alice@example.com", Some("1928301774"))
///     .to("Bob", "sip:bob@example.com", None)
///     .build();
/// ```
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::invite("sip:bob@example.com").unwrap();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::register("sip:registrar.example.com").unwrap();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::bye("sip:bob@example.com").unwrap();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::options("sip:bob@example.com").unwrap();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::ack("sip:bob@example.com").unwrap();
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
    /// # Example
    ///
    /// ```rust
    /// use rvoip_sip_core::builder::SimpleRequestBuilder;
    ///
    /// let builder = SimpleRequestBuilder::cancel("sip:bob@example.com").unwrap();
    /// ```
    pub fn cancel(uri: &str) -> Result<Self> {
        Self::new(Method::Cancel, uri)
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
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.request = self.request.with_header(TypedHeader::CallId(CallId::new(call_id)));
        self
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
    pub fn cseq(mut self, seq: u32) -> Self {
        let method = self.request.method.clone();
        self.request = self.request.with_header(
            TypedHeader::CSeq(CSeq::new(seq, method))
        );
        self
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
    pub fn max_forwards(mut self, value: u32) -> Self {
        self.request = self.request.with_header(
            TypedHeader::MaxForwards(MaxForwards::new(value as u8))
        );
        self
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
    /// let user_agent = UserAgent::new();  // Create empty User-Agent header
    /// 
    /// let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
    ///     .header(TypedHeader::UserAgent(user_agent));
    /// ```
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.request = self.request.with_header(header);
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