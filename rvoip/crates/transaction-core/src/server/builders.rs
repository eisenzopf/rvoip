//! Server-Side SIP Response Builders
//!
//! This module provides high-level, fluent builders for creating SIP responses
//! commonly used by SIP servers. These builders handle all the RFC 3261 
//! requirements automatically and provide sensible defaults.

use std::net::SocketAddr;
use std::str::FromStr;
use uuid::Uuid;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::{
    content_type::ContentType,
    expires::Expires,
    contact::{Contact, ContactParamInfo},
    sip_response::Response,
    allow::Allow,
    via::Via,
};
use crate::error::{Error, Result};

/// Builder for SIP responses
/// 
/// Provides a fluent interface for creating properly formatted SIP responses
/// with all required headers according to RFC 3261.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::server::builders::ResponseBuilder;
/// use rvoip_transaction_core::builders::client_quick;
/// use rvoip_sip_core::StatusCode;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let request = client_quick::invite(
///     "sip:alice@example.com", 
///     "sip:bob@example.com", 
///     local_addr, 
///     None
/// ).unwrap();
/// 
/// let response = ResponseBuilder::new(StatusCode::Ok)
///     .from_request(&request)
///     .with_to_tag("server-tag")
///     .with_contact("sip:server@example.com")
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct ResponseBuilder {
    status_code: StatusCode,
    reason_phrase: Option<String>,
    // Headers from original request
    via_headers: Vec<Via>,
    from_header: Option<From>,
    to_header: Option<To>,
    call_id: Option<String>,
    cseq: Option<CSeq>,
    // Additional headers for response
    to_tag: Option<String>,
    contact: Option<String>,
    content_type: Option<String>,
    expires: Option<u32>,
    sdp_content: Option<String>,
    custom_headers: Vec<TypedHeader>,
}

impl ResponseBuilder {
    /// Create a new response builder with the given status code
    pub fn new(status_code: StatusCode) -> Self {
        Self {
            status_code,
            reason_phrase: None,
            via_headers: Vec::new(),
            from_header: None,
            to_header: None,
            call_id: None,
            cseq: None,
            to_tag: None,
            contact: None,
            content_type: None,
            expires: None,
            sdp_content: None,
            custom_headers: Vec::new(),
        }
    }
    
    /// Initialize from a SIP request (copies required headers)
    pub fn from_request(mut self, request: &Request) -> Self {
        // Copy Via headers
        for header in &request.headers {
            if let TypedHeader::Via(via) = header {
                self.via_headers.push(via.clone());
            }
        }
        
        // Copy From header
        if let Some(TypedHeader::From(from)) = request.header(&HeaderName::From) {
            self.from_header = Some(from.clone());
        }
        
        // Copy To header (we'll add tag later if needed)
        if let Some(TypedHeader::To(to)) = request.header(&HeaderName::To) {
            self.to_header = Some(to.clone());
        }
        
        // Copy Call-ID
        if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
            self.call_id = Some(call_id.value().to_string());
        }
        
        // Copy CSeq
        if let Some(TypedHeader::CSeq(cseq)) = request.header(&HeaderName::CSeq) {
            self.cseq = Some(cseq.clone());
        }
        
