//! Client-Side SIP Request Builders
//!
//! This module provides high-level, fluent builders for creating SIP requests
//! commonly used by SIP clients. These builders handle all the RFC 3261 
//! requirements automatically and provide sensible defaults.

use std::net::SocketAddr;
use std::str::FromStr;
use uuid::Uuid;
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::types::{
    content_type::ContentType,
    expires::Expires,
    contact::{Contact, ContactParamInfo},
    route::Route,
};
use crate::error::{Error, Result};
use crate::utils::dialog_utils::generate_branch;

/// Builder for INVITE requests
/// 
/// Provides a fluent interface for creating properly formatted INVITE requests
/// with all required headers according to RFC 3261.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::client::builders::InviteBuilder;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let invite = InviteBuilder::new()
///     .from_to("sip:alice@example.com", "sip:bob@example.com") 
///     .local_address(local_addr)
///     .with_sdp("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...")
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct InviteBuilder {
    from_uri: Option<String>,
    from_display_name: Option<String>,
    from_tag: Option<String>,
    to_uri: Option<String>,
    to_display_name: Option<String>,
    to_tag: Option<String>,
    request_uri: Option<String>,
    call_id: Option<String>,
    cseq: u32,
    local_address: Option<SocketAddr>,
    branch: Option<String>,
    route_set: Vec<Uri>,
    contact: Option<String>,
    sdp_content: Option<String>,
    custom_headers: Vec<TypedHeader>,
    max_forwards: u8,
}

impl InviteBuilder {
    /// Create a new INVITE builder with sensible defaults
    pub fn new() -> Self {
        Self {
            from_uri: None,
            from_display_name: None,
            from_tag: None,
            to_uri: None,
            to_display_name: None,
            to_tag: None,
            request_uri: None,
            call_id: None,
            cseq: 1,
            local_address: None,
            branch: None,
            route_set: Vec::new(),
            contact: None,
            sdp_content: None,
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
    }
    
    /// Set From and To URIs with automatic tag generation
    pub fn from_to(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.from_uri = Some(from.into());
        self.to_uri = Some(to.into());
        self.request_uri = self.to_uri.clone(); // Default request URI to To URI
        
        // Auto-generate From tag for new dialogs
        if self.from_tag.is_none() {
            self.from_tag = Some(format!("tag-{}", Uuid::new_v4().simple()));
        }
        
        self
    }
    
    /// Set From URI with display name and optional tag
    pub fn from_detailed(mut self, display_name: Option<&str>, uri: impl Into<String>, tag: Option<&str>) -> Self {
        self.from_uri = Some(uri.into());
        self.from_display_name = display_name.map(|s| s.to_string());
        self.from_tag = tag.map(|s| s.to_string()).or_else(|| {
            Some(format!("tag-{}", Uuid::new_v4().simple()))
        });
        self
    }
    
    /// Set To URI with display name and optional tag
    pub fn to_detailed(mut self, display_name: Option<&str>, uri: impl Into<String>, tag: Option<&str>) -> Self {
        let uri_string = uri.into();
        self.to_uri = Some(uri_string.clone());
        self.to_display_name = display_name.map(|s| s.to_string());
        self.to_tag = tag.map(|s| s.to_string());
        
        // Default request URI to To URI if not already set
        if self.request_uri.is_none() {
            self.request_uri = Some(uri_string);
        }
        
        self
    }
    
    /// Set the request URI (defaults to To URI if not specified)
    pub fn request_uri(mut self, uri: impl Into<String>) -> Self {
        self.request_uri = Some(uri.into());
        self
    }
    
    /// Set the local address for Via header generation
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.local_address = Some(addr);
        self
    }
    
    /// Set a specific Call-ID (auto-generated if not specified)
    pub fn call_id(mut self, call_id: impl Into<String>) -> Self {
        self.call_id = Some(call_id.into());
        self
    }
    
    /// Set the CSeq number (defaults to 1)
    pub fn cseq(mut self, cseq: u32) -> Self {
        self.cseq = cseq;
        self
    }
    
