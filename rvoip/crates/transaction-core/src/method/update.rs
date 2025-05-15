//! UPDATE method utilities for SIP transactions
//!
//! Implements special handling for UPDATE requests according to RFC 3311.

use std::net::SocketAddr;
use rvoip_sip_core::prelude::*;
use crate::error::{Error, Result};

/// Creates an UPDATE request based on an existing dialog
///
/// According to RFC 3311, UPDATE is used to modify aspects of a session
/// without changing the dialog state. It is particularly useful for
/// updating session parameters (like SDP) before the session is established.
///
/// An UPDATE request:
/// - Must be sent within an existing dialog
/// - Should have a new CSeq number
/// - Should contain a Contact header
/// - Typically includes SDP to modify session parameters
///
/// # Arguments
/// * `dialog_request` - A previous in-dialog request (like INVITE) to base the UPDATE on
/// * `local_addr` - Local address to use in the Via header
/// * `new_sdp` - Optional SDP to include in the UPDATE
///
/// # Returns
/// A new UPDATE request
pub fn create_update_request(
    dialog_request: &Request,
    local_addr: &SocketAddr,
    new_sdp: Option<String>,
) -> Result<Request> {
    // Validate that this request can be used as basis for an UPDATE
    if dialog_request.to().is_none() || dialog_request.from().is_none() || 
       dialog_request.call_id().is_none() || dialog_request.cseq().is_none() {
        return Err(Error::Other("Invalid dialog request - missing required headers".to_string()));
    }
    
    // Get dialog identifiers from the request
    let call_id = dialog_request.call_id().unwrap().clone();
    let to = dialog_request.to().unwrap().clone();
    let from = dialog_request.from().unwrap().clone();
    
    // Get CSeq and increment it
    let old_cseq = dialog_request.cseq().unwrap();
    let new_cseq_num = old_cseq.sequence() + 1;
    
    // Create a new UPDATE request
    let uri = dialog_request.uri().clone();
    let mut update = Request::new(Method::Update, uri);
    
    // Add headers
    update = update.with_header(TypedHeader::CallId(call_id));
    update = update.with_header(TypedHeader::To(to));
    update = update.with_header(TypedHeader::From(from));
    update = update.with_header(TypedHeader::CSeq(CSeq::new(new_cseq_num, Method::Update)));
    
    // Add optional SDP body
    if let Some(sdp) = new_sdp {
        let content_type = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        update = update.with_header(TypedHeader::ContentType(content_type));
        update = update.with_body(sdp.into_bytes());
    }
    
    Ok(update)
}