        self
    }
    
    /// Set a custom reason phrase (optional)
    pub fn reason_phrase(mut self, phrase: impl Into<String>) -> Self {
        self.reason_phrase = Some(phrase.into());
        self
    }
    
    /// Add a To tag (required for 2xx responses to dialog-creating requests)
    pub fn with_to_tag(mut self, tag: impl Into<String>) -> Self {
        self.to_tag = Some(tag.into());
        self
    }
    
    /// Add a generated To tag
    pub fn with_generated_to_tag(mut self) -> Self {
        self.to_tag = Some(format!("tag-{}", Uuid::new_v4().simple()));
        self
    }
    
    /// Add Contact header
    pub fn with_contact(mut self, contact: impl Into<String>) -> Self {
        self.contact = Some(contact.into());
        self
    }
    
    /// Add Contact header with local address
    pub fn with_contact_address(mut self, local_addr: SocketAddr, user: Option<&str>) -> Self {
        let user_part = user.unwrap_or("server");
        self.contact = Some(format!("sip:{}@{}", user_part, local_addr));
        self
    }
    
    /// Add SDP content
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp_content = Some(sdp.into());
        self.content_type = Some("application/sdp".to_string());
        self
    }
    
    /// Add Expires header (for REGISTER responses)
    pub fn with_expires(mut self, seconds: u32) -> Self {
        self.expires = Some(seconds);
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.custom_headers.push(header);
        self
    }
    
    /// Build the response
    pub fn build(self) -> Result<Response> {
        // Build the response with just the status code
        let mut response = Response::new(self.status_code);
        
        // Set custom reason phrase if provided
        if let Some(reason) = self.reason_phrase {
            response = response.with_reason(reason);
        }
        
        // Add Via headers (must be in same order as request)
        for via_header in self.via_headers {
            response.headers.push(TypedHeader::Via(via_header));
        }
        
        // Add From header
        if let Some(from) = self.from_header {
            response.headers.push(TypedHeader::From(from));
        }
        
        // Add To header (with tag if specified)
        if let Some(mut to) = self.to_header {
            if let Some(tag) = self.to_tag {
                to = to.with_tag(&tag);
            }
            response.headers.push(TypedHeader::To(to));
        }
        
        // Add Call-ID
        if let Some(call_id) = self.call_id {
            response.headers.push(TypedHeader::CallId(CallId::new(&call_id)));
        }
        
        // Add CSeq
        if let Some(cseq) = self.cseq {
            response.headers.push(TypedHeader::CSeq(cseq));
        }
        
        // Add Contact header if specified
        if let Some(contact) = self.contact {
            let contact_uri: Uri = contact.parse().map_err(|e| Error::Other(format!("Invalid contact URI: {}", e)))?;
            let contact_info = ContactParamInfo {
                address: Address::new(contact_uri)
            };
            response.headers.push(TypedHeader::Contact(Contact::new_params(vec![contact_info])));
        }
        
        // Add Expires header if specified
        if let Some(expires) = self.expires {
            response.headers.push(TypedHeader::Expires(Expires::new(expires)));
        }
        
        // Add content if specified
        if let Some(sdp_content) = &self.sdp_content {
            if let Some(_content_type) = &self.content_type {
                response.headers.push(TypedHeader::ContentType(
                    ContentType::from_type_subtype("application", "sdp")
                ));
            }
            response.headers.push(TypedHeader::ContentLength(ContentLength::new(sdp_content.len() as u32)));
            response.body = sdp_content.as_bytes().to_vec().into();
        } else {
            response.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
        }
        
        // Add custom headers
        for header in self.custom_headers {
            response.headers.push(header);
        }
        
        Ok(response)
    }
}

/// Builder for INVITE responses
/// 
/// Specialized builder for INVITE responses with SDP negotiation support.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::server::builders::InviteResponseBuilder;
/// use rvoip_transaction_core::builders::client_quick;
/// use rvoip_sip_core::StatusCode;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let request = client_quick::invite(
///     "sip:alice@example.com", 
///     "sip:bob@example.com", 
///     local_addr, 
///     None
/// ).unwrap();
/// 
/// let response = InviteResponseBuilder::new(StatusCode::Ok)
///     .from_request(&request)
///     .with_sdp_answer("v=0\r\no=server 456 789 IN IP4 127.0.0.1\r\n...")
///     .with_contact_address(local_addr, Some("server"))
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct InviteResponseBuilder {
    inner: ResponseBuilder,
    sdp_answer: Option<String>,
    early_media: bool,
}

impl InviteResponseBuilder {
    /// Create a new INVITE response builder
    pub fn new(status_code: StatusCode) -> Self {
        Self {
            inner: ResponseBuilder::new(status_code),
            sdp_answer: None,
            early_media: false,
        }
    }
    
    /// Initialize from INVITE request
    pub fn from_request(mut self, request: &Request) -> Self {
        self.inner = self.inner.from_request(request);
        self
    }
    
    /// Add SDP answer for 200 OK responses
    pub fn with_sdp_answer(mut self, sdp: impl Into<String>) -> Self {
        self.sdp_answer = Some(sdp.into());
        self
    }
    
    /// Enable early media for 18x responses
    pub fn with_early_media(mut self, sdp: impl Into<String>) -> Self {
        self.sdp_answer = Some(sdp.into());
        self.early_media = true;
        self
    }
    
    /// Add Contact header
    pub fn with_contact(mut self, contact: impl Into<String>) -> Self {
        self.inner = self.inner.with_contact(contact);
        self
    }
    
    /// Add Contact header with local address
    pub fn with_contact_address(mut self, local_addr: SocketAddr, user: Option<&str>) -> Self {
        self.inner = self.inner.with_contact_address(local_addr, user);
        self
    }
    
    /// Add To tag (auto-generated if not specified for 18x/2xx responses)
    pub fn with_to_tag(mut self, tag: impl Into<String>) -> Self {
        self.inner = self.inner.with_to_tag(tag);
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.inner = self.inner.header(header);
        self
    }
    
    /// Build the INVITE response
    pub fn build(mut self) -> Result<Response> {
        // Auto-generate To tag for 18x and 2xx responses if not specified
        if self.inner.status_code.as_u16() >= 180 && self.inner.to_tag.is_none() {
            self.inner = self.inner.with_generated_to_tag();
        }
        
        // Add SDP content if specified
        if let Some(sdp) = self.sdp_answer {
            self.inner = self.inner.with_sdp(sdp);
        }
        
        self.inner.build()
    }
}

