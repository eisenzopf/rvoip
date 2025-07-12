//! Dialog-related utilities for transaction-core
//! 
//! This module provides utilities for working with SIP dialogs, including
//! templates for creating in-dialog requests and helper functions.

use std::str::FromStr;
use std::net::SocketAddr;
use uuid::Uuid;

use rvoip_sip_core::prelude::*;

/// Template for creating dialog-aware SIP requests
/// 
/// This template contains all the dialog-specific information needed to create
/// a proper in-dialog request using transaction-core helpers. It avoids direct
/// SIP message creation in dialog-core.
#[derive(Debug, Clone)]
pub struct DialogRequestTemplate {
    /// SIP method for the request
    pub method: Method,
    /// Target URI (where to send the request)  
    pub target_uri: Uri,
    /// Call-ID for this dialog
    pub call_id: String,
    /// Local URI (our identity)
    pub local_uri: Uri,
    /// Remote URI (their identity)
    pub remote_uri: Uri,
    /// Local tag
    pub local_tag: Option<String>,
    /// Remote tag
    pub remote_tag: Option<String>,
    /// CSeq sequence number to use
    pub cseq_number: u32,
    /// Route set to follow
    pub route_set: Vec<Uri>,
}

/// Generate a random branch parameter for Via header (RFC 3261 magic cookie + random string)
pub fn generate_branch() -> String {
    format!("z9hG4bK-{}", Uuid::new_v4().simple())
}

/// Create a proper in-dialog SIP request from a dialog template
/// 
/// This helper allows dialog-core to delegate request creation to transaction-core
/// for proper RFC 3261 compliance and architectural separation.
/// 
/// **NOTE**: This is a simplified implementation that provides basic functionality.
/// For production use, consider using more sophisticated request builders.
pub fn create_request_from_dialog_template(
    template: &DialogRequestTemplate,
    local_address: std::net::SocketAddr,
    body: Option<String>,
    content_type: Option<String>,
) -> Request {
    // Create a basic request using the deprecated Dialog::create_request as a fallback
    // This is a temporary solution until proper transaction-core builders are available
    let mut request = Request::new(template.method.clone(), template.target_uri.clone());
    
    // Add essential SIP headers manually
    
    // Call-ID
    request.headers.push(TypedHeader::CallId(
        CallId::new(template.call_id.clone())
    ));
    
    // From header with local tag
    let mut from_addr = Address::new(template.local_uri.clone());
    if let Some(local_tag) = &template.local_tag {
        from_addr.set_tag(local_tag);
    }
    request.headers.push(TypedHeader::From(From::new(from_addr)));
    
    // To header with remote tag
    let mut to_addr = Address::new(template.remote_uri.clone());
    if let Some(remote_tag) = &template.remote_tag {
        to_addr.set_tag(remote_tag);
    }
    request.headers.push(TypedHeader::To(To::new(to_addr)));
    
    // CSeq header
    request.headers.push(TypedHeader::CSeq(
        CSeq::new(template.cseq_number, template.method.clone())
    ));
    
    // Via header with local address and new branch
    let via = Via::new(
        "SIP",
        "2.0", 
        "UDP",
        &local_address.ip().to_string(),
        Some(local_address.port()),
        vec![Param::branch(&generate_branch())]
    ).unwrap_or_else(|e| {
        // Log the error for debugging
        eprintln!("Failed to create Via header with local address {}: {}", local_address, e);
        
        // Try a simpler fallback without branch parameter
        Via::new("SIP", "2.0", "UDP", &local_address.ip().to_string(), Some(local_address.port()), vec![])
            .unwrap_or_else(|e2| {
                // Log the second error and panic - we should never reach this point
                eprintln!("Critical error: Failed to create Via header even without branch parameter: {}", e2);
                panic!("Unable to create Via header with local address {}", local_address);
            })
    });
    request.headers.push(TypedHeader::Via(via));
    
    // Max-Forwards
    request.headers.push(TypedHeader::MaxForwards(MaxForwards::new(70)));
    
    // Contact header (for dialog-creating and target refresh requests)
    if matches!(template.method, Method::Invite | Method::Subscribe | Method::Update) {
        let contact_uri = Uri::new(
            Scheme::Sip,
            Host::Address(local_address.ip())
        ).with_port(local_address.port());
        
        let contact_addr = Address::new(contact_uri);
        let contact_info = ContactParamInfo { address: contact_addr };
        let contact = Contact::new_params(vec![contact_info]);
        request.headers.push(TypedHeader::Contact(contact));
    }
    
    // Route headers (if route set exists)
    for route_uri in &template.route_set {
        let route_addr = Address::new(route_uri.clone());
        let route_value = ParserRouteValue(route_addr);
        let route = Route::new(vec![route_value]);
        request.headers.push(TypedHeader::Route(route));
    }
    
    // Content headers and body
    if let Some(body_content) = body {
        let body_bytes = body_content.into_bytes();
        
        // Content-Length
        request.headers.push(TypedHeader::ContentLength(
            ContentLength::new(body_bytes.len() as u32)
        ));
        
        // Content-Type 
        if let Some(ct) = content_type {
            if let Ok(content_type_header) = ContentType::from_str(&ct) {
                request.headers.push(TypedHeader::ContentType(content_type_header));
            }
        }
        
        request.body = body_bytes.into();
    } else {
        // Explicit Content-Length: 0 when no body
        request.headers.push(TypedHeader::ContentLength(ContentLength::new(0)));
    }
    
    request
} 