    /// Add SDP content with automatic Content-Type and Content-Length headers
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp_content = Some(sdp.into());
        self
    }
    
    /// Add a route (for proxy routing)
    pub fn add_route(mut self, route: Uri) -> Self {
        self.route_set.push(route);
        self
    }
    
    /// Set Contact header
    pub fn contact(mut self, contact: impl Into<String>) -> Self {
        self.contact = Some(contact.into());
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.custom_headers.push(header);
        self
    }
    
    /// Set Max-Forwards (defaults to 70)
    pub fn max_forwards(mut self, max_forwards: u8) -> Self {
        self.max_forwards = max_forwards;
        self
    }
    
    /// Build the INVITE request
    pub fn build(self) -> Result<Request> {
        // Validate required fields
        let from_uri = self.from_uri.ok_or_else(|| Error::Other("From URI is required".to_string()))?;
        let to_uri = self.to_uri.ok_or_else(|| Error::Other("To URI is required".to_string()))?;
        let request_uri = self.request_uri.unwrap_or_else(|| to_uri.clone());
        let local_addr = self.local_address.ok_or_else(|| Error::Other("Local address is required for Via header".to_string()))?;
        
        // Generate defaults
        let call_id = self.call_id.unwrap_or_else(|| format!("call-{}", Uuid::new_v4()));
        let branch = self.branch.unwrap_or_else(|| generate_branch());
        let from_tag = self.from_tag.unwrap_or_else(|| format!("tag-{}", Uuid::new_v4().simple()));
        
        // Build the request
        let mut builder = SimpleRequestBuilder::new(Method::Invite, &request_uri)
            .map_err(|e| Error::Other(format!("Failed to create request builder: {}", e)))?;
        
        // Add From header
        builder = builder.from(
            self.from_display_name.as_deref().unwrap_or("User"),
            &from_uri,
            Some(&from_tag)
        );
        
        // Add To header
        builder = builder.to(
            self.to_display_name.as_deref().unwrap_or("User"),
            &to_uri,
            self.to_tag.as_deref()
        );
        
        // Add basic headers
        builder = builder
            .call_id(&call_id)
            .cseq(self.cseq)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(self.max_forwards.into());
        
        // Add Contact header if specified
        if let Some(contact) = self.contact {
            builder = builder.contact(&contact, None);
        }
        
        // Add Route headers
        for route in self.route_set {
            builder = builder.header(TypedHeader::Route(Route::with_uri(route)));
        }
        
        // Add SDP content if specified
        if let Some(sdp_content) = &self.sdp_content {
            builder = builder
                .header(TypedHeader::ContentType(ContentType::from_type_subtype("application", "sdp")))
                .header(TypedHeader::ContentLength(ContentLength::new(sdp_content.len() as u32)))
                .body(sdp_content.as_bytes().to_vec());
        } else {
            builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
        }
        
        // Add custom headers
        for header in self.custom_headers {
            builder = builder.header(header);
        }
        
        Ok(builder.build())
    }
}

impl Default for InviteBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for BYE requests
/// 
/// Creates proper BYE requests for terminating established dialogs.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::client::builders::ByeBuilder;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let bye = ByeBuilder::new()
///     .from_dialog("call-123", "sip:alice@example.com", "tag-alice", "sip:bob@example.com", "tag-bob")
///     .local_address(local_addr)
///     .cseq(2)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct ByeBuilder {
    call_id: Option<String>,
    from_uri: Option<String>,
    from_tag: Option<String>,
    to_uri: Option<String>,
    to_tag: Option<String>,
    request_uri: Option<String>,
    cseq: u32,
    local_address: Option<SocketAddr>,
    route_set: Vec<Uri>,
    custom_headers: Vec<TypedHeader>,
    max_forwards: u8,
}

impl ByeBuilder {
    /// Create a new BYE builder
    pub fn new() -> Self {
        Self {
            call_id: None,
            from_uri: None,
            from_tag: None,
            to_uri: None,
            to_tag: None,
            request_uri: None,
            cseq: 1,
            local_address: None,
            route_set: Vec::new(),
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
    }
    
    /// Set dialog information (all required fields for in-dialog request)
    pub fn from_dialog(
        mut self,
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>
    ) -> Self {
        let to_uri_string = to_uri.into();
        self.call_id = Some(call_id.into());
        self.from_uri = Some(from_uri.into());
        self.from_tag = Some(from_tag.into());
        self.to_uri = Some(to_uri_string.clone());
        self.to_tag = Some(to_tag.into());
        self.request_uri = Some(to_uri_string); // Default to To URI
        self
    }
    
    /// Set the request URI (defaults to To URI)
    pub fn request_uri(mut self, uri: impl Into<String>) -> Self {
        self.request_uri = Some(uri.into());
        self
    }
    
    /// Set the local address for Via header
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.local_address = Some(addr);
        self
    }
    
    /// Set the CSeq number
    pub fn cseq(mut self, cseq: u32) -> Self {
        self.cseq = cseq;
        self
    }
    
