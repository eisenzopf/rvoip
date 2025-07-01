//! # UPDATE Method Utilities for SIP Transactions
//!
//! This module implements special handling for UPDATE requests according to RFC 3311.
//! The UPDATE method allows a client to update parameters of a session (such as media streams
//! and their codecs) without changing the session state.
//!
//! ## UPDATE in the SIP Transaction Layer
//!
//! According to RFC 3311, UPDATE has several characteristics important for transaction handling:
//!
//! 1. **In-Dialog Only**: UPDATE is intended for in-dialog usage and must include a To tag
//! 2. **Session Modification**: Primarily used to modify session parameters via SDP
//! 3. **Pre-answer Usage**: Can be sent before a final response to an INVITE
//! 4. **Regular Transaction**: Uses standard Non-INVITE transaction state machines
//! 5. **Target Refresh**: May update dialog targets when Contact is included
//!
//! ## Relationship to Other SIP Methods
//!
//! UPDATE complements other SIP methods in important ways:
//!
//! - **INVITE**: Initial session establishment; requires UAS to alert the user
//! - **re-INVITE**: Mid-dialog session modification; requires UAS to alert the user
//! - **UPDATE**: Mid-dialog session modification without alerting the user
//! - **INFO**: Mid-dialog application information; does not modify session parameters
//!
//! ## Transaction Layer Responsibilities
//!
//! The transaction layer handles UPDATE as a Non-INVITE transaction, but must ensure:
//! - Proper in-dialog context (To tag must be present)
//! - Appropriate Contact headers for target refresh capability
//! - Standard Non-INVITE transaction state machine
//!
//! ## Diagram: UPDATE in a Dialog
//!
//! ```text
//! Client                                Server
//!   |                                     |
//!   |--------INVITE-------------------->  |
//!   |                                     |
//!   |<---------100 Trying----------------- |
//!   |                                     |
//!   |<---------180 Ringing---------------- |
//!   |                                     |
//!   |                                     |
//!   |--------UPDATE--------------------->  | ← Updates session before final answer
//!   |        (Non-INVITE Transaction)     |
//!   |                                     |
//!   |<---------200 OK for UPDATE---------- |
//!   |                                     |
//!   |<---------200 OK for INVITE---------- |
//!   |                                     |
//!   |--------ACK------------------------>  |
//!   |                                     |
//! ```
//!
//! This module provides utility functions for creating and validating UPDATE 
//! requests according to RFC 3311.

use std::net::SocketAddr;
use std::str::FromStr;
use rvoip_sip_core::prelude::*;
use crate::error::{Error, Result};

/// Creates an UPDATE request based on an existing dialog
///
/// According to RFC 3311, UPDATE is used to modify aspects of a session
/// without changing the dialog state. It is particularly useful for
/// updating session parameters (like SDP) before the session is established.
///
/// An UPDATE request:
/// - Must be sent within an existing dialog (To tag must be present)
/// - Should have a new CSeq number (higher than previous requests)
/// - Should contain a Contact header
/// - Typically includes SDP to modify session parameters
///
/// # RFC References
/// - RFC 3311 Section 5.1: UPDATE Method
/// - RFC 3311 Section 5.2: Headers
///
/// # Arguments
/// * `dialog_request` - A previous in-dialog request (like INVITE) to base the UPDATE on
/// * `local_addr` - Local address to use in the Via header
/// * `new_sdp` - Optional SDP to include in the UPDATE
///
/// # Returns
/// * `Result<Request>` - The UPDATE request or an error
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_transaction_core::method::update::create_update_request;
/// # use rvoip_transaction_core::error::Result;
/// #
/// # // Create a simple INVITE request with To tag (simulating an in-dialog request)
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", Some("bob-tag"))  // To tag makes this in-dialog
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
/// #     .build();
/// #
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// #
/// // Create an UPDATE request to modify the session
/// let new_sdp = "v=0\r\no=alice 2890844526 2890844527 IN IP4 127.0.0.1\r\ns=\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
/// let update = create_update_request(&invite, &local_addr, Some(new_sdp.to_string())).unwrap();
///
/// // The UPDATE should have the same Call-ID as the INVITE
/// assert_eq!(invite.call_id().unwrap(), update.call_id().unwrap());
///
/// // But an incremented CSeq number
/// assert_eq!(invite.cseq().unwrap().seq + 1, update.cseq().unwrap().seq);
/// ```
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
    
    // Check that this is an in-dialog request (To header must have a tag)
    if dialog_request.to().unwrap().tag().is_none() {
        return Err(Error::Other("Cannot create UPDATE - not an in-dialog request (To tag missing)".to_string()));
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
    
    // Create a Via header with a new branch parameter
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
    let host = local_addr.ip().to_string();
    let port = Some(local_addr.port());
    let params = vec![rvoip_sip_core::types::Param::branch(branch)];
    let via = rvoip_sip_core::types::via::Via::new(
        "SIP", "2.0", "UDP", &host, port, params
    )?;
    update = update.with_header(TypedHeader::Via(via));
    
    // Create a Contact header - recommended for UPDATE
    let contact_uri = format!("sip:{}:{}", local_addr.ip(), local_addr.port());
    let contact_addr = rvoip_sip_core::types::address::Address::new(
        rvoip_sip_core::types::uri::Uri::sip(format!("{}:{}", local_addr.ip(), local_addr.port()))
    );
    let contact_param = rvoip_sip_core::types::contact::ContactParamInfo { address: contact_addr };
    let contact = rvoip_sip_core::types::contact::Contact::new_params(vec![contact_param]);
    update = update.with_header(TypedHeader::Contact(contact));
    
    // Add Max-Forwards header
    update = update.with_header(TypedHeader::MaxForwards(rvoip_sip_core::types::max_forwards::MaxForwards::new(70)));
    
    // Add optional SDP body
    if let Some(sdp) = new_sdp {
        let content_type = ContentType::new(ContentTypeValue {
            m_type: "application".to_string(),
            m_subtype: "sdp".to_string(),
            parameters: std::collections::HashMap::new(),
        });
        update = update.with_header(TypedHeader::ContentType(content_type));
        
        // Set Content-Length header first (before converting to bytes consumes the sdp string)
        let content_length = rvoip_sip_core::types::content_length::ContentLength::new(sdp.len() as u32);
        update = update.with_header(TypedHeader::ContentLength(content_length));
        
        // Then add the body
        update = update.with_body(sdp.into_bytes());
    } else {
        // Set empty Content-Length
        let content_length = rvoip_sip_core::types::content_length::ContentLength::new(0);
        update = update.with_header(TypedHeader::ContentLength(content_length));
    }
    
    Ok(update)
}