/// Builder for REGISTER responses
/// 
/// Specialized builder for REGISTER responses with contact and expires handling.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::server::builders::RegisterResponseBuilder;
/// use rvoip_transaction_core::builders::client_quick;
/// use rvoip_sip_core::StatusCode;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let request = client_quick::register(
///     "sip:registrar.example.com",
///     "sip:alice@example.com",
///     "Alice",
///     local_addr,
///     Some(3600)
/// ).unwrap();
/// 
/// let response = RegisterResponseBuilder::new(StatusCode::Ok)
///     .from_request(&request)
///     .with_expires(3600)
///     .with_registered_contacts(vec!["sip:alice@192.168.1.100".to_string()])
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct RegisterResponseBuilder {
    inner: ResponseBuilder,
    registered_contacts: Vec<String>,
}

impl RegisterResponseBuilder {
    /// Create a new REGISTER response builder
    pub fn new(status_code: StatusCode) -> Self {
        Self {
            inner: ResponseBuilder::new(status_code),
            registered_contacts: Vec::new(),
        }
    }
    
    /// Initialize from REGISTER request
    pub fn from_request(mut self, request: &Request) -> Self {
        self.inner = self.inner.from_request(request);
        self
    }
    
    /// Set expires value
    pub fn with_expires(mut self, seconds: u32) -> Self {
        self.inner = self.inner.with_expires(seconds);
        self
    }
    
    /// Add registered contacts (for 200 OK responses)
    pub fn with_registered_contacts(mut self, contacts: Vec<String>) -> Self {
        self.registered_contacts = contacts;
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.inner = self.inner.header(header);
        self
    }
    
    /// Build the REGISTER response
    pub fn build(mut self) -> Result<Response> {
        // Add Contact headers for registered contacts (200 OK responses)
        if self.inner.status_code == StatusCode::Ok && !self.registered_contacts.is_empty() {
            for contact in self.registered_contacts {
                let contact_uri: Uri = contact.parse().map_err(|e| Error::Other(format!("Invalid contact URI: {}", e)))?;
                let contact_info = ContactParamInfo {
                    address: Address::new(contact_uri)
                };
                let contact_header = TypedHeader::Contact(Contact::new_params(vec![contact_info]));
                self.inner = self.inner.header(contact_header);
            }
        }
        
        self.inner.build()
    }
}

/// Convenience functions for common server responses
pub mod quick {
    use super::*;
    
    /// Create a 100 Trying response
    pub fn trying(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::Trying)
            .from_request(request)
            .build()
    }
    
    /// Create a 180 Ringing response
    pub fn ringing(request: &Request, contact: Option<String>) -> Result<Response> {
        let mut builder = InviteResponseBuilder::new(StatusCode::Ringing)
            .from_request(request);
        
        if let Some(contact_uri) = contact {
            builder = builder.with_contact(contact_uri);
        }
        
        builder.build()
    }
    
    /// Create a 200 OK response for INVITE with SDP
    pub fn ok_invite(request: &Request, sdp_answer: Option<String>, contact: String) -> Result<Response> {
        let mut builder = InviteResponseBuilder::new(StatusCode::Ok)
            .from_request(request)
            .with_contact(contact);
        
        if let Some(sdp) = sdp_answer {
            builder = builder.with_sdp_answer(sdp);
        }
        
        builder.build()
    }
    
    /// Create a 200 OK response for BYE
    pub fn ok_bye(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::Ok)
            .from_request(request)
            .build()
    }
    
    /// Create a 200 OK response for REGISTER
    pub fn ok_register(request: &Request, expires: u32, contacts: Vec<String>) -> Result<Response> {
        RegisterResponseBuilder::new(StatusCode::Ok)
            .from_request(request)
            .with_expires(expires)
            .with_registered_contacts(contacts)
            .build()
    }
    
    /// Create a 200 OK response for OPTIONS
    pub fn ok_options(request: &Request, allow_methods: Vec<Method>) -> Result<Response> {
        let mut allow_header = Allow::new();
        for method in allow_methods {
            allow_header.add_method(method);
        }
        
        ResponseBuilder::new(StatusCode::Ok)
            .from_request(request)
            .header(TypedHeader::Allow(allow_header))
            .build()
    }
    
    /// Create a 200 OK response for MESSAGE
    pub fn ok_message(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::Ok)
            .from_request(request)
            .build()
    }
    
    /// Create a 486 Busy Here response
    pub fn busy_here(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::BusyHere)
            .from_request(request)
            .build()
    }
    
    /// Create a 487 Request Terminated response
    pub fn request_terminated(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::RequestTerminated)
            .from_request(request)
            .build()
    }
    
    /// Create a 404 Not Found response
    pub fn not_found(request: &Request) -> Result<Response> {
        ResponseBuilder::new(StatusCode::NotFound)
            .from_request(request)
            .build()
    }
    
    /// Create a 500 Server Internal Error response
    pub fn server_error(request: &Request, reason: Option<String>) -> Result<Response> {
        let mut builder = ResponseBuilder::new(StatusCode::ServerInternalError)
            .from_request(request);
        
        if let Some(reason_phrase) = reason {
            builder = builder.reason_phrase(reason_phrase);
        }
        
        builder.build()
    }
} 