    /// Add a route
    pub fn add_route(mut self, route: Uri) -> Self {
        self.route_set.push(route);
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.custom_headers.push(header);
        self
    }
    
    /// Build the BYE request
    pub fn build(self) -> Result<Request> {
        // Validate required fields
        let call_id = self.call_id.ok_or_else(|| Error::Other("Call-ID is required".to_string()))?;
        let from_uri = self.from_uri.ok_or_else(|| Error::Other("From URI is required".to_string()))?;
        let from_tag = self.from_tag.ok_or_else(|| Error::Other("From tag is required for in-dialog request".to_string()))?;
        let to_uri = self.to_uri.ok_or_else(|| Error::Other("To URI is required".to_string()))?;
        let to_tag = self.to_tag.ok_or_else(|| Error::Other("To tag is required for in-dialog request".to_string()))?;
        let request_uri = self.request_uri.unwrap_or_else(|| to_uri.clone());
        let local_addr = self.local_address.ok_or_else(|| Error::Other("Local address is required".to_string()))?;
        
        // Generate branch for this request
        let branch = generate_branch();
        
        // Build the request
        let mut builder = SimpleRequestBuilder::new(Method::Bye, &request_uri)
            .map_err(|e| Error::Other(format!("Failed to create request builder: {}", e)))?;
        
        builder = builder
            .from("User", &from_uri, Some(&from_tag))
            .to("User", &to_uri, Some(&to_tag))
            .call_id(&call_id)
            .cseq(self.cseq)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(self.max_forwards.into())
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        // Add Route headers
        for route in self.route_set {
            builder = builder.header(TypedHeader::Route(Route::with_uri(route)));
        }
        
        // Add custom headers
        for header in self.custom_headers {
            builder = builder.header(header);
        }
        
        Ok(builder.build())
    }
}

impl Default for ByeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for REGISTER requests
/// 
/// Creates proper REGISTER requests for SIP registration.
/// 
/// # Example
/// ```
/// use rvoip_transaction_core::client::builders::RegisterBuilder;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let register = RegisterBuilder::new()
///     .registrar("sip:registrar.example.com")
///     .user_info("sip:alice@example.com", "Alice")
///     .local_address(local_addr)
///     .expires(3600)
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct RegisterBuilder {
    registrar_uri: Option<String>,
    from_uri: Option<String>,
    from_display_name: Option<String>,
    contact_uri: Option<String>,
    local_address: Option<SocketAddr>,
    expires: Option<u32>,
    call_id: Option<String>,
    cseq: u32,
    custom_headers: Vec<TypedHeader>,
    max_forwards: u8,
}

impl RegisterBuilder {
    /// Create a new REGISTER builder
    pub fn new() -> Self {
        Self {
            registrar_uri: None,
            from_uri: None,
            from_display_name: None,
            contact_uri: None,
            local_address: None,
            expires: None,
            call_id: None,
            cseq: 1,
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
    }
    
    /// Set the registrar URI (Request-URI and To header)
    pub fn registrar(mut self, uri: impl Into<String>) -> Self {
        self.registrar_uri = Some(uri.into());
        self
    }
    
    /// Set user information (From header)
    pub fn user_info(mut self, uri: impl Into<String>, display_name: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
        self.from_display_name = Some(display_name.into());
        self
    }
    
    /// Set Contact URI (defaults to local address if not specified)
    pub fn contact(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }
    
    /// Set the local address
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.local_address = Some(addr);
        self
    }
    
    /// Set registration expiration time in seconds
    pub fn expires(mut self, seconds: u32) -> Self {
        self.expires = Some(seconds);
        self
    }
    
    /// Set Call-ID (auto-generated if not specified)
    pub fn call_id(mut self, call_id: impl Into<String>) -> Self {
        self.call_id = Some(call_id.into());
        self
    }
    
    /// Set CSeq number
    pub fn cseq(mut self, cseq: u32) -> Self {
        self.cseq = cseq;
        self
    }
    
    /// Add a custom header
    pub fn header(mut self, header: TypedHeader) -> Self {
        self.custom_headers.push(header);
        self
    }
    
