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
    event::{Event, EventType},
};
use crate::transaction::error::{Error, Result};
use crate::transaction::utils::dialog_utils::generate_branch;

/// Builder for INVITE requests
/// 
/// Provides a fluent interface for creating properly formatted INVITE requests
/// with all required headers according to RFC 3261.
/// 
/// # Example
/// ```
/// use rvoip_dialog_core::transaction::client::builders::InviteBuilder;
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
    
    /// Create INVITE from existing dialog context
    /// 
    /// This method provides dialog-aware INVITE building by automatically
    /// extracting and using dialog context to populate the builder fields.
    /// This is particularly useful for re-INVITE scenarios or when creating
    /// INVITEs within established dialogs.
    /// 
    /// # Arguments
    /// * `call_id` - The dialog's Call-ID
    /// * `from_uri` - Local URI (From header)
    /// * `from_tag` - Local tag (From header tag)
    /// * `to_uri` - Remote URI (To header)
    /// * `to_tag` - Remote tag (To header tag)
    /// * `cseq` - Next CSeq number for this dialog
    /// * `local_address` - Local address for Via header
    /// 
    /// # Returns
    /// An InviteBuilder pre-configured with dialog context
    /// 
    /// # Example
    /// ```rust,no_run
    /// use rvoip_dialog_core::transaction::client::builders::InviteBuilder;
    /// use std::net::SocketAddr;
    /// 
    /// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    /// let reinvite = InviteBuilder::from_dialog(
    ///     "call-123",
    ///     "sip:alice@example.com",
    ///     "tag-alice",
    ///     "sip:bob@example.com", 
    ///     "tag-bob",
    ///     2, // Next CSeq
    ///     local_addr
    /// )
    /// .with_sdp("v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n...")
    /// .build()
    /// .unwrap();
    /// ```
    pub fn from_dialog(
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>,
        cseq: u32,
        local_address: SocketAddr
    ) -> Self {
        let to_uri_string = to_uri.into();
        Self {
            from_uri: Some(from_uri.into()),
            from_display_name: None, // Will use default "User" 
            from_tag: Some(from_tag.into()),
            to_uri: Some(to_uri_string.clone()),
            to_display_name: None, // Will use default "User"
            to_tag: Some(to_tag.into()),
            request_uri: Some(to_uri_string), // Default to To URI for re-INVITE
            call_id: Some(call_id.into()),
            cseq,
            local_address: Some(local_address),
            branch: None, // Will be auto-generated
            route_set: Vec::new(), // Can be added with add_route()
            contact: None, // Can be set with contact()
            sdp_content: None, // Can be set with with_sdp()
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
    }
    
    /// Create INVITE from dialog context with enhanced options
    /// 
    /// This method provides even more control over dialog-aware INVITE creation
    /// by allowing specification of route set and contact information from
    /// the dialog context.
    /// 
    /// # Arguments
    /// * `call_id` - The dialog's Call-ID
    /// * `from_uri` - Local URI (From header)
    /// * `from_tag` - Local tag (From header tag)  
    /// * `from_display_name` - Optional display name for From header
    /// * `to_uri` - Remote URI (To header)
    /// * `to_tag` - Remote tag (To header tag)
    /// * `to_display_name` - Optional display name for To header
    /// * `request_uri` - Request URI (may differ from To URI due to routing)
    /// * `cseq` - Next CSeq number for this dialog
    /// * `local_address` - Local address for Via header
    /// * `route_set` - Route set from dialog for proxy routing
    /// * `contact` - Contact header from dialog
    /// 
    /// # Returns
    /// An InviteBuilder pre-configured with full dialog context
    pub fn from_dialog_enhanced(
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        from_display_name: Option<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>,
        to_display_name: Option<String>,
        request_uri: impl Into<String>,
        cseq: u32,
        local_address: SocketAddr,
        route_set: Vec<Uri>,
        contact: Option<String>
    ) -> Self {
        Self {
            from_uri: Some(from_uri.into()),
            from_display_name,
            from_tag: Some(from_tag.into()),
            to_uri: Some(to_uri.into()),
            to_display_name,
            to_tag: Some(to_tag.into()),
            request_uri: Some(request_uri.into()),
            call_id: Some(call_id.into()),
            cseq,
            local_address: Some(local_address),
            branch: None, // Will be auto-generated
            route_set,
            contact,
            sdp_content: None, // Can be set with with_sdp()
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
/// use rvoip_dialog_core::transaction::client::builders::ByeBuilder;
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
    
    /// Create BYE from dialog context with enhanced options
    /// 
    /// This method provides enhanced dialog-aware BYE building with automatic
    /// route set and contact handling from the dialog context. This ensures
    /// proper routing and compliance with RFC 3261 dialog requirements.
    /// 
    /// # Arguments
    /// * `call_id` - The dialog's Call-ID
    /// * `from_uri` - Local URI (From header)
    /// * `from_tag` - Local tag (From header tag)
    /// * `to_uri` - Remote URI (To header)
    /// * `to_tag` - Remote tag (To header tag)
    /// * `request_uri` - Request URI (may differ from To URI due to routing)
    /// * `cseq` - Next CSeq number for this dialog
    /// * `local_address` - Local address for Via header
    /// * `route_set` - Route set from dialog for proxy routing
    /// 
    /// # Returns
    /// A ByeBuilder pre-configured with full dialog context
    /// 
    /// # Example
    /// ```rust,no_run
    /// use rvoip_dialog_core::transaction::client::builders::ByeBuilder;
    /// use rvoip_sip_core::Uri;
    /// use std::net::SocketAddr;
    /// 
    /// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
    /// let route1: Uri = "sip:proxy1.example.com".parse().unwrap();
    /// let route2: Uri = "sip:proxy2.example.com".parse().unwrap();
    /// let bye = ByeBuilder::from_dialog_enhanced(
    ///     "call-123",
    ///     "sip:alice@example.com",
    ///     "tag-alice", 
    ///     "sip:bob@example.com",
    ///     "tag-bob",
    ///     "sip:proxy.example.com", // Request-URI from route set
    ///     3, // Next CSeq
    ///     local_addr,
    ///     vec![route1, route2] // Route set from dialog
    /// )
    /// .build()
    /// .unwrap();
    /// ```
    pub fn from_dialog_enhanced(
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>,
        request_uri: impl Into<String>,
        cseq: u32,
        local_address: SocketAddr,
        route_set: Vec<Uri>
    ) -> Self {
        Self {
            call_id: Some(call_id.into()),
            from_uri: Some(from_uri.into()),
            from_tag: Some(from_tag.into()),
            to_uri: Some(to_uri.into()),
            to_tag: Some(to_tag.into()),
            request_uri: Some(request_uri.into()),
            cseq,
            local_address: Some(local_address),
            route_set,
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
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
/// use rvoip_dialog_core::transaction::client::builders::RegisterBuilder;
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

/// Builder for general in-dialog requests
/// 
/// Provides a fluent interface for creating SIP requests within established dialogs.
/// This builder handles all the common in-dialog request patterns and can be used
/// for methods like REFER, UPDATE, INFO, NOTIFY, and custom methods.
/// 
/// # Example
/// ```
/// use rvoip_dialog_core::transaction::client::builders::InDialogRequestBuilder;
/// use rvoip_sip_core::Method;
/// use std::net::SocketAddr;
/// 
/// let local_addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
/// let refer = InDialogRequestBuilder::new(Method::Refer)
///     .from_dialog("call-123", "sip:alice@example.com", "tag-alice", 
///                  "sip:bob@example.com", "tag-bob", 2, local_addr)
///     .with_body("Refer-To: sip:charlie@example.com\r\n")
///     .with_content_type("message/sipfrag")
///     .build()
///     .unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct InDialogRequestBuilder {
    method: Method,
    call_id: Option<String>,
    from_uri: Option<String>,
    from_tag: Option<String>,
    to_uri: Option<String>,
    to_tag: Option<String>,
    request_uri: Option<String>,
    cseq: u32,
    local_address: Option<SocketAddr>,
    route_set: Vec<Uri>,
    body: Option<String>,
    content_type: Option<String>,
    event_type: Option<String>, // For NOTIFY requests
    custom_headers: Vec<TypedHeader>,
    max_forwards: u8,
}

impl InDialogRequestBuilder {
    /// Create a new in-dialog request builder for the specified method
    pub fn new(method: Method) -> Self {
        Self {
            method,
            call_id: None,
            from_uri: None,
            from_tag: None,
            to_uri: None,
            to_tag: None,
            request_uri: None,
            cseq: 1,
            local_address: None,
            route_set: Vec::new(),
            body: None,
            content_type: None,
            event_type: None,
            custom_headers: Vec::new(),
            max_forwards: 70,
        }
    }
    
    /// Set dialog information for the request
    pub fn from_dialog(
        mut self,
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>,
        cseq: u32,
        local_address: SocketAddr
    ) -> Self {
        let to_uri_string = to_uri.into();
        self.call_id = Some(call_id.into());
        self.from_uri = Some(from_uri.into());
        self.from_tag = Some(from_tag.into());
        self.to_uri = Some(to_uri_string.clone());
        self.to_tag = Some(to_tag.into());
        self.request_uri = Some(to_uri_string); // Default to To URI
        self.cseq = cseq;
        self.local_address = Some(local_address);
        self
    }
    
    /// Set enhanced dialog information with route set
    pub fn from_dialog_enhanced(
        mut self,
        call_id: impl Into<String>,
        from_uri: impl Into<String>,
        from_tag: impl Into<String>,
        to_uri: impl Into<String>,
        to_tag: impl Into<String>,
        request_uri: impl Into<String>,
        cseq: u32,
        local_address: SocketAddr,
        route_set: Vec<Uri>
    ) -> Self {
        self.call_id = Some(call_id.into());
        self.from_uri = Some(from_uri.into());
        self.from_tag = Some(from_tag.into());
        self.to_uri = Some(to_uri.into());
        self.to_tag = Some(to_tag.into());
        self.request_uri = Some(request_uri.into());
        self.cseq = cseq;
        self.local_address = Some(local_address);
        self.route_set = route_set;
        self
    }
    
    /// Set the request body
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
    
    /// Set the Content-Type header
    pub fn with_content_type(mut self, content_type: impl Into<String>) -> Self {
        self.content_type = Some(content_type.into());
        self
    }
    
    /// Set the request URI
    pub fn request_uri(mut self, uri: impl Into<String>) -> Self {
        self.request_uri = Some(uri.into());
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
    
    /// Set Max-Forwards
    pub fn max_forwards(mut self, max_forwards: u8) -> Self {
        self.max_forwards = max_forwards;
        self
    }
    
    /// Set the event type (for NOTIFY requests)
    pub fn with_event(mut self, event: impl Into<String>) -> Self {
        self.event_type = Some(event.into());
        self
    }
    
    /// Build the in-dialog request
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
        let mut builder = SimpleRequestBuilder::new(self.method.clone(), &request_uri)
            .map_err(|e| Error::Other(format!("Failed to create request builder: {}", e)))?;
        
        builder = builder
            .from("User", &from_uri, Some(&from_tag))
            .to("User", &to_uri, Some(&to_tag))
            .call_id(&call_id)
            .cseq(self.cseq)
            .via(&local_addr.to_string(), "UDP", Some(&branch))
            .max_forwards(self.max_forwards.into());
        
        // Add Event header for NOTIFY requests
        if let Some(event) = &self.event_type {
            let event_header = Event::new(EventType::Token(event.clone()));
            builder = builder.header(TypedHeader::Event(event_header));
        }
        
        // Add Route headers
        for route in self.route_set {
            builder = builder.header(TypedHeader::Route(Route::with_uri(route)));
        }
        
        // Add content if specified
        if let Some(body_content) = &self.body {
            if let Some(content_type) = &self.content_type {
                // Parse content type
                let parts: Vec<&str> = content_type.split('/').collect();
                if parts.len() == 2 {
                    builder = builder.header(TypedHeader::ContentType(
                        ContentType::from_type_subtype(parts[0], parts[1])
                    ));
                }
            }
            builder = builder
                .header(TypedHeader::ContentLength(ContentLength::new(body_content.len() as u32)))
                .body(body_content.as_bytes().to_vec());
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

impl InDialogRequestBuilder {
    /// Create a REFER request builder with dialog context
    /// 
    /// # Arguments
    /// * `target_uri` - URI to refer the call to
    /// 
    /// # Returns
    /// Pre-configured InDialogRequestBuilder for REFER
    pub fn for_refer(target_uri: impl Into<String>) -> Self {
        Self::new(Method::Refer)
            .with_body(format!("Refer-To: {}\r\n", target_uri.into()))
            .with_content_type("message/sipfrag")
    }
    
    /// Create an UPDATE request builder with dialog context
    /// 
    /// # Arguments
    /// * `sdp` - Optional SDP for media updates
    /// 
    /// # Returns
    /// Pre-configured InDialogRequestBuilder for UPDATE
    pub fn for_update(sdp: Option<String>) -> Self {
        let mut builder = Self::new(Method::Update);
        if let Some(sdp_content) = sdp {
            builder = builder.with_body(sdp_content).with_content_type("application/sdp");
        }
        builder
    }
    
    /// Create an INFO request builder with dialog context
    /// 
    /// # Arguments
    /// * `content` - Information content to send
    /// * `content_type` - Optional content type (defaults to "application/info")
    /// 
    /// # Returns
    /// Pre-configured InDialogRequestBuilder for INFO
    pub fn for_info(content: impl Into<String>, content_type: Option<String>) -> Self {
        Self::new(Method::Info)
            .with_body(content.into())
            .with_content_type(content_type.unwrap_or_else(|| "application/info".to_string()))
    }
    
    /// Create a NOTIFY request builder with dialog context
    /// 
    /// # Arguments
    /// * `event` - Event type for the notification
    /// * `body` - Optional notification body
    /// 
    /// # Returns
    /// Pre-configured InDialogRequestBuilder for NOTIFY
    pub fn for_notify(event: impl Into<String>, body: Option<String>) -> Self {
        let mut builder = Self::new(Method::Notify)
            .with_event(event);
        
        if let Some(notification_body) = body {
            builder = builder.with_body(notification_body);
        }
        
        builder
    }
    
    /// Create a MESSAGE request builder with dialog context
    /// 
    /// # Arguments
    /// * `content` - Message content
    /// * `content_type` - Optional content type (defaults to "text/plain")
    /// 
    /// # Returns
    /// Pre-configured InDialogRequestBuilder for MESSAGE
    pub fn for_message(content: impl Into<String>, content_type: Option<String>) -> Self {
        Self::new(Method::Message)
            .with_body(content.into())
            .with_content_type(content_type.unwrap_or_else(|| "text/plain".to_string()))
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