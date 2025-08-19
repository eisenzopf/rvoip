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