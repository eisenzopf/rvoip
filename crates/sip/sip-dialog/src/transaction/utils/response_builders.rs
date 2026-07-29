//! SIP response builders for transaction-core
//!
//! This module provides convenient functions for creating various types of SIP responses
//! according to RFC 3261 specifications.

use crate::transaction::TransactionKey;
use rvoip_sip_core::prelude::*;
use sha2::{Digest, Sha256};
use std::str::FromStr;

const LOCAL_TO_TAG_DOMAIN: &[u8] = b"rvoip-local-response-tag-v1";
const LOCAL_TO_TAG_PREFIX: &str = "rvoip-";
const LOCAL_TO_TAG_DIGEST_BYTES: usize = 16;
const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";

/// Create a response based on a request.
///
/// RFC 3261 section 8.2.6.2 requires a UAS to add a To tag to every
/// response other than 100 (Trying) when the request's To header is
/// untagged. The compatibility entry point derives that tag from the SIP
/// transaction-identifying fields in the request, so independently built
/// responses for the same request retain the same tag.
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    create_response_inner(request, status, None)
}

/// Create a response for one exact internal transaction generation.
///
/// This is the authoritative form for transaction-manager generated
/// responses. Including the exact [`TransactionKey`] and its internal
/// generation keeps tags stable across retransmissions while distinguishing
/// separate admitted generations even when malformed or hostile peers reuse
/// the same on-wire identifiers.
///
/// The existing [`create_response`] API remains available for callers that do
/// not own an exact transaction generation.
pub fn create_response_for_transaction_generation(
    request: &Request,
    status: StatusCode,
    transaction_id: &TransactionKey,
    generation: u64,
) -> Response {
    create_response_inner(request, status, Some((transaction_id, generation)))
}

fn create_response_inner(
    request: &Request,
    status: StatusCode,
    transaction_identity: Option<(&TransactionKey, u64)>,
) -> Response {
    let mut builder = ResponseBuilder::new(status, None);

    // Copy needed headers from request to response using the header method
    for header in request
        .headers
        .iter()
        .filter(|header| matches!(header, TypedHeader::Via(_)))
    {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::To) {
        let header = match header {
            TypedHeader::To(to) if status.as_u16() > 100 && to.tag().is_none() => {
                let tag = derive_local_to_tag(request, transaction_identity);
                TypedHeader::To(to.clone().with_tag(tag))
            }
            _ => header.clone(),
        };
        builder = builder.header(header);
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

fn derive_local_to_tag(
    request: &Request,
    transaction_identity: Option<(&TransactionKey, u64)>,
) -> String {
    let mut hasher = Sha256::new();
    update_digest_component(&mut hasher, LOCAL_TO_TAG_DOMAIN);

    if let Some((transaction_id, generation)) = transaction_identity {
        update_digest_component(&mut hasher, b"exact-transaction-generation");
        update_digest_component(&mut hasher, transaction_id.branch().as_bytes());
        update_digest_component(&mut hasher, transaction_id.method().to_string().as_bytes());
        update_digest_component(&mut hasher, &[u8::from(transaction_id.is_server())]);
        update_digest_component(&mut hasher, &generation.to_be_bytes());
    } else {
        update_digest_component(&mut hasher, b"request-transaction-fingerprint");
        update_digest_component(&mut hasher, request.method().to_string().as_bytes());
        update_digest_component(&mut hasher, request.uri().to_string().as_bytes());
        update_digest_optional(&mut hasher, request.via_branch());
        update_digest_optional(
            &mut hasher,
            request.call_id().map(|call_id| call_id.value()),
        );
        update_digest_optional(
            &mut hasher,
            request.cseq_number().map(|sequence| sequence.to_string()),
        );
        update_digest_optional(
            &mut hasher,
            request.cseq().map(|cseq| cseq.method().to_string()),
        );
        update_digest_optional(&mut hasher, request.from_tag());
        update_digest_optional(&mut hasher, request.from_uri());
        update_digest_optional(&mut hasher, request.to_uri());
    }

    encode_local_to_tag(&hasher.finalize()[..LOCAL_TO_TAG_DIGEST_BYTES])
}

fn update_digest_optional<T>(hasher: &mut Sha256, value: Option<T>)
where
    T: AsRef<[u8]>,
{
    match value {
        Some(value) => {
            hasher.update([1]);
            update_digest_component(hasher, value.as_ref());
        }
        None => hasher.update([0]),
    }
}

fn update_digest_component(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn encode_local_to_tag(digest: &[u8]) -> String {
    let mut tag = String::with_capacity(LOCAL_TO_TAG_PREFIX.len() + LOCAL_TO_TAG_DIGEST_BYTES * 2);
    tag.push_str(LOCAL_TO_TAG_PREFIX);
    for byte in digest {
        tag.push(LOWER_HEX[usize::from(byte >> 4)] as char);
        tag.push(LOWER_HEX[usize::from(byte & 0x0f)] as char);
    }
    tag
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
    // Start with basic response
    let mut response = create_response(request, StatusCode::Ok);

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
    let mut response = create_response(request, StatusCode::Ok);

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
    create_ringing_response(request)
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
    // Start with basic ringing response
    let mut response = create_ringing_response(request);

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
