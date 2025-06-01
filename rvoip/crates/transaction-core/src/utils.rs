use rand::{thread_rng, Rng};
// Use the prelude for easier access to common types
use rvoip_sip_core::prelude::*;
use std::str::FromStr;
use tracing::debug;

use crate::error::{self, Error, Result};
use crate::transaction::TransactionKey; // Import TransactionKey
use crate::transaction::TransactionKind; // Import Kind

use uuid::Uuid;
use std::net::SocketAddr;

/// Generate a random branch parameter for Via header (RFC 3261 magic cookie + random string)
pub fn generate_branch() -> String {
    format!("z9hG4bK-{}", Uuid::new_v4().simple())
}

/// Extract the branch parameter from the first Via header of a message
pub fn extract_branch(message: &Message) -> Option<String> {
    message
        .first_via() // Use the dedicated helper method
        .and_then(|via| via.branch().map(|s| s.to_string())) // Access the branch parameter directly
}

/// Extract the Call-ID value from a message
pub fn extract_call_id(message: &Message) -> Option<String> {
    message
        .header(&HeaderName::CallId)
        .and_then(|h| if let TypedHeader::CallId(cid) = h { Some(cid.to_string()) } else { None })
}

/// Extract the CSeq sequence number and method from a message
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    message
        .header(&HeaderName::CSeq)
        .and_then(|h| if let TypedHeader::CSeq(cseq) = h { Some((cseq.sequence(), cseq.method().clone())) } else { None })
}

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

/// Extract the transaction classification (prefix) and branch from a message
/// Used by manager to determine transaction type and potentially match.
pub fn extract_transaction_parts(message: &Message) -> Result<(TransactionKind, String)> {
    let branch = extract_branch(message)
        .ok_or_else(|| Error::Other("Missing branch parameter in Via header".to_string()))?;

    let kind = match message {
        Message::Request(req) => {
            match req.method() {
                 Method::Invite => TransactionKind::InviteServer,
                 Method::Ack => TransactionKind::InviteServer, // Matches existing IST
                 Method::Cancel => TransactionKind::InviteServer, // Matches existing IST
                 _ => TransactionKind::NonInviteServer,
             }
        }
        Message::Response(_) => {
            let (_, cseq_method) = extract_cseq(message)
                .ok_or_else(|| Error::Other("Missing or invalid CSeq header in Response".to_string()))?;

            if cseq_method == Method::Invite {
                TransactionKind::InviteClient
            } else {
                TransactionKind::NonInviteClient
            }
        }
    };

    Ok((kind, branch))
}

/// Extract a potential client transaction ID branch from a response.
/// Used by the manager to find the matching client transaction.
pub fn extract_client_branch_from_response(response: &Response) -> Option<String> {
    response.first_via()
        .and_then(|via| via.branch().map(|b| b.to_string()))
}

/// Extract the destination address from a transaction ID
///
/// NOTE: This is a temporary placeholder function. In a proper implementation,
/// this destination should be retrieved from the transaction registry.
/// The transaction manager now maintains a mapping of transaction IDs to their destinations
/// in the transaction_destinations field, which should be used instead of this function.
///
/// This function is kept for backward compatibility but will always return the testing configuration
/// which may not be correct for production usage.
pub fn extract_destination(_transaction_id: &str) -> Option<std::net::SocketAddr> {
    // This function is problematic and should be removed.
    // Destination should be stored with the transaction or derived differently.
    // Returning None to force callers to handle missing destination.
    debug!("WARNING: Using placeholder extract_destination. This function is deprecated and returns None.");
    None
    // Some(std::net::SocketAddr::from(([127, 0, 0, 1], 5071)))
}

/// Extract a transaction key from a SIP message if possible.
pub fn transaction_key_from_message(message: &Message) -> Option<TransactionKey> {
    match message {
        Message::Request(request) => {
            // Get Via header using TypedHeader
            if let Some(via) = request.typed_header::<Via>() {
                if let Some(first_via) = via.0.first() {
                    if let Some(branch) = first_via.branch() {
                        let method = request.method();
                        return Some(TransactionKey::new(branch.to_string(), method.clone(), true));
                    }
                }
            }
            None
        }
        Message::Response(response) => {
            // Get Via header using TypedHeader
            if let Some(via) = response.typed_header::<Via>() {
                if let Some(first_via) = via.0.first() {
                    if let Some(branch) = first_via.branch() {
                        // Get method from CSeq header
                        if let Some(cseq) = response.typed_header::<CSeq>() {
                            return Some(TransactionKey::new(branch.to_string(), cseq.method.clone(), false));
                        }
                    }
                }
            }
            None
        }
    }
}