    /// Build the REGISTER request
    pub fn build(self) -> Result<Request> {
        // Validate required fields
        let registrar_uri = self.registrar_uri.ok_or_else(|| Error::Other("Registrar URI is required".to_string()))?;
        let from_uri = self.from_uri.ok_or_else(|| Error::Other("From URI is required".to_string()))?;
        let local_addr = self.local_address.ok_or_else(|| Error::Other("Local address is required".to_string()))?;
        
        // Generate defaults
        let call_id = self.call_id.unwrap_or_else(|| format!("reg-{}", Uuid::new_v4()));
        let from_tag = format!("tag-{}", Uuid::new_v4().simple());
        let branch = generate_branch();
        let contact_uri = self.contact_uri.unwrap_or_else(|| format!("sip:user@{}", local_addr));
        
        // Build the request
        let mut builder = SimpleRequestBuilder::new(Method::Register, &registrar_uri)
            .map_err(|e| Error::Other(format!("Failed to create request builder: {}", e)))?;
        
        builder = builder
            .from(
                self.from_display_name.as_deref().unwrap_or("User"),
                &from_uri,
                Some(&from_tag)
            )
            .to("", &registrar_uri, None) // To header same as registrar, no tag
            .call_id(&call_id)
            .cseq(self.cseq)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(self.max_forwards.into())
            .contact(&contact_uri, None)
            .header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        // Add Expires header if specified
        if let Some(expires) = self.expires {
            builder = builder.header(TypedHeader::Expires(Expires::new(expires)));
        }
        
        // Add custom headers
        for header in self.custom_headers {
            builder = builder.header(header);
        }
        
        Ok(builder.build())
    }
}

impl Default for RegisterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience functions for common client requests
pub mod quick {
    use super::*;
    
    /// Create a simple INVITE request
    pub fn invite(
        from: &str,
        to: &str,
        local_addr: SocketAddr,
        sdp: Option<&str>
    ) -> Result<Request> {
        let mut builder = InviteBuilder::new()
            .from_to(from, to)
            .local_address(local_addr);
        
        if let Some(sdp_content) = sdp {
            builder = builder.with_sdp(sdp_content);
        }
        
        builder.build()
    }
    
    /// Create a BYE request for an established dialog
    pub fn bye(
        call_id: &str,
        from_uri: &str,
        from_tag: &str,
        to_uri: &str,
        to_tag: &str,
        local_addr: SocketAddr,
        cseq: u32
    ) -> Result<Request> {
        ByeBuilder::new()
            .from_dialog(call_id, from_uri, from_tag, to_uri, to_tag)
            .local_address(local_addr)
            .cseq(cseq)
            .build()
    }
    
    /// Create a REGISTER request
    pub fn register(
        registrar: &str,
        user_uri: &str,
        display_name: &str,
        local_addr: SocketAddr,
        expires: Option<u32>
    ) -> Result<Request> {
        let mut builder = RegisterBuilder::new()
            .registrar(registrar)
            .user_info(user_uri, display_name)
            .local_address(local_addr);
        
        if let Some(exp) = expires {
            builder = builder.expires(exp);
        }
        
        builder.build()
    }
    
    /// Create an OPTIONS request
    pub fn options(
        target_uri: &str,
        from_uri: &str,
        local_addr: SocketAddr
    ) -> Result<Request> {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::header::TypedHeader;
        use rvoip_sip_core::types::max_forwards::MaxForwards;
        use rvoip_sip_core::types::content_length::ContentLength;
        
        let request = SimpleRequestBuilder::new(Method::Options, target_uri)?
            .from("User", from_uri, Some(&format!("tag-{}", Uuid::new_v4().simple())))
            .to("User", target_uri, None)
            .call_id(&format!("options-{}", Uuid::new_v4()))
            .cseq(1)
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
        
        Ok(request)
    }
    
    /// Create a MESSAGE request for instant messaging
    pub fn message(
        target_uri: &str,
        from_uri: &str,
        local_addr: SocketAddr,
        content: &str
    ) -> Result<Request> {
        use rvoip_sip_core::builder::SimpleRequestBuilder;
        use rvoip_sip_core::types::header::TypedHeader;
        use rvoip_sip_core::types::max_forwards::MaxForwards;
        use rvoip_sip_core::types::content_length::ContentLength;
        use rvoip_sip_core::types::content_type::ContentType;
        use uuid::Uuid;
        use std::str::FromStr;
        
        let request = SimpleRequestBuilder::new(Method::Message, target_uri)?
            .from("User", from_uri, Some(&format!("tag-{}", Uuid::new_v4().simple())))
            .to("User", target_uri, None)
            .call_id(&format!("message-{}", Uuid::new_v4()))
            .cseq(1)
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentType(ContentType::from_str("text/plain").unwrap()))
            .header(TypedHeader::ContentLength(ContentLength::new(content.len() as u32)))
            .body(content.as_bytes().to_vec())
            .build();
        
        Ok(request)
    }
} 