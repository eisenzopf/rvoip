//! SIP response builders for transaction-core
//!
//! This module provides convenient functions for creating various types of SIP responses
//! according to RFC 3261 specifications.

use rvoip_sip_core::prelude::*;
use std::str::FromStr;
use uuid::Uuid;

/// Create a response based on a request
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut builder = ResponseBuilder::new(status, None);

    // RFC 3261 §8.2.6.2: every Via of the request, in order. A proxy adds
    // its own Via line, so copying only the first one drops the client's.
    for header in request
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Via(_)))
    {
        builder = builder.header(header.clone());
    }
    // RFC 3261 §12.1.1: echo Record-Route so a dialog-forming response
    // teaches the UAC its route set. Proxies ignore it elsewhere.
    for header in request
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::RecordRoute(_)))
    {
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

    let mut response = builder.build();
    // RFC 3891 §6.2: every response this stack authors says it understands
    // `Replaces`. Stamped here rather than at each call site because this is
    // the shared builder behind the stack's own rejections, and a 481 that
    // still advertises the capability tells the peer the dialog was missing
    // rather than the extension.
    crate::manager::transaction_integration::inject_replaces_support_response(&mut response);
    response
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
    let methods_str = allowed_methods
        .iter()
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
    response
        .headers
        .push(TypedHeader::Expires(Expires::new(expires)));

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
    contact_port: Option<u16>,
) -> Response {
    // Generate a unique To-tag for this dialog
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());

    // Start with basic response
    let mut response = create_response(request, StatusCode::Ok);

    // Update the To header to include the tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);

        // Replace the To header
        response
            .headers
            .retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }

    // Create Contact header using proper sip-core URI builder
    let mut contact_uri = Uri::sip(contact_host).with_user(contact_user);
    if let Some(port) = contact_port {
        contact_uri = contact_uri.with_port(port);
    }

    let contact_addr = Address::new(contact_uri);
    let contact_info = ContactParamInfo {
        address: contact_addr,
    };
    let contact = Contact::new_params(vec![contact_info]);
    response.headers.push(TypedHeader::Contact(contact));

    response
}