/// Validates that an UPDATE request meets the requirements of RFC 3311
///
/// UPDATE requests:
/// - Must be sent within a dialog (To tag must be present)
/// - Should contain a Contact header (warning issued but validation passes)
/// - Typically modify session parameters using SDP
pub fn validate_update_request(request: &Request) -> Result<()> {
    // Check that this is an UPDATE request
    if request.method() != Method::Update {
        return Err(Error::Other("Request method is not UPDATE".to_string()));
    }
    
    // Check that it has the required headers
    if request.call_id().is_none() {
        return Err(Error::Other("UPDATE request missing Call-ID header".to_string()));
    }
    
    if request.from().is_none() {
        return Err(Error::Other("UPDATE request missing From header".to_string()));
    }
    
    // To header must be present and must have a tag for in-dialog requests
    match request.to() {
        None => return Err(Error::Other("UPDATE request missing To header".to_string())),
        Some(to) => {
            let to_tag = to.tag();
            println!("DEBUG: To tag value from request: {:?}", to_tag);
            if to_tag.is_none() || to_tag.unwrap().is_empty() {
                return Err(Error::Other("UPDATE request To header missing tag (must be in-dialog)".to_string()));
            }
        }
    }
    
    if request.cseq().is_none() {
        return Err(Error::Other("UPDATE request missing CSeq header".to_string()));
    }
    
    // UPDATE should have a Contact header - just log a warning but don't fail validation
    // according to RFC 3311 it's recommended but not strictly required
    let has_contact = request.headers.iter().any(|h| matches!(h, TypedHeader::Contact(_)));
    if !has_contact {
        // Log a warning but continue
        use tracing::warn;
        warn!("UPDATE request missing recommended Contact header");
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    
    #[test]
    fn test_validate_update_request() {
        // Create a minimal but valid UPDATE request
        let mut request = Request::new(Method::Update, Uri::from_str("sip:alice@example.com").unwrap());
        
        // Create required headers
        let call_id = CallId::new("test-call-id");
        
        // Create From header with tag in the address
        let from_uri = Uri::from_str("sip:alice@example.com").unwrap();
        let from_addr = Address {
            display_name: Some("Alice".to_string()),
            uri: from_uri,
            params: vec![Param::tag("alice-tag".to_string())],
        };
        let from = From::new(from_addr);
        
        // Create To header with tag in the address
        let to_uri = Uri::from_str("sip:bob@example.com").unwrap();
        let to_addr = Address {
            display_name: Some("Bob".to_string()),
            uri: to_uri,
            params: vec![Param::tag("bob-tag".to_string())],
        };
        let to = To::new(to_addr);
        
        let cseq = CSeq::new(1, Method::Update);
        let contact_addr = Address::new(Uri::from_str("sip:alice@192.168.1.2:5060").unwrap());
        let contact_param = ContactParamInfo { address: contact_addr };
        let contact = Contact::new_params(vec![contact_param]);
        
        request = request
            .with_header(TypedHeader::CallId(call_id.clone()))
            .with_header(TypedHeader::From(from.clone()))
            .with_header(TypedHeader::To(to.clone()))
            .with_header(TypedHeader::CSeq(cseq.clone()))
            .with_header(TypedHeader::Contact(contact.clone()));
        
        // Validate the request - should be valid
        println!("DEBUG: Validating initial request with To tag: {:?}", request.to().unwrap().tag());
        assert!(validate_update_request(&request).is_ok());
        
        // Test with incorrect method - should fail validation
        let mut method_request = request.clone();
        method_request.method = Method::Info;
        assert!(validate_update_request(&method_request).is_err(),
                "Request with non-UPDATE method should fail validation");
        
        // Test without To tag
        
        // Create a To header with no tag at all
        let to_addr_no_tag = Address::new(Uri::from_str("sip:bob@example.com").unwrap());
        println!("DEBUG: To header with no tag params: {:?}", to_addr_no_tag.params);
        let to_no_tag = To::new(to_addr_no_tag);
        println!("DEBUG: To header with no tag (from To): {:?}", to_no_tag.tag());
        
        // Create a completely fresh request with the no-tag To header
        let mut no_tag_request = Request::new(Method::Update, Uri::from_str("sip:bob@example.com").unwrap());
        no_tag_request = no_tag_request
            .with_header(TypedHeader::CallId(call_id.clone()))
            .with_header(TypedHeader::From(from.clone()))
            .with_header(TypedHeader::To(to_no_tag.clone()))
            .with_header(TypedHeader::CSeq(cseq.clone()));
        
        // Verify the request has a To header with no tag
        println!("DEBUG: No tag request To header: {:?}", no_tag_request.to());
        if let Some(no_tag_to) = no_tag_request.to() {
            println!("DEBUG: No tag request To tag: {:?}", no_tag_to.tag());
            assert_eq!(no_tag_to.tag(), None, "Should have no tag");
        }
        
        // Validate should fail
        let result = validate_update_request(&no_tag_request);
        println!("DEBUG: Validation result for no tag: {:?}", result);
        assert!(result.is_err(), "UPDATE request without To tag should fail validation");
        
        // Test with empty tag
        // Create a To header with an empty tag
        let to_addr_empty_tag = Address {
            display_name: Some("Bob".to_string()),
            uri: Uri::from_str("sip:bob@example.com").unwrap(),
            params: vec![Param::tag("".to_string())],
        };
        println!("DEBUG: To header with empty tag params: {:?}", to_addr_empty_tag.params);
        let to_empty_tag = To::new(to_addr_empty_tag);
        println!("DEBUG: To header with empty tag (from To): {:?}", to_empty_tag.tag());
        
        // Create a completely fresh request with the empty-tag To header
        let mut empty_tag_request = Request::new(Method::Update, Uri::from_str("sip:bob@example.com").unwrap());
        empty_tag_request = empty_tag_request
            .with_header(TypedHeader::CallId(call_id.clone()))
            .with_header(TypedHeader::From(from.clone()))
            .with_header(TypedHeader::To(to_empty_tag.clone()))
            .with_header(TypedHeader::CSeq(cseq.clone()));
        
        // Verify the request has a To header with an empty tag
        println!("DEBUG: Empty tag request To header: {:?}", empty_tag_request.to());
        if let Some(empty_tag_to) = empty_tag_request.to() {
            println!("DEBUG: Empty tag request To tag: {:?}", empty_tag_to.tag());
            assert_eq!(empty_tag_to.tag(), Some(""), "Should have empty tag");
        }
        
        // Validate should fail for empty tag
        let result2 = validate_update_request(&empty_tag_request);
        println!("DEBUG: Validation result for empty tag: {:?}", result2);
        assert!(result2.is_err(), "UPDATE request with empty To tag should fail validation");
        
        // Test without Contact - should issue a warning but still validate
        let mut request_no_contact = request.clone();
        request_no_contact.headers.retain(|h| !matches!(h, TypedHeader::Contact(_)));
        assert!(validate_update_request(&request_no_contact).is_ok(),
                "Missing Contact should issue a warning but pass validation according to RFC 3311");
    }
} 