/// Create an ACK request from an original INVITE and its response.
pub fn create_ack_from_invite(original_request: &Request, response: &Response) -> Result<Request> {
    // Get request URI from the original INVITE's Request-URI
    let request_uri = original_request.uri().to_string();
    let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri)?;
    
    // Copy Route headers from original INVITE (if present)
    for header in original_request.headers.iter() {
        if let TypedHeader::Route(route) = header {
            ack_builder = ack_builder.header(TypedHeader::Route(route.clone()));
        }
    }
    
    // Copy From, To, Call-ID from the original request and response
    if let Some(from) = original_request.typed_header::<From>() {
        ack_builder = ack_builder.header(TypedHeader::From(from.clone()));
    } else {
        return Err(Error::Other("Missing From header in original request".to_string()));
    }
    
    // Use To header from response to get the to-tag
    if let Some(to) = response.typed_header::<To>() {
        ack_builder = ack_builder.header(TypedHeader::To(to.clone()));
    } else {
        return Err(Error::Other("Missing To header in response".to_string()));
    }
    
    if let Some(call_id) = original_request.typed_header::<CallId>() {
        ack_builder = ack_builder.header(TypedHeader::CallId(call_id.clone()));
    } else {
        return Err(Error::Other("Missing Call-ID header in original request".to_string()));
    }
    
    // Create CSeq header for ACK (same seq as INVITE, but method is ACK)
    if let Some(cseq) = original_request.typed_header::<CSeq>() {
        let ack_cseq = CSeq {
            seq: cseq.seq,
            method: Method::Ack,
        };
        ack_builder = ack_builder.header(TypedHeader::CSeq(ack_cseq));
    } else {
        return Err(Error::Other("Missing CSeq header in original request".to_string()));
    }
    
    // Add Via header from original request (top Via only)
    if let Some(via) = original_request.typed_header::<Via>() {
        ack_builder = ack_builder.header(TypedHeader::Via(via.clone()));
    } else {
        return Err(Error::Other("Missing Via header in original request".to_string()));
    }
    
    // Build the ACK request
    Ok(ack_builder.build())
}

/// Determine which kind of transaction to create based on the request method.
pub fn determine_transaction_kind(request: &Request, is_server: bool) -> TransactionKind {
    match (request.method(), is_server) {
        (Method::Invite, true) => TransactionKind::InviteServer,
        (Method::Invite, false) => TransactionKind::InviteClient,
        (_, true) => TransactionKind::NonInviteServer,
        (_, false) => TransactionKind::NonInviteClient,
    }
}

/// Create a test SIP request with the specified method
pub fn create_test_request(method: Method) -> Request {
    // Create a Scheme and Host for URI
    let scheme = Scheme::Sip;
    let example_com = Host::Domain("example.com".to_string());
    let example_net = Host::Domain("example.net".to_string());
    
    // Create URIs and Address properly
    let from_uri = Uri::new(scheme.clone(), example_com.clone())
        .with_parameter(Param::tag(Uuid::new_v4().simple().to_string()));
    
    let from_addr = Address::new(from_uri);
    
    let to_addr = Address::new(Uri::new(scheme.clone(), example_net.clone()));
    
    // Format request URI as string (since RequestBuilder::new expects &str)
    let request_uri_string = Uri::new(scheme, example_net).to_string();
    
    // Create Via properly
    let via = Via::new(
        "SIP",              // protocol_name
        "2.0",              // protocol_version
        "UDP",              // transport
        "example.com",      // host
        None,               // port
        vec![Param::branch(&generate_branch())]  // parameters
    ).unwrap();  // Handle potential error
    
    let cseq = CSeq::new(1, method.clone());
    
    let builder = RequestBuilder::new(method, &request_uri_string).unwrap();  // Handle potential error
    
    // Add headers
    builder
        .header(TypedHeader::From(From::new(from_addr)))
        .header(TypedHeader::To(To::new(to_addr)))
        .header(TypedHeader::Via(via))
        .header(TypedHeader::CallId(CallId::new(Uuid::new_v4().to_string())))
        .header(TypedHeader::CSeq(cseq))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build()
}