/// Create a 200 OK response for INVITE using an explicit Contact URI.
pub fn create_ok_response_with_contact_uri(
    request: &Request,
    contact_uri: &str,
) -> std::result::Result<Response, rvoip_sip_core::error::Error> {
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());
    let mut response = create_response(request, StatusCode::Ok);

    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);
        response
            .headers
            .retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }

    let contact_addr = Address::new(Uri::from_str(contact_uri)?);
    let contact = Contact::new_params(vec![ContactParamInfo {
        address: contact_addr,
    }]);
    response.headers.push(TypedHeader::Contact(contact));

    Ok(response)
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
        response
            .headers
            .retain(|h| !matches!(h, TypedHeader::To(_)));
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
    contact_port: Option<u16>,
) -> Response {
    // Generate a unique To-tag for this early dialog
    let to_tag = format!("tag-{}", Uuid::new_v4().simple());

    // Start with basic ringing response
    let mut response = create_ringing_response(request);

    // Update the To header to include the tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        let new_to = to.clone().with_tag(&to_tag);

        // Replace the To header
        response
            .headers
            .retain(|h| !matches!(h, TypedHeader::To(_)));
        response.headers.push(TypedHeader::To(new_to));
    }

    // Create Contact header using proper sip-core URI builder
    let mut contact_uri = Uri::sip(contact_host).with_user(contact_user);
    if let Some(port) = contact_port {
        contact_uri = contact_uri.with_port(port);
    }

    let contact_addr = Address::new(contact_uri);
    let contact_info = ContactParamInfo {
        address: contact_addr,
    };
    let contact = Contact::new_params(vec![contact_info]);
    response.headers.push(TypedHeader::Contact(contact));

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_request(wire: &str) -> Request {
        match rvoip_sip_core::parse_message(wire.as_bytes()).expect("request parses") {
            Message::Request(request) => request,
            Message::Response(_) => panic!("expected a request"),
        }
    }

    fn via_lines(response: &Response) -> Vec<String> {
        Message::Response(response.clone())
            .to_string()
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("via:"))
            .map(str::to_owned)
            .collect()
    }

    fn bye_with_vias(via_block: &str) -> Request {
        parse_request(&format!(
            "BYE sip:edge@10.244.5.46:5060 SIP/2.0\r\n\
             {via_block}\
             Max-Forwards: 69\r\n\
             From: <sip:load@10.244.5.82>;tag=0\r\n\
             To: <sip:edge@10.244.5.46>;tag=23fe5d34\r\n\
             Call-ID: rust-load-0-0-21360\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    const PROXY_VIA: &str = "SIP/2.0/TCP 10.244.4.137:15070;branch=z9hG4bK0acd.5180fb6621701fc84f1af91379feb36c.0;received=10.244.4.137";
    const CLIENT_VIA: &str = "SIP/2.0/UDP 10.244.5.82:5090;branch=z9hG4bK-bye-rust-load-0-0-21360";

    #[test]
    fn response_keeps_every_separate_via_line_in_order() {
        let request = bye_with_vias(&format!("Via: {PROXY_VIA}\r\nVia: {CLIENT_VIA}\r\n"));
        let response = create_response(&request, StatusCode::Ok);

        let vias = via_lines(&response);
        assert_eq!(vias.len(), 2, "{vias:?}");
        assert!(
            vias[0].contains("SIP/2.0/TCP 10.244.4.137:15070"),
            "{vias:?}"
        );
        assert!(vias[0].contains("received=10.244.4.137"), "{vias:?}");
        assert!(vias[1].contains("SIP/2.0/UDP 10.244.5.82:5090"), "{vias:?}");
        assert!(
            vias[1].contains("branch=z9hG4bK-bye-rust-load-0-0-21360"),
            "{vias:?}"
        );
    }

    #[test]
    fn response_keeps_a_single_via() {
        let request = bye_with_vias(&format!("Via: {CLIENT_VIA}\r\n"));
        let vias = via_lines(&create_response(&request, StatusCode::Ok));
        assert_eq!(vias.len(), 1, "{vias:?}");
        assert!(vias[0].contains("10.244.5.82:5090"), "{vias:?}");
    }

    #[test]
    fn response_keeps_three_vias_mixing_lines_and_commas() {
        let request = bye_with_vias(&format!(
            "Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bK-first;rport=5060;received=192.0.2.1\r\n\
             Via: {PROXY_VIA}, {CLIENT_VIA}\r\n"
        ));
        let response = create_response(&request, StatusCode::Ok);
        let wire = Message::Response(response.clone()).to_string();

        let order = [
            wire.find("z9hG4bK-first"),
            wire.find("z9hG4bK0acd.5180fb6621701fc84f1af91379feb36c.0"),
            wire.find("z9hG4bK-bye-rust-load-0-0-21360"),
        ];
        assert!(order.iter().all(Option::is_some), "{wire}");
        assert!(order[0] < order[1] && order[1] < order[2], "{wire}");
        assert!(wire.contains("rport=5060"), "{wire}");

        let reparsed = match rvoip_sip_core::parse_message(wire.as_bytes()).expect("reparse") {
            Message::Response(response) => response,
            Message::Request(_) => panic!("expected a response"),
        };
        let branches: Vec<String> = reparsed
            .via_headers()
            .iter()
            .flat_map(|via| via.headers().iter().cloned().collect::<Vec<_>>())
            .filter_map(|entry| entry.branch().map(str::to_owned))
            .collect();
        assert_eq!(
            branches,
            vec![
                "z9hG4bK-first",
                "z9hG4bK0acd.5180fb6621701fc84f1af91379feb36c.0",
                "z9hG4bK-bye-rust-load-0-0-21360",
            ]
        );
    }

    #[test]
    fn response_keeps_dialog_identity() {
        let request = bye_with_vias(&format!("Via: {PROXY_VIA}\r\nVia: {CLIENT_VIA}\r\n"));
        let response = create_response(&request, StatusCode::Ok);

        assert_eq!(
            response.call_id().map(|id| id.to_string()),
            Some("rust-load-0-0-21360".to_string())
        );
        let cseq = response.cseq().expect("CSeq");
        assert_eq!((cseq.seq, cseq.method.clone()), (2, Method::Bye));
        assert_eq!(response.from().and_then(|from| from.tag()), Some("0"));
        assert_eq!(response.to().and_then(|to| to.tag()), Some("23fe5d34"));
    }
}