/// Validates that an UPDATE request meets the requirements of RFC 3311
///
/// According to RFC 3311, an UPDATE request must:
/// - Be sent within a dialog (To tag must be present)
/// - Should contain a Contact header (warning issued but validation passes)
/// - Typically modify session parameters using SDP (not strictly required)
///
/// # RFC References
/// - RFC 3311 Section 5.1: UPDATE Method Definition
/// - RFC 3311 Section 5.2: Header Field Support
///
/// # Arguments
/// * `request` - The UPDATE request to validate
///
/// # Returns
/// * `Result<()>` - Ok if valid, Error if not
///
/// # Example
/// ```
/// # use std::str::FromStr;
/// # use rvoip_sip_core::prelude::*;
/// # use rvoip_transaction_core::method::update::validate_update_request;
/// #
/// # // Create a minimal valid UPDATE request
/// # let mut request = Request::new(Method::Update, Uri::from_str("sip:bob@example.com").unwrap());
/// # 
/// # // Add required headers
/// # let call_id = CallId::new("test-call-id");
/// # 
/// # // Create From header with tag
/// # let from_uri = Uri::from_str("sip:alice@example.com").unwrap();
/// # let from_addr = Address {
/// #     display_name: Some("Alice".to_string()),
/// #     uri: from_uri,
/// #     params: vec![Param::tag("alice-tag".to_string())],
/// # };
/// # let from = From::new(from_addr);
/// # 
/// # // Create To header with tag (required for in-dialog)
/// # let to_uri = Uri::from_str("sip:bob@example.com").unwrap();
/// # let to_addr = Address {
/// #     display_name: Some("Bob".to_string()),
/// #     uri: to_uri,
/// #     params: vec![Param::tag("bob-tag".to_string())],
/// # };
/// # let to = To::new(to_addr);
/// # 
/// # let cseq = CSeq::new(1, Method::Update);
/// # 
/// # request = request
/// #     .with_header(TypedHeader::CallId(call_id))
/// #     .with_header(TypedHeader::From(from))
/// #     .with_header(TypedHeader::To(to))
/// #     .with_header(TypedHeader::CSeq(cseq));
/// #
/// // Validate the UPDATE request
/// let result = validate_update_request(&request);
/// assert!(result.is_ok());
/// ```
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
    use std::net::SocketAddr;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
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
        assert!(validate_update_request(&request).is_ok());
        
        // Test with incorrect method - should fail validation
        let mut method_request = request.clone();
        method_request.method = Method::Info;
        assert!(validate_update_request(&method_request).is_err(),
                "Request with non-UPDATE method should fail validation");
        
        // Test without To tag
        
        // Create a To header with no tag at all
        let to_addr_no_tag = Address::new(Uri::from_str("sip:bob@example.com").unwrap());
        let to_no_tag = To::new(to_addr_no_tag);
        
        // Create a completely fresh request with the no-tag To header
        let mut no_tag_request = Request::new(Method::Update, Uri::from_str("sip:bob@example.com").unwrap());
        no_tag_request = no_tag_request
            .with_header(TypedHeader::CallId(call_id.clone()))
            .with_header(TypedHeader::From(from.clone()))
            .with_header(TypedHeader::To(to_no_tag.clone()))
            .with_header(TypedHeader::CSeq(cseq.clone()));
        
        // Validate should fail
        let result = validate_update_request(&no_tag_request);
        assert!(result.is_err(), "UPDATE request without To tag should fail validation");
        
        // Test with empty tag
        // Create a To header with an empty tag
        let to_addr_empty_tag = Address {
            display_name: Some("Bob".to_string()),
            uri: Uri::from_str("sip:bob@example.com").unwrap(),
            params: vec![Param::tag("".to_string())],
        };
        let to_empty_tag = To::new(to_addr_empty_tag);
        
        // Create a completely fresh request with the empty-tag To header
        let mut empty_tag_request = Request::new(Method::Update, Uri::from_str("sip:bob@example.com").unwrap());
        empty_tag_request = empty_tag_request
            .with_header(TypedHeader::CallId(call_id.clone()))
            .with_header(TypedHeader::From(from.clone()))
            .with_header(TypedHeader::To(to_empty_tag.clone()))
            .with_header(TypedHeader::CSeq(cseq.clone()));
        
        // Validate should fail for empty tag
        let result2 = validate_update_request(&empty_tag_request);
        assert!(result2.is_err(), "UPDATE request with empty To tag should fail validation");
        
        // Test without Contact - should issue a warning but still validate
        let mut request_no_contact = request.clone();
        request_no_contact.headers.retain(|h| !matches!(h, TypedHeader::Contact(_)));
        assert!(validate_update_request(&request_no_contact).is_ok(),
                "Missing Contact should issue a warning but pass validation according to RFC 3311");
    }
    
    #[test]
    fn test_create_update_request() {
        // Create an in-dialog INVITE request (with To tag)
        let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag"))  // To tag makes this in-dialog
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
            .max_forwards(70)
            .build();
            
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let sdp = "v=0\r\no=alice 2890844526 2890844527 IN IP4 127.0.0.1\r\ns=\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        
        // Create an UPDATE request
        let update = create_update_request(&invite, &local_addr, Some(sdp.to_string())).unwrap();
        
        // Verify it's an UPDATE request
        assert_eq!(update.method(), Method::Update);
        
        // Verify dialog identifiers are preserved
        assert_eq!(update.call_id().unwrap(), invite.call_id().unwrap());
        assert_eq!(update.from().unwrap().tag(), invite.from().unwrap().tag());
        assert_eq!(update.to().unwrap().tag(), invite.to().unwrap().tag());
        
        // Verify CSeq is incremented
        assert_eq!(update.cseq().unwrap().seq, invite.cseq().unwrap().seq + 1);
        assert_eq!(update.cseq().unwrap().method, Method::Update);
        
        // Verify Via has a new branch parameter
        assert!(update.first_via().is_some());
        assert!(update.first_via().unwrap().branch().is_some());
        
        // Verify Contact header is present
        let has_contact = update.headers.iter().any(|h| matches!(h, TypedHeader::Contact(_)));
        assert!(has_contact, "UPDATE should include a Contact header");
        
        // Verify SDP is included properly
        assert_eq!(update.body(), sdp.as_bytes());
        assert!(update.headers.iter().any(|h| matches!(h, TypedHeader::ContentType(_))));
        
        // Verify Content-Length matches SDP size
        if let Some(TypedHeader::ContentLength(content_length)) = update.header(&HeaderName::ContentLength) {
            assert_eq!(content_length.0 as usize, sdp.len());
        } else {
            panic!("UPDATE missing Content-Length header");
        }
    }
    
    #[test]
    fn test_create_update_without_sdp() {
        // Create an in-dialog INVITE
        let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag"))
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.branch"))
            .build();
            
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        // Create an UPDATE without SDP
        let update = create_update_request(&invite, &local_addr, None).unwrap();
        
        // Verify empty body
        assert!(update.body().is_empty());
        
        // Verify Content-Length: 0
        if let Some(TypedHeader::ContentLength(content_length)) = update.header(&HeaderName::ContentLength) {
            assert_eq!(content_length.0, 0);
        } else {
            panic!("UPDATE missing Content-Length header");
        }
    }
    
    #[test]
    fn test_create_update_invalid_input() {
        // Create an out-of-dialog INVITE (no To tag)
        let out_of_dialog_invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)  // No To tag
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.branch"))
            .build();
            
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        // Should fail because UPDATE requires an in-dialog request
        let result = create_update_request(&out_of_dialog_invite, &local_addr, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("To tag missing"));
    }
} 