/// Create an in-dialog request based on an existing dialog request
///
/// This is the **primary helper** that dialog-core should use instead of creating
/// SIP requests directly. It ensures proper RFC 3261 compliance for in-dialog requests.
///
/// An in-dialog request:
/// - Must be sent within an existing dialog (To tag must be present)
/// - Should have a new CSeq number (higher than previous requests)
/// - Should contain a Contact header
/// - Uses the dialog's established route and contact information
///
/// # RFC References
/// - RFC 3261 Section 12.2: Requests within a Dialog
/// - RFC 3261 Section 12.2.1.1: Generating the Request
///
/// # Arguments
/// * `dialog_request` - A previous in-dialog request to base the new request on
/// * `new_method` - The SIP method for the new request (BYE, INFO, REFER, etc.)
/// * `local_addr` - Local address to use in the Via header
/// * `body` - Optional body content (e.g., SDP for re-INVITE)
/// * `content_type` - Optional content type for the body
///
/// # Returns
/// * `Result<Request>` - The new in-dialog request or an error
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_transaction_core::utils::create_in_dialog_request;
/// # 
/// # // This would be a previous dialog request like an INVITE response
/// # let dialog_request = create_dialog_request(); 
/// # let local_addr = SocketAddr::from(([127, 0, 0, 1], 5060));
/// 
/// // Create a BYE request within the dialog
/// let bye_request = create_in_dialog_request(
///     &dialog_request,
///     Method::Bye,
///     &local_addr,
///     None,
///     None,
/// )?;
/// 
/// // Create an INFO request with message content
/// let info_body = Some("Application-specific info".to_string());
/// let info_request = create_in_dialog_request(
///     &dialog_request,
///     Method::Info,
///     &local_addr,
///     info_body,
///     Some("application/info"),
/// )?;
/// ```
pub fn create_in_dialog_request(
    dialog_request: &Request,
    new_method: Method,
    local_addr: &SocketAddr,
    body: Option<String>,
    content_type: Option<&str>,
) -> Result<Request> {
    // Validate that this request can be used as basis for an in-dialog request
    if dialog_request.to().is_none() || dialog_request.from().is_none() || 
       dialog_request.call_id().is_none() || dialog_request.cseq().is_none() {
        return Err(Error::Other("Invalid dialog request - missing required headers".to_string()));
    }
    
    // Check that this is an in-dialog request (To header must have a tag)
    if dialog_request.to().unwrap().tag().is_none() {
        return Err(Error::Other(format!(
            "Cannot create {} - not an in-dialog request (To tag missing)", 
            new_method
        )));
    }
    
    // Get dialog identifiers from the request
    let call_id = dialog_request.call_id().unwrap().clone();
    let to = dialog_request.to().unwrap().clone();
    let from = dialog_request.from().unwrap().clone();
    
    // Get CSeq and increment it for the new request
    let old_cseq = dialog_request.cseq().unwrap();
    let new_cseq_num = old_cseq.sequence() + 1;
    
    // Use the same Request-URI as the dialog request (RFC 3261 Section 12.2.1.1)
    let uri = dialog_request.uri().clone();
    let mut request = Request::new(new_method.clone(), uri);
    
    // Add dialog headers (RFC 3261 Section 12.2.1.1)
    request = request.with_header(TypedHeader::CallId(call_id));
    request = request.with_header(TypedHeader::To(to));
    request = request.with_header(TypedHeader::From(from));
    request = request.with_header(TypedHeader::CSeq(CSeq::new(new_cseq_num, new_method.clone())));
    
    // Create a Via header with a new branch parameter (RFC 3261 Section 8.1.1.7)
    let branch = generate_branch();
    let host = local_addr.ip().to_string();
    let port = Some(local_addr.port());
    let params = vec![rvoip_sip_core::types::Param::branch(branch)];
    let via = rvoip_sip_core::types::via::Via::new(
        "SIP", "2.0", "UDP", &host, port, params
    )?;
    request = request.with_header(TypedHeader::Via(via));
    
    // Copy Route headers from dialog request (if present) - RFC 3261 Section 12.2.1.1
    for header in dialog_request.headers.iter() {
        if let TypedHeader::Route(route) = header {
            request = request.with_header(TypedHeader::Route(route.clone()));
        }
    }
    
    // Create a Contact header - recommended for most in-dialog requests
    let contact_uri = format!("sip:{}:{}", local_addr.ip(), local_addr.port());
    let contact_addr = rvoip_sip_core::types::address::Address::new(
        rvoip_sip_core::types::uri::Uri::sip(&contact_uri)
    );
    let contact_param = rvoip_sip_core::types::contact::ContactParamInfo { address: contact_addr };
    let contact = rvoip_sip_core::types::contact::Contact::new_params(vec![contact_param]);
    request = request.with_header(TypedHeader::Contact(contact));
    
    // Add Max-Forwards header (RFC 3261 Section 8.1.1.4)
    request = request.with_header(TypedHeader::MaxForwards(rvoip_sip_core::types::max_forwards::MaxForwards::new(70)));
    
    // Add optional body content
    if let Some(body_content) = body {
        // Set Content-Type header if provided
        if let Some(ct) = content_type {
            let parts: Vec<&str> = ct.split('/').collect();
            if parts.len() == 2 {
                let content_type_header = ContentType::new(ContentTypeValue {
                    m_type: parts[0].to_string(),
                    m_subtype: parts[1].to_string(),
                    parameters: std::collections::HashMap::new(),
                });
                request = request.with_header(TypedHeader::ContentType(content_type_header));
            } else {
                return Err(Error::Other(format!("Invalid content type format: {}", ct)));
            }
        }
        
        // Set Content-Length header first (before converting to bytes consumes the body_content string)
        let content_length = rvoip_sip_core::types::content_length::ContentLength::new(body_content.len() as u32);
        request = request.with_header(TypedHeader::ContentLength(content_length));
        
        // Then add the body
        request = request.with_body(body_content.into_bytes());
    } else {
        // Set empty Content-Length
        let content_length = rvoip_sip_core::types::content_length::ContentLength::new(0);
        request = request.with_header(TypedHeader::ContentLength(content_length));
    }
    
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::Contact;
    use rvoip_sip_core::types::uri::Uri;
    use rvoip_sip_core::types::Scheme;
    use rvoip_sip_core::types::Host;
    use rvoip_sip_core::types::address::Address;

    #[test]
    fn test_create_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);

        // Check status code
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check version (as string instead of enum constant)
        assert_eq!(response.version.to_string(), "SIP/2.0");
        
        // Check headers were copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // Branch should exist but we don't check its exact value since it's generated
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        if let Some(TypedHeader::From(from)) = response.header(&HeaderName::From) {
            assert_eq!(from.address().uri.host, Host::Domain("example.com".to_string()));
        } else {
            panic!("Missing From header in response");
        }
        
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.address().uri.host, Host::Domain("example.net".to_string()));
            assert!(to.tag().is_none(), "To tag should not be present");
        } else {
            panic!("Missing To header in response");
        }
        
        if let Some(TypedHeader::CallId(_)) = response.header(&HeaderName::CallId) {
            // Call-ID exists, which is what we care about
        } else {
            panic!("Missing Call-ID header in response");
        }
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
            assert_eq!(cseq.sequence(), 1);
        } else {
            panic!("Missing CSeq header in response");
        }
        
        if let Some(TypedHeader::ContentLength(content_length)) = response.header(&HeaderName::ContentLength) {
            assert_eq!(content_length.0, 0);
        } else {
            panic!("Missing Content-Length header in response");
        }
    }

    #[test]
    fn test_create_response_with_to_tag() {
        let request = create_test_request(Method::Invite);
        let mut response = create_response(&request, StatusCode::Ok);

        // Add a tag to the To header
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            // Create a new To header with a tag
            let new_to = to.clone().with_tag("totag");
            
            // Replace the To header
            response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
            response.headers.push(TypedHeader::To(new_to));
        }

        // Check status code is correct
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check Via header was copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // We only check that a branch exists, not its specific value
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        // Check the To header now has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.tag().unwrap(), "totag");
        } else {
            panic!("Missing To header in response");
        }
    }

    #[test]
    fn test_create_trying_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Trying);
        assert_eq!(response.status(), StatusCode::Trying);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ringing_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ringing);
        assert_eq!(response.status(), StatusCode::Ringing);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ok_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);
        assert_eq!(response.status(), StatusCode::Ok);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_response_with_contact() {
        let request = create_test_request(Method::Invite);
        let mut response = create_response(&request, StatusCode::Ok);

        // Create a Contact with proper API according to documentation example:
        // 1. Create a URI
        let uri = Uri::new(Scheme::Sip, Host::Domain("example.org".to_string()));
        
        // 2. Create an Address with the URI
        let address = Address::new_with_display_name("Test User", uri);
        
        // 3. Create a ContactParamInfo with the Address
        let contact_info = ContactParamInfo { address };
        
        // 4. Create a Contact with the params
        let contact = Contact::new_params(vec![contact_info]);
        
        // Add the contact header
        response.headers.push(TypedHeader::Contact(contact.clone()));

        // Check status code is correct
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check Via header was copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // We only check that a branch exists, not its specific value
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        // Check the Contact header was added and has the correct address
        if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            // Get the first address and verify its properties
            if let Some(addr) = contact.address() {
                assert_eq!(addr.uri.host, Host::Domain("example.org".to_string()));
                // Verify display name
                assert_eq!(addr.display_name, Some("Test User".to_string()));
            } else {
                panic!("Contact has no address");
            }
        } else {
            panic!("Missing Contact header in response");
        }
    }

    #[test]
    fn test_create_ok_response_with_dialog_info() {
        let request = create_test_request(Method::Invite);
        let response = create_ok_response_with_dialog_info(&request, "alice", "192.168.1.100", Some(5060));

        // Check status code
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check that To header has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert!(to.tag().is_some(), "To header should have a tag");
            assert!(to.tag().unwrap().starts_with("tag-"), "To tag should start with 'tag-'");
        } else {
            panic!("Missing To header in response");
        }
        
        // Check that Contact header was added with proper URI
        if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            if let Some(addr) = contact.address() {
                assert_eq!(addr.uri.user, Some("alice".to_string()));
                assert_eq!(addr.uri.host, Host::Domain("192.168.1.100".to_string()));
                assert_eq!(addr.uri.port, Some(5060));
            } else {
                panic!("Contact has no address");
            }
        } else {
            panic!("Missing Contact header in response");
        }
    }

    #[test]
    fn test_create_ringing_response_with_tag() {
        let request = create_test_request(Method::Invite);
        let response = create_ringing_response_with_tag(&request);

        // Check status code
        assert_eq!(response.status(), StatusCode::Ringing);
        
        // Check that To header has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert!(to.tag().is_some(), "To header should have a tag");
            assert!(to.tag().unwrap().starts_with("tag-"), "To tag should start with 'tag-'");
        } else {
            panic!("Missing To header in response");
        }
        
        // Check that Contact header was NOT added (basic ringing response)
        assert!(response.header(&HeaderName::Contact).is_none(), 
               "Basic ringing response should not have Contact header");
    }

    #[test]
    fn test_create_ringing_response_with_dialog_info() {
        let request = create_test_request(Method::Invite);
        let response = create_ringing_response_with_dialog_info(&request, "bob", "10.0.0.1", None);

        // Check status code
        assert_eq!(response.status(), StatusCode::Ringing);
        
        // Check that To header has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert!(to.tag().is_some(), "To header should have a tag");
            assert!(to.tag().unwrap().starts_with("tag-"), "To tag should start with 'tag-'");
        } else {
            panic!("Missing To header in response");
        }
        
        // Check that Contact header was added with proper URI (no port)
        if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            if let Some(addr) = contact.address() {
                assert_eq!(addr.uri.user, Some("bob".to_string()));
                assert_eq!(addr.uri.host, Host::Domain("10.0.0.1".to_string()));
                assert_eq!(addr.uri.port, None); // No port specified
            } else {
                panic!("Contact has no address");
            }
        } else {
            panic!("Missing Contact header in response");
        }
    }
} 