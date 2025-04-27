use std::str::FromStr;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::{
    Method, 
    StatusCode, 
    Version,
    sip_message::{Request, Response},
    uri::{Uri, Host, Scheme},
    to::To,
    from::From,
    call_id::CallId,
    cseq::CSeq,
    contact::{Contact, ContactParamInfo},
    content_type::ContentType,
    content_length::ContentLength,
    via::{Via, ViaHeader, SentProtocol},
    Address,
    TypedHeader,
    Param,
    max_forwards::MaxForwards,
};
use crate::parser::uri::parse_uri_lenient;

/// Builder for SIP request messages
pub struct RequestBuilder {
    request: Request,
}

impl RequestBuilder {
    /// Create a new RequestBuilder with the specified method and URI
    pub fn new(method: Method, uri: &str) -> Result<Self> {
        // First try parsing as a standard URI
        match Uri::from_str(uri) {
            Ok(uri) => {
                let mut request = Request::new(method, uri);
                request.version = Version::new(2, 0); // Default to SIP/2.0
                Ok(Self { request })
            },
            Err(_) => {
                // Use our lenient parser for non-standard schemes like in RFC compliance tests
                match parse_uri_lenient(uri.as_bytes()) {
                    Ok((_, uri)) => {
                        let mut request = Request::new(method, uri);
                        request.version = Version::new(2, 0); // Default to SIP/2.0
                        Ok(Self { request })
                    },
                    Err(_) => {
                        // If not even the lenient parser works, check if it has a colon
                        if uri.contains(':') {
                            // Just create a custom URI that preserves the raw string
                            let uri = Uri::custom(uri);
                            let mut request = Request::new(method, uri);
                            request.version = Version::new(2, 0); // Default to SIP/2.0
                            Ok(Self { request })
                        } else {
                            // It's not a valid URI and doesn't have a scheme, so report error
                            Err(Error::InvalidUri("URI missing scheme".to_string()))
                        }
                    }
                }
            }
        }
    }
    
    /// Create a RequestBuilder from an existing Request
    ///
    /// # Parameters
    /// - `request`: An existing Request object to build upon
    ///
    /// # Returns
    /// A new RequestBuilder containing the provided request
    pub fn from_request(request: Request) -> Self {
        Self { request }
    }

