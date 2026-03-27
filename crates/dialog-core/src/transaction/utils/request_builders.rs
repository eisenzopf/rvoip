//! SIP request builder utilities for transaction-core
//! 
//! This module provides functions for creating various types of SIP requests
//! according to RFC 3261 specifications.

use uuid::Uuid;
use rvoip_sip_core::prelude::*;
use crate::transaction::error::{Error, Result};

use super::dialog_utils::generate_branch;

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

/// Create a test SIP request with the specified method
pub fn create_test_request(method: Method) -> Result<Request> {
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
    ).map_err(|e| Error::Other(format!("Failed to create Via header: {}", e)))?;

    let cseq = CSeq::new(1, method.clone());

    let builder = RequestBuilder::new(method, &request_uri_string)
        .map_err(|e| Error::Other(format!("Failed to create request builder: {}", e)))?;
    
    // Add headers
    Ok(builder
        .header(TypedHeader::From(From::new(from_addr)))
        .header(TypedHeader::To(To::new(to_addr)))
        .header(TypedHeader::Via(via))
        .header(TypedHeader::CallId(CallId::new(Uuid::new_v4().to_string())))
        .header(TypedHeader::CSeq(cseq))
        .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .header(TypedHeader::ContentLength(ContentLength::new(0)))
        .build())
}

/// Build a proxy-forwarded copy of an INVITE request (RFC 3261 §16.6).
///
/// Modifications applied:
/// 1. Decrement Max-Forwards by 1.
/// 2. Prepend a new Via header with the proxy's address and a fresh branch.
/// 3. Prepend a Record-Route header with `lr` so the proxy stays in the path.
/// 4. All other headers and the body are copied unchanged.
///
/// The caller is responsible for updating the Request-URI when the target
/// differs from the original (pass `target_uri = original.uri().to_string()`
/// to keep it the same).
pub fn create_forwarded_request(
    original: &Request,
    proxy_host: &str,
    proxy_port: u16,
    transport: &str,
    target_uri: &str,
) -> Result<Request> {
    use rvoip_sip_core::builder::SimpleRequestBuilder;

    let mut builder = SimpleRequestBuilder::new(original.method().clone(), target_uri)
        .map_err(|e| Error::Other(format!("Failed to create forwarded request builder: {}", e)))?;

    // Copy headers, adjusting Max-Forwards and skipping Via (we add ours below).
    let mut found_max_forwards = false;
    for header in &original.headers {
        match header {
            TypedHeader::Via(_) => {
                // Keep original Via headers — our Via is prepended after the loop.
                builder = builder.header(header.clone());
            }
            TypedHeader::MaxForwards(mf) => {
                // RFC 3261 §16.6 step 3: decrement Max-Forwards.
                let new_val = if mf.0 > 0 { mf.0 - 1 } else { 0 };
                builder = builder.header(TypedHeader::MaxForwards(MaxForwards::new(new_val)));
                found_max_forwards = true;
            }
            TypedHeader::ContentLength(_) => {
                // Will be recalculated when body is set.
            }
            _ => {
                builder = builder.header(header.clone());
            }
        }
    }
    if !found_max_forwards {
        builder = builder.header(TypedHeader::MaxForwards(MaxForwards::new(69)));
    }

    // RFC 3261 §16.6 step 2: add our Via on top with a fresh branch.
    let branch = format!("z9hG4bK-{}", Uuid::new_v4().simple());
    builder = builder.via(
        &format!("{}:{}", proxy_host, proxy_port),
        transport,
        Some(&branch),
    );

    // RFC 3261 §16.6 step 4: add Record-Route with lr (loose-routing).
    // Build: <sip:proxy_host:proxy_port;lr>
    let rr_uri = Uri::sip(proxy_host)
        .with_port(proxy_port)
        .with_parameter(Param::Lr);
    let rr_addr = Address::new(rr_uri);
    let rr_entry = RecordRouteEntry::new(rr_addr);
    let rr = RecordRoute::new(vec![rr_entry]);
    builder = builder.header(TypedHeader::RecordRoute(rr));

    // Copy body.
    if !original.body().is_empty() {
        builder = builder.body(original.body().to_vec());
    } else {
        builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
    }

    Ok(builder.build())
} 