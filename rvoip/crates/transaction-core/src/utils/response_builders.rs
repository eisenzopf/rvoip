//! SIP response builders for transaction-core
//! 
//! This module provides convenient functions for creating various types of SIP responses
//! according to RFC 3261 specifications.

use std::str::FromStr;
use uuid::Uuid;
use rvoip_sip_core::prelude::*;

/// Create a response based on a request
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut builder = ResponseBuilder::new(status, None);
    
    // Copy needed headers from request to response using the header method
    if let Some(header) = request.header(&HeaderName::Via) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::To) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CallId) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CSeq) {
        builder = builder.header(header.clone());
    }
    
    // Add Content-Length: 0
    builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    builder.build()
}

/// Convenience method to create a 100 Trying response
pub fn create_trying_response(request: &Request) -> Response {
    create_response(request, StatusCode::Trying)
}

/// Convenience method to create a 180 Ringing response
pub fn create_ringing_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ringing)
}

/// Convenience method to create a 200 OK response
pub fn create_ok_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
}

/// Create a 200 OK response for BYE requests
/// 
/// This function creates a simple 200 OK response for BYE requests.
/// Unlike INVITE responses, BYE responses don't need To-tags (dialog already established)
/// or Contact headers (dialog is being terminated).
/// 
/// # Arguments
/// * `request` - The original BYE request
/// 
/// # Returns
/// A simple 200 OK response for BYE termination
pub fn create_ok_response_for_bye(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
}

/// Create a 200 OK response for CANCEL requests
/// 
/// This function creates a simple 200 OK response for CANCEL requests.
/// CANCEL responses are always simple 200 OK responses without additional headers.
/// 
/// # Arguments
/// * `request` - The original CANCEL request
/// 
/// # Returns
/// A simple 200 OK response for CANCEL acknowledgment
pub fn create_ok_response_for_cancel(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
}

/// Create a 200 OK response for OPTIONS requests with Allow header
/// 
/// This function creates a 200 OK response for OPTIONS requests that includes
/// an Allow header listing the supported SIP methods.
/// 
/// # Arguments
/// * `request` - The original OPTIONS request
/// * `allowed_methods` - List of methods supported by this server/UA
/// 
/// # Returns
/// A 200 OK response with Allow header for OPTIONS capability query
pub fn create_ok_response_for_options(request: &Request, allowed_methods: &[Method]) -> Response {
    let mut response = create_response(request, StatusCode::Ok);
    
    // Add Allow header with supported methods
    let methods_str = allowed_methods.iter()
        .map(|m| m.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    
    // Create Allow header using proper typed header
    let allow = rvoip_sip_core::types::allow::Allow::from_str(&methods_str)
        .unwrap_or_else(|_| rvoip_sip_core::types::allow::Allow::new());
    
    response.headers.push(TypedHeader::Allow(allow));
    
    response
}

/// Create a 200 OK response for MESSAGE requests
/// 
/// This function creates a simple 200 OK response for MESSAGE requests.
/// MESSAGE responses are typically simple acknowledgments.
/// 
/// # Arguments
/// * `request` - The original MESSAGE request
/// 
/// # Returns
/// A simple 200 OK response for MESSAGE acknowledgment
pub fn create_ok_response_for_message(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
}

/// Create a 200 OK response for REGISTER requests with Contact and Expires
/// 
/// This function creates a 200 OK response for REGISTER requests that includes
/// the registered Contact header and Expires value.
/// 
/// # Arguments
/// * `request` - The original REGISTER request
/// * `expires` - The registration expiration time in seconds
/// 
/// # Returns
/// A 200 OK response with Contact and Expires headers for REGISTER confirmation
pub fn create_ok_response_for_register(request: &Request, expires: u32) -> Response {
    let mut response = create_response(request, StatusCode::Ok);
    
    // Copy Contact header from request (if present)
    if let Some(contact_header) = request.header(&HeaderName::Contact) {
        response.headers.push(contact_header.clone());
    }
    
    // Add Expires header using proper typed header
    response.headers.push(TypedHeader::Expires(Expires::new(expires)));
    
    response
}

/// Create a 200 OK response with To-tag and Contact header for dialog establishment
/// 
/// This function creates a proper 200 OK response for INVITE requests that includes:
/// - A generated To-tag for dialog identification
/// - A Contact header for future in-dialog requests
/// - All standard headers copied from the request
/// 
/// # Arguments
/// * `request` - The original INVITE request
/// * `contact_user` - The user part for the Contact URI (e.g., "server", "alice", etc.)
/// * `contact_host` - The host/IP for the Contact URI (e.g., "192.168.1.1")
/// * `contact_port` - Optional port for the Contact URI
/// 
/// # Returns
/// A 200 OK response ready for dialog establishment
pub fn create_ok_response_with_dialog_info(
    request: &Request, 
    contact_user: &str, 
    contact_host: &str, 
    contact_port: Option<u16>
) -> Response {
    // Generate a unique To-tag for this dialog
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    // Start with basic response
    let mut response = create_response(request, StatusCode::Ok);
    
    // Update the To header to include the tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);
        
        // Replace the To header
        response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }
    
    // Create Contact header using proper sip-core URI builder
    let mut contact_uri = Uri::sip(contact_host).with_user(contact_user);
    if let Some(port) = contact_port {
        contact_uri = contact_uri.with_port(port);
    }
    
    let contact_addr = Address::new(contact_uri);
    let contact_info = ContactParamInfo { address: contact_addr };
    let contact = Contact::new_params(vec![contact_info]);
    response.headers.push(TypedHeader::Contact(contact));
    
    response
}

/// Create a 180 Ringing response with To-tag for early dialog establishment
/// 
/// This function creates a 180 Ringing response that includes a To-tag,
/// which establishes an early dialog state.
/// 
/// # Arguments
/// * `request` - The original INVITE request
/// 
/// # Returns
/// A 180 Ringing response with To-tag for early dialog
pub fn create_ringing_response_with_tag(request: &Request) -> Response {
    // Generate a unique To-tag for this early dialog
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    // Start with basic ringing response
    let mut response = create_ringing_response(request);
    
    // Update the To header to include the tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);
        
        // Replace the To header
        response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }
    
    response
}

/// Create a 180 Ringing response with To-tag and Contact header for early dialog
/// 
/// This function creates a 180 Ringing response that includes both a To-tag
/// and Contact header for early dialog establishment with media capabilities.
/// 
/// # Arguments
/// * `request` - The original INVITE request
/// * `contact_user` - The user part for the Contact URI (e.g., "server", "alice", etc.)
/// * `contact_host` - The host/IP for the Contact URI (e.g., "192.168.1.1")
/// * `contact_port` - Optional port for the Contact URI
/// 
/// # Returns
/// A 180 Ringing response with To-tag and Contact header
pub fn create_ringing_response_with_dialog_info(
    request: &Request, 
    contact_user: &str, 
    contact_host: &str, 
    contact_port: Option<u16>
) -> Response {
    // Generate a unique To-tag for this early dialog
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());
    
    // Start with basic ringing response
    let mut response = create_ringing_response(request);
    
    // Update the To header to include the tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);
        
        // Replace the To header
        response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }
    
    // Create Contact header using proper sip-core URI builder
    let mut contact_uri = Uri::sip(contact_host).with_user(contact_user);
    if let Some(port) = contact_port {
        contact_uri = contact_uri.with_port(port);
    }
    
    let contact_addr = Address::new(contact_uri);
    let contact_info = ContactParamInfo { address: contact_addr };
    let contact = Contact::new_params(vec![contact_info]);
    response.headers.push(TypedHeader::Contact(contact));
    
    response
} 