    /// Create an INVITE request
    ///
    /// The INVITE method is used to establish a media session between user agents.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn invite(uri: &str) -> Result<Self> {
        Self::new(Method::Invite, uri)
    }

    /// Create a REGISTER request
    ///
    /// The REGISTER method is used by a user agent to register its address with a SIP server.
    ///
    /// # Parameters
    /// - `uri`: The registrar URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn register(uri: &str) -> Result<Self> {
        Self::new(Method::Register, uri)
    }

    /// Create an OPTIONS request
    ///
    /// The OPTIONS method is used to query the capabilities of a server.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn options(uri: &str) -> Result<Self> {
        Self::new(Method::Options, uri)
    }

    /// Create a BYE request
    ///
    /// The BYE method is used to terminate a SIP dialog.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn bye(uri: &str) -> Result<Self> {
        Self::new(Method::Bye, uri)
    }

    /// Create an ACK request
    ///
    /// The ACK method is used to acknowledge receipt of a final response to an INVITE.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn ack(uri: &str) -> Result<Self> {
        Self::new(Method::Ack, uri)
    }

    /// Create a CANCEL request
    ///
    /// The CANCEL method is used to cancel a pending INVITE request.
    ///
    /// # Parameters
    /// - `uri`: The target URI as a string
    ///
    /// # Returns
    /// A Result containing the RequestBuilder or an error if the URI is invalid
    pub fn cancel(uri: &str) -> Result<Self> {
        Self::new(Method::Cancel, uri)
    }

    /// Set the SIP version (default is 2.0)
    ///
    /// # Parameters
    /// - `major`: Major version number
    /// - `minor`: Minor version number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn version(mut self, major: u8, minor: u8) -> Self {
        self.request.version = Version::new(major, minor);
        self
    }

    /// Add a Via header
    ///
    /// Returns a specialized builder for constructing the Via header.
    ///
    /// # Parameters
    /// - `host`: The host or IP address
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    ///
    /// # Returns
    /// A ViaBuilder for configuring the Via header
    pub fn via(self, host: &str, transport: &str) -> ViaBuilder<Self> {
        ViaBuilder::new(self, host, transport)
    }

    /// Add a From header
    ///
    /// Returns a specialized builder for constructing the From header.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header
    /// - `uri`: The URI for the From header
    ///
    /// # Returns
    /// An AddressBuilder for configuring the From header
    pub fn from(self, display_name: &str, uri: &str) -> AddressBuilder<Self, FromHeader> {
        AddressBuilder::new(self, display_name, uri, FromHeader)
    }

    /// Add a To header
    ///
    /// Returns a specialized builder for constructing the To header.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header
    /// - `uri`: The URI for the To header
    ///
    /// # Returns
    /// An AddressBuilder for configuring the To header
    pub fn to(self, display_name: &str, uri: &str) -> AddressBuilder<Self, ToHeader> {
        AddressBuilder::new(self, display_name, uri, ToHeader)
    }

    /// Add a simple To header without parameters
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header
    /// - `uri`: The URI for the To header
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn simple_to(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let to_addr = Address::new(Some(display_name), uri);
        self.request = self.request.with_header(TypedHeader::To(To(to_addr)));
        Ok(self)
    }

    /// Add a simple From header without parameters
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header
    /// - `uri`: The URI for the From header
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn simple_from(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let from_addr = Address::new(Some(display_name), uri);
        self.request = self.request.with_header(TypedHeader::From(From(from_addr)));
        Ok(self)
    }

    /// Add a Call-ID header
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.request = self.request.with_header(TypedHeader::CallId(
            CallId(call_id.to_string())
        ));
        self
    }

    /// Add a CSeq header
    ///
    /// Automatically uses the same method as the request.
    ///
    /// # Parameters
    /// - `seq`: The sequence number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn cseq(mut self, seq: u32) -> Self {
        let method = self.request.method.clone();
        self.request = self.request.with_header(TypedHeader::CSeq(
            CSeq::new(seq, method)
        ));
        self
    }

    /// Add a Max-Forwards header
    ///
    /// # Parameters
    /// - `value`: The maximum number of hops
    ///
    /// # Returns
    /// Self for method chaining
    pub fn max_forwards(mut self, value: u32) -> Self {
        self.request = self.request.with_header(TypedHeader::MaxForwards(
            MaxForwards(value as u8)
        ));
        self
    }

    /// Add a Contact header
    ///
    /// # Parameters
    /// - `uri`: The URI for the Contact header (e.g., "sip:alice@192.168.1.1")
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn contact(mut self, uri: &str) -> Result<Self> {
        let contact_uri = Uri::from_str(uri)?;
        let contact_address = Address::new(None::<String>, contact_uri);
        let contact_param = ContactParamInfo { address: contact_address };
        self.request = self.request.with_header(TypedHeader::Contact(
            Contact::new_params(vec![contact_param])
        ));
        Ok(self)
    }

    /// Add a contact header with display name
    ///
    /// # Parameters
    /// - `display_name`: The display name for the Contact header
    /// - `uri`: The URI for the Contact header
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn contact_with_name(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let contact_uri = Uri::from_str(uri)?;
        let contact_address = Address::new(Some(display_name), contact_uri);
        let contact_param = ContactParamInfo { address: contact_address };
        self.request = self.request.with_header(TypedHeader::Contact(
            Contact::new_params(vec![contact_param])
        ));
        Ok(self)
    }

    /// Add a Content-Type header
    ///
    /// # Parameters
    /// - `content_type`: The content type string (e.g., "application/sdp")
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the content type is invalid
    pub fn content_type(mut self, content_type: &str) -> Result<Self> {
        self.request = self.request.with_header(TypedHeader::ContentType(
            ContentType::from_str(content_type)?
        ));
        Ok(self)
    }

    /// Add body content
    ///
    /// Automatically adds a Content-Length header based on the body length.
    ///
    /// # Parameters
    /// - `body`: The message body as a string
    ///
    /// # Returns
    /// Self for method chaining
    pub fn body(mut self, body: &str) -> Self {
        let content_length = body.len() as u32;
        self.request = self.request.with_header(TypedHeader::ContentLength(
            ContentLength(content_length)
        ));
        self.request.body = Bytes::from(body.to_string());
        self
    }

    /// Add a custom header
    ///
    /// # Parameters
    /// - `header`: The typed header to add
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
    ///     .header(TypedHeader::UserAgent("MyAgent/1.0".parse().unwrap()))
    ///     .build();
    /// ```
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.request = self.request.with_header(header);
        self
    }

    /// Build the final SIP request
    ///
    /// # Returns
    /// The constructed Request object
    pub fn build(self) -> Request {
        self.request
    }

    /// Internal method to update the request - used by sub-builders
    fn update_request(&mut self, request: Request) {
        self.request = request;
    }
}

/// Builder for SIP response messages
///
/// Provides a fluent API for creating SIP responses with proper headers and parameters.
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// let response = ResponseBuilder::ok()
///     .from("Alice", "sip:alice@example.com")
///         .with_tag("1928301774")
///         .done()
///     .to("Bob", "sip:bob@example.com")
///         .with_tag("a6c85cf")
///         .done()
///     .call_id("a84b4c76e66710")
///     .cseq(1, Method::Invite)
///     .build();
/// ```
pub struct ResponseBuilder {
    response: Response,
}

impl ResponseBuilder {
    /// Create a new ResponseBuilder with the specified status code
    ///
    /// # Parameters
    /// - `status`: The SIP status code for the response
    ///
    /// # Returns
    /// A new ResponseBuilder with the specified status
    ///
    /// # Examples
    ///
    /// ```
    /// use rvoip_sip_core::prelude::*;
    ///
    /// let builder = ResponseBuilder::new(StatusCode::Ok);
    /// ```
    pub fn new(status: StatusCode) -> Self {
        let mut response = Response::new(status);
        response.version = Version::new(2, 0); // Default to SIP/2.0
        
        // Set default reason phrase based on status code
        let reason = match status {
            StatusCode::Ok => "OK",
            StatusCode::Trying => "Trying",
            StatusCode::Ringing => "Ringing",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::Unauthorized => "Unauthorized",
            StatusCode::NotFound => "Not Found",
            StatusCode::ServerInternalError => "Internal Server Error",
            _ => "",
        };
        response.reason = if !reason.is_empty() { Some(reason.to_string()) } else { None };
        
        Self { response }
    }

    /// Create a 200 OK response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 200 OK
    pub fn ok() -> Self {
        Self::new(StatusCode::Ok)
    }

    /// Create a 100 Trying response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 100 Trying
    pub fn trying() -> Self {
        Self::new(StatusCode::Trying)
    }

    /// Create a 180 Ringing response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 180 Ringing
    pub fn ringing() -> Self {
        Self::new(StatusCode::Ringing)
    }

