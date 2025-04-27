use std::str::FromStr;
use bytes::Bytes;

use crate::error::{Error, Result};
use crate::types::{
    Method, 
    StatusCode, 
    Version,
    sip_message::{Request, Response},
    uri::{Uri, Host},
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

/// Builder for SIP request messages
pub struct RequestBuilder {
    request: Request,
}

impl RequestBuilder {
    /// Create a new RequestBuilder with the specified method and URI
    pub fn new(method: Method, uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let mut request = Request::new(method, uri);
        request.version = Version::new(2, 0); // Default to SIP/2.0
        Ok(Self { request })
    }
    
    /// Create a RequestBuilder from an existing Request
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

    /// Create an OPTIONS request
    pub fn options(uri: &str) -> Result<Self> {
        Self::new(Method::Options, uri)
    }

    /// Create a BYE request
    pub fn bye(uri: &str) -> Result<Self> {
        Self::new(Method::Bye, uri)
    }

    /// Create an ACK request
    pub fn ack(uri: &str) -> Result<Self> {
        Self::new(Method::Ack, uri)
    }

    /// Create a CANCEL request
    pub fn cancel(uri: &str) -> Result<Self> {
        Self::new(Method::Cancel, uri)
    }

    /// Set the SIP version (default is 2.0)
    pub fn version(mut self, major: u8, minor: u8) -> Self {
        self.request.version = Version::new(major, minor);
        self
    }

    /// Add a Via header
    pub fn via(self, host: &str, transport: &str) -> ViaBuilder<Self> {
        ViaBuilder::new(self, host, transport)
    }

    /// Add a From header
    pub fn from(self, display_name: &str, uri: &str) -> AddressBuilder<Self, FromHeader> {
        AddressBuilder::new(self, display_name, uri, FromHeader)
    }

    /// Add a To header
    pub fn to(self, display_name: &str, uri: &str) -> AddressBuilder<Self, ToHeader> {
        AddressBuilder::new(self, display_name, uri, ToHeader)
    }

    /// Add a simple To header without parameters
    pub fn simple_to(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let to_addr = Address::new(Some(display_name), uri);
        self.request = self.request.with_header(TypedHeader::To(To(to_addr)));
        Ok(self)
    }

    /// Add a simple From header without parameters
    pub fn simple_from(mut self, display_name: &str, uri: &str) -> Result<Self> {
        let uri = Uri::from_str(uri)?;
        let from_addr = Address::new(Some(display_name), uri);
        self.request = self.request.with_header(TypedHeader::From(From(from_addr)));
        Ok(self)
    }

    /// Add a Call-ID header
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.request = self.request.with_header(TypedHeader::CallId(
            CallId(call_id.to_string())
        ));
        self
    }

    /// Add a CSeq header
    pub fn cseq(mut self, seq: u32) -> Self {
        let method = self.request.method.clone();
        self.request = self.request.with_header(TypedHeader::CSeq(
            CSeq::new(seq, method)
        ));
        self
    }

    /// Add a Max-Forwards header
    pub fn max_forwards(mut self, value: u32) -> Self {
        self.request = self.request.with_header(TypedHeader::MaxForwards(
            MaxForwards(value as u8)
        ));
        self
    }

    /// Add a Contact header
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
    pub fn content_type(mut self, content_type: &str) -> Result<Self> {
        self.request = self.request.with_header(TypedHeader::ContentType(
            ContentType::from_str(content_type)?
        ));
        Ok(self)
    }

    /// Add body content
    pub fn body(mut self, body: &str) -> Self {
        let content_length = body.len() as u32;
        self.request = self.request.with_header(TypedHeader::ContentLength(
            ContentLength(content_length)
        ));
        self.request.body = Bytes::from(body.to_string());
        self
    }

    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.request = self.request.with_header(header);
        self
    }

    /// Build the final SIP request
    pub fn build(self) -> Request {
        self.request
    }

    /// Internal method to update the request - used by sub-builders
    fn update_request(&mut self, request: Request) {
        self.request = request;
    }
}

/// Builder for SIP response messages
pub struct ResponseBuilder {
    response: Response,
}

impl ResponseBuilder {
    /// Create a new ResponseBuilder with the specified status code
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
    pub fn ok() -> Self {
        Self::new(StatusCode::Ok)
    }

    /// Create a 100 Trying response
    pub fn trying() -> Self {
        Self::new(StatusCode::Trying)
    }

    /// Create a 180 Ringing response
    pub fn ringing() -> Self {
        Self::new(StatusCode::Ringing)
    }

    /// Create a 400 Bad Request response
    pub fn bad_request() -> Self {
        Self::new(StatusCode::BadRequest)
    }