    /// Create a 400 Bad Request response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 400 Bad Request
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BadRequest)
    }

    /// Create a 404 Not Found response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 404 Not Found
    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound)
    }

    /// Create a 500 Internal Server Error response
    ///
    /// # Returns
    /// A new ResponseBuilder with status 500 Internal Server Error
    pub fn server_error() -> Self {
        Self::new(StatusCode::ServerInternalError)
    }
    
    /// Set the SIP version (default is 2.0)
    ///
    /// # Parameters
    /// - `major`: Major version number
    /// - `minor`: Minor version number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn version(mut self, major: u8, minor: u8) -> Self {
        self.response.version = Version::new(major, minor);
        self
    }

    /// Set the response reason phrase
    ///
    /// # Parameters
    /// - `reason`: The custom reason phrase
    ///
    /// # Returns
    /// Self for method chaining
    pub fn reason(mut self, reason: &str) -> Self {
        self.response.reason = Some(reason.to_string());
        self
    }

    /// Add a From header
    ///
    /// Returns a specialized builder for constructing the From header.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the From header
    /// - `uri`: The URI for the From header
    ///
    /// # Returns
    /// An AddressBuilder for configuring the From header
    pub fn from(self, display_name: &str, uri: &str) -> AddressBuilder<Self, FromHeader> {
        AddressBuilder::new(self, display_name, uri, FromHeader)
    }

    /// Add a To header
    ///
    /// Returns a specialized builder for constructing the To header.
    ///
    /// # Parameters
    /// - `display_name`: The display name for the To header
    /// - `uri`: The URI for the To header
    ///
    /// # Returns
    /// An AddressBuilder for configuring the To header
    pub fn to(self, display_name: &str, uri: &str) -> AddressBuilder<Self, ToHeader> {
        AddressBuilder::new(self, display_name, uri, ToHeader)
    }

    /// Add a Via header
    ///
    /// Returns a specialized builder for constructing the Via header.
    ///
    /// # Parameters
    /// - `host`: The host or IP address
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    ///
    /// # Returns
    /// A ViaBuilder for configuring the Via header
    pub fn via(self, host: &str, transport: &str) -> ViaBuilder<Self> {
        ViaBuilder::new(self, host, transport)
    }

    /// Add a Call-ID header
    ///
    /// # Parameters
    /// - `call_id`: The Call-ID value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.response = self.response.with_header(TypedHeader::CallId(
            CallId(call_id.to_string())
        ));
        self
    }

    /// Add a CSeq header
    ///
    /// # Parameters
    /// - `seq`: The sequence number
    /// - `method`: The SIP method for the CSeq
    ///
    /// # Returns
    /// Self for method chaining
    pub fn cseq(mut self, seq: u32, method: Method) -> Self {
        self.response = self.response.with_header(TypedHeader::CSeq(
            CSeq::new(seq, method)
        ));
        self
    }

    /// Add a Contact header
    ///
    /// # Parameters
    /// - `uri`: The URI for the Contact header
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn contact(mut self, uri: &str) -> Result<Self> {
        let contact_uri = Uri::from_str(uri)?;
        let contact_address = Address::new(None::<String>, contact_uri);
        let contact_param = ContactParamInfo { address: contact_address };
        self.response = self.response.with_header(TypedHeader::Contact(
            Contact::new_params(vec![contact_param])
        ));
        Ok(self)
    }

    /// Add a contact header with display name
    ///
    /// # Parameters
    /// - `display_name`: The display name for the Contact header
    /// - `uri`: The URI for the Contact header
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the URI is invalid
    pub fn contact_with_name(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let contact_uri = Uri::from_str(uri)?;
        let contact_address = Address::new(Some(display_name), contact_uri);
        let contact_param = ContactParamInfo { address: contact_address };
        self.response = self.response.with_header(TypedHeader::Contact(
            Contact::new_params(vec![contact_param])
        ));
        Ok(self)
    }

    /// Add a Content-Type header
    ///
    /// # Parameters
    /// - `content_type`: The content type string (e.g., "application/sdp")
    ///
    /// # Returns
    /// A Result containing Self for method chaining, or an error if the content type is invalid
    pub fn content_type(mut self, content_type: &str) -> Result<Self> {
        self.response = self.response.with_header(TypedHeader::ContentType(
            ContentType::from_str(content_type)?
        ));
        Ok(self)
    }

    /// Add body content
    ///
    /// Automatically adds a Content-Length header based on the body length.
    ///
    /// # Parameters
    /// - `body`: The message body as a string
    ///
    /// # Returns
    /// Self for method chaining
    pub fn body(mut self, body: &str) -> Self {
        let content_length = body.len() as u32;
        self.response = self.response.with_header(TypedHeader::ContentLength(
            ContentLength(content_length)
        ));
        self.response.body = Bytes::from(body.to_string());
        self
    }

    /// Add a custom header
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

    /// Build the final SIP response
    ///
    /// # Returns
    /// The constructed Response object
    pub fn build(self) -> Response {
        self.response
    }

    /// Internal method to update the response - used by sub-builders
    fn update_response(&mut self, response: Response) {
        self.response = response;
    }
}

// Builder for Via header
/// Builder for Via header
///
/// The Via header provides information about the path taken by the SIP request
/// and the path that should be followed for responses.
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .via("192.168.1.1", "UDP")
///         .with_branch("z9hG4bK776asdhds")
///         .with_rport()
///         .done()
///     .build();
/// ```
pub struct ViaBuilder<P> {
    parent: P,
    protocol: SentProtocol,
    host: String,
    port: Option<u16>,
    params: Vec<Param>,
}

impl<P> ViaBuilder<P> {
    /// Creates a new ViaBuilder with the given host and transport
    ///
    /// # Parameters
    /// - `parent`: The parent builder to return to when done
    /// - `host`: The host or IP address
    /// - `transport`: The transport protocol (UDP, TCP, TLS, etc.)
    ///
    /// # Returns
    /// A new ViaBuilder
    fn new(parent: P, host: &str, transport: &str) -> Self {
        let protocol = SentProtocol {
            name: "SIP".to_string(),
            version: "2.0".to_string(),
            transport: transport.to_uppercase(),
        };
        
        Self {
            parent,
            protocol,
            host: host.to_string(),
            port: None,
            params: Vec::new(),
        }
    }

    /// Adds a branch parameter
    ///
    /// The branch parameter is a unique identifier for the SIP transaction.
    ///
    /// # Parameters
    /// - `branch`: The branch identifier (should start with "z9hG4bK" for RFC 3261 compliance)
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_branch(mut self, branch: &str) -> Self {
        self.params.push(Param::Branch(branch.to_string()));
        self
    }

    /// Adds a received parameter
    ///
    /// The received parameter indicates the source IP address from which the message was received.
    ///
    /// # Parameters
    /// - `ip`: The IP address from which the request was received
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_received(mut self, ip: std::net::IpAddr) -> Self {
        self.params.push(Param::Received(ip));
        self
    }

    /// Adds a rport parameter
    ///
    /// The rport parameter with no value enables response routing back through NAT.
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_rport(mut self) -> Self {
        self.params.push(Param::Rport(None));
        self
    }

    /// Adds a rport parameter with value
    ///
    /// The rport parameter with a value indicates the port from which the request was sent.
    ///
    /// # Parameters
    /// - `port`: The port number
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_rport_value(mut self, port: u16) -> Self {
        self.params.push(Param::Rport(Some(port)));
        self
    }

    /// Adds a ttl parameter
    ///
    /// The ttl parameter indicates the time-to-live value for multicast messages.
    ///
    /// # Parameters
    /// - `ttl`: The time-to-live value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_ttl(mut self, ttl: u8) -> Self {
        self.params.push(Param::Ttl(ttl));
        self
    }

    /// Adds a maddr parameter
    ///
    /// The maddr parameter indicates the multicast address.
    ///
    /// # Parameters
    /// - `maddr`: The multicast address
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_maddr(mut self, maddr: &str) -> Self {
        self.params.push(Param::Maddr(maddr.to_string()));
        self
    }
    
    /// Adds a generic parameter 
    ///
    /// # Parameters
    /// - `name`: The parameter name
    /// - `value`: The optional parameter value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_param(mut self, name: &str, value: Option<&str>) -> Self {
        let param = match value {
            Some(val) => Param::Other(name.to_string(), Some(val.into())),
            None => Param::Other(name.to_string(), None),
        };
        self.params.push(param);
        self
    }
}

// Via builder for Request
impl ViaBuilder<RequestBuilder> {
    /// Finishes building the Via header and returns to the RequestBuilder
    ///
    /// # Returns
    /// The parent RequestBuilder with the Via header added
    pub fn done(self) -> RequestBuilder {
        let sent_by_host = match self.host.parse() {
            Ok(host) => host,
            Err(_) => Host::Domain(self.host.clone()), // Fallback to domain
        };
        
        let via_header = ViaHeader {
            sent_protocol: self.protocol,
            sent_by_host,
            sent_by_port: self.port,
            params: self.params,
        };
        
        let mut parent = self.parent;
        let via = TypedHeader::Via(Via(vec![via_header]));
        parent.request = parent.request.with_header(via);
        parent
    }
}

// Via builder for Response
impl ViaBuilder<ResponseBuilder> {
    /// Finishes building the Via header and returns to the ResponseBuilder
    ///
    /// # Returns
    /// The parent ResponseBuilder with the Via header added
    pub fn done(self) -> ResponseBuilder {
        let sent_by_host = match self.host.parse() {
            Ok(host) => host,
            Err(_) => Host::Domain(self.host.clone()), // Fallback to domain
        };
        
        let via_header = ViaHeader {
            sent_protocol: self.protocol,
            sent_by_host,
            sent_by_port: self.port,
            params: self.params,
        };
        
        let mut parent = self.parent;
        let via = TypedHeader::Via(Via(vec![via_header]));
        parent.response = parent.response.with_header(via);
        parent
    }
}

// Marker traits for From/To header types
/// Marker trait for From header in AddressBuilder
///
/// This is used as a type parameter in the AddressBuilder to indicate
/// that it's building a From header.
pub struct FromHeader;

/// Marker trait for To header in AddressBuilder
///
/// This is used as a type parameter in the AddressBuilder to indicate
/// that it's building a To header.
pub struct ToHeader;

// Builder for address-based headers (From, To)
/// Builder for address headers (From, To)
///
/// This builder creates headers like From and To that contain a SIP address
/// with possible parameters including tags for dialog identification.
///
/// # Example
///
/// ```rust
/// use rvoip_sip_core::prelude::*;
///
/// let request = RequestBuilder::invite("sip:bob@example.com").unwrap()
///     .from("Alice", "sip:alice@example.com")
///         .with_tag("1928301774")
///         .done()
///     .build();
/// ```
pub struct AddressBuilder<P, T> {
    parent: P,
    address: Address,
    _marker: T,
}

impl<P, T> AddressBuilder<P, T> {
    /// Creates a new AddressBuilder with the given display name and URI
    ///
    /// # Parameters
    /// - `parent`: The parent builder to return to when done
    /// - `display_name`: The display name for the address (empty string for none)
    /// - `uri`: The SIP URI as a string
    /// - `_marker`: Type marker to determine the header type (FromHeader or ToHeader)
    ///
    /// # Returns
    /// A new AddressBuilder with the specified address
    fn new(parent: P, display_name: &str, uri: &str, _marker: T) -> Self {
        let uri = match Uri::from_str(uri) {
            Ok(uri) => uri,
            Err(_) => Uri::sip("invalid.example.com"), // Default URI for invalid inputs
        };
        let address = Address::new(Some(display_name), uri);
        
        Self {
            parent,
            address,
            _marker,
        }
    }

    /// Adds a tag parameter to the address
    ///
    /// Tags are used to uniquely identify dialogs. UAs must generate
    /// unique tags for From headers, and copy To tags from requests
    /// when creating responses within a dialog.
    ///
    /// # Parameters
    /// - `tag`: The tag value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.address.set_tag(tag);
        self
    }

    /// Adds a custom parameter to the address
    ///
    /// # Parameters
    /// - `name`: The parameter name
    /// - `value`: The optional parameter value
    ///
    /// # Returns
    /// Self for method chaining
    pub fn with_param(mut self, name: &str, value: Option<&str>) -> Self {
        self.address.set_param(name, value);
        self
    }
}

// AddressBuilder for From header in Request
impl AddressBuilder<RequestBuilder, FromHeader> {
    /// Finishes building the From header and returns to the RequestBuilder
    ///
    /// # Returns
    /// The parent RequestBuilder with the From header added
    pub fn done(self) -> RequestBuilder {
        let mut parent = self.parent;
        let from_header = TypedHeader::From(From(self.address));
        parent.request = parent.request.with_header(from_header);
        parent
    }
}

// AddressBuilder for To header in Request
impl AddressBuilder<RequestBuilder, ToHeader> {
    /// Finishes building the To header and returns to the RequestBuilder
    ///
    /// # Returns
    /// The parent RequestBuilder with the To header added
    pub fn done(self) -> RequestBuilder {
        let mut parent = self.parent;
        let to_header = TypedHeader::To(To(self.address));
        parent.request = parent.request.with_header(to_header);
        parent
    }
}

// AddressBuilder for From header in Response
impl AddressBuilder<ResponseBuilder, FromHeader> {
    /// Finishes building the From header and returns to the ResponseBuilder
    ///
    /// # Returns
    /// The parent ResponseBuilder with the From header added
    pub fn done(self) -> ResponseBuilder {
        let mut parent = self.parent;
        let from_header = TypedHeader::From(From(self.address));
        parent.response = parent.response.with_header(from_header);
        parent
    }
}

// AddressBuilder for To header in Response
impl AddressBuilder<ResponseBuilder, ToHeader> {
    /// Finishes building the To header and returns to the ResponseBuilder
    ///
    /// # Returns
    /// The parent ResponseBuilder with the To header added
    pub fn done(self) -> ResponseBuilder {
        let mut parent = self.parent;
        let to_header = TypedHeader::To(To(self.address));
        parent.response = parent.response.with_header(to_header);
        parent
    }
} 