    /// Create a 404 Not Found response
    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound)
    }

    /// Create a 500 Internal Server Error response
    pub fn server_error() -> Self {
        Self::new(StatusCode::ServerInternalError)
    }
    
    /// Set the SIP version (default is 2.0)
    pub fn version(mut self, major: u8, minor: u8) -> Self {
        self.response.version = Version::new(major, minor);
        self
    }

    /// Set the response reason phrase
    pub fn reason(mut self, reason: &str) -> Self {
        self.response.reason = Some(reason.to_string());
        self
    }

    /// Add a From header
    pub fn from(self, display_name: &str, uri: &str) -> AddressBuilder<Self, FromHeader> {
        AddressBuilder::new(self, display_name, uri, FromHeader)
    }

    /// Add a To header
    pub fn to(self, display_name: &str, uri: &str) -> AddressBuilder<Self, ToHeader> {
        AddressBuilder::new(self, display_name, uri, ToHeader)
    }

    /// Add a Via header
    pub fn via(self, host: &str, transport: &str) -> ViaBuilder<Self> {
        ViaBuilder::new(self, host, transport)
    }

    /// Add a Call-ID header
    pub fn call_id(mut self, call_id: &str) -> Self {
        self.response = self.response.with_header(TypedHeader::CallId(
            CallId(call_id.to_string())
        ));
        self
    }

    /// Add a CSeq header
    pub fn cseq(mut self, seq: u32, method: Method) -> Self {
        self.response = self.response.with_header(TypedHeader::CSeq(
            CSeq::new(seq, method)
        ));
        self
    }

    /// Add a Contact header
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
    pub fn content_type(mut self, content_type: &str) -> Result<Self> {
        self.response = self.response.with_header(TypedHeader::ContentType(
            ContentType::from_str(content_type)?
        ));
        Ok(self)
    }

    /// Add body content
    pub fn body(mut self, body: &str) -> Self {
        let content_length = body.len() as u32;
        self.response = self.response.with_header(TypedHeader::ContentLength(
            ContentLength(content_length)
        ));
        self.response.body = Bytes::from(body.to_string());
        self
    }

    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.response = self.response.with_header(header);
        self
    }

    /// Build the final SIP response
    pub fn build(self) -> Response {
        self.response
    }

    /// Internal method to update the response - used by sub-builders
    fn update_response(&mut self, response: Response) {
        self.response = response;
    }
}

// Builder for Via header
pub struct ViaBuilder<P> {
    parent: P,
    protocol: SentProtocol,
    host: String,
    port: Option<u16>,
    params: Vec<Param>,
}

impl<P> ViaBuilder<P> {
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

    /// Add a branch parameter
    pub fn with_branch(mut self, branch: &str) -> Self {
        self.params.push(Param::Branch(branch.to_string()));
        self
    }

    /// Add a received parameter
    pub fn with_received(mut self, ip: std::net::IpAddr) -> Self {
        self.params.push(Param::Received(ip));
        self
    }

    /// Add a rport parameter
    pub fn with_rport(mut self) -> Self {
        self.params.push(Param::Rport(None));
        self
    }

    /// Add a rport parameter with value
    pub fn with_rport_value(mut self, port: u16) -> Self {
        self.params.push(Param::Rport(Some(port)));
        self
    }

    /// Add a ttl parameter
    pub fn with_ttl(mut self, ttl: u8) -> Self {
        self.params.push(Param::Ttl(ttl));
        self
    }

    /// Add a maddr parameter
    pub fn with_maddr(mut self, maddr: &str) -> Self {
        self.params.push(Param::Maddr(maddr.to_string()));
        self
    }
    
    /// Add a generic parameter 
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
    /// Finish building the Via header and return to the RequestBuilder
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
    /// Finish building the Via header and return to the ResponseBuilder
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
pub struct FromHeader;
pub struct ToHeader;

// Builder for address-based headers (From, To)
pub struct AddressBuilder<P, T> {
    parent: P,
    address: Address,
    _marker: T,
}

impl<P, T> AddressBuilder<P, T> {
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

    /// Add a tag parameter
    pub fn with_tag(mut self, tag: &str) -> Self {
        self.address.set_tag(tag);
        self
    }

    /// Add a custom parameter
    pub fn with_param(mut self, name: &str, value: Option<&str>) -> Self {
        self.address.set_param(name, value);
        self
    }
}

// AddressBuilder for From header in Request
impl AddressBuilder<RequestBuilder, FromHeader> {
    /// Finish building the From header and return to the RequestBuilder
    pub fn done(self) -> RequestBuilder {
        let mut parent = self.parent;
        let from_header = TypedHeader::From(From(self.address));
        parent.request = parent.request.with_header(from_header);
        parent
    }
}

// AddressBuilder for To header in Request
impl AddressBuilder<RequestBuilder, ToHeader> {
    /// Finish building the To header and return to the RequestBuilder
    pub fn done(self) -> RequestBuilder {
        let mut parent = self.parent;
        let to_header = TypedHeader::To(To(self.address));
        parent.request = parent.request.with_header(to_header);
        parent
    }
}

// AddressBuilder for From header in Response
impl AddressBuilder<ResponseBuilder, FromHeader> {
    /// Finish building the From header and return to the ResponseBuilder
    pub fn done(self) -> ResponseBuilder {
        let mut parent = self.parent;
        let from_header = TypedHeader::From(From(self.address));
        parent.response = parent.response.with_header(from_header);
        parent
    }
}

// AddressBuilder for To header in Response
impl AddressBuilder<ResponseBuilder, ToHeader> {
    /// Finish building the To header and return to the ResponseBuilder
    pub fn done(self) -> ResponseBuilder {
        let mut parent = self.parent;
        let to_header = TypedHeader::To(To(self.address));
        parent.response = parent.response.with_header(to_header);
        parent
    }
} 