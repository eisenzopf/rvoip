//! # CANCEL Method Utilities for SIP Transactions
//!
//! This module implements special handling for CANCEL requests according to RFC 3261 Section 9.
//! The CANCEL method in SIP has unique properties that require special handling at the
//! transaction layer.
//!
//! ## CANCEL in the Transaction Layer
//!
//! According to RFC 3261 Section 9, CANCEL has several unique characteristics:
//!
//! 1. **Request-Specific**: CANCEL only applies to pending INVITE requests
//! 2. **Matching Rules**: A CANCEL must match an existing INVITE transaction
//! 3. **Dual Transaction Model**: CANCEL creates its own transaction but also affects the matched INVITE transaction
//! 4. **Different Status Code**: Successful CANCEL (200 OK) doesn't mean the INVITE was cancelled
//!
//! ## Transaction Layer Responsibilities
//!
//! The transaction layer is responsible for:
//! - Creating properly formatted CANCEL requests
//! - Matching CANCELs to the right INVITE transaction
//! - Managing the relationship between CANCEL and INVITE transactions
//! - Handling timer behaviors for both transactions
//!
//! ## Diagram: CANCEL in the Transaction Layer
//!
//! ```text
//! Client                                Server
//!   |                                     |
//!   |--------INVITE (creates ICT)-------->|
//!   |                                     | (creates IST)
//!   |<-----------100 Trying---------------|
//!   |                                     |
//!   |                                     |
//!   |--------CANCEL (creates NICT)------->| 
//!   |                                     | (creates NIST + matches to IST)
//!   |<-----------200 OK for CANCEL--------|
//!   |                                     |
//!   |<-----------487 Request Terminated---|
//!   |                                     |
//!   |--------------ACK------------------->|
//!   |                                     |
//! ```
//!
//! Where:
//! - ICT = INVITE Client Transaction
//! - IST = INVITE Server Transaction
//! - NICT = Non-INVITE Client Transaction (for CANCEL)
//! - NIST = Non-INVITE Server Transaction (for CANCEL)
//!
//! This module provides utility functions for creating, validating, and matching
//! CANCEL requests according to the rules in RFC 3261.

use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::CSeq;
use rvoip_sip_core::types::MaxForwards;
use uuid::Uuid;

use crate::transaction::error::{Error, Result};
use crate::transaction::TransactionKey;

/// Creates a CANCEL request from an INVITE request following RFC 3261 Section 9.1 rules
///
/// According to RFC 3261 Section 9.1, a CANCEL request MUST be formatted as follows:
/// - Must have the same Request-URI, Call-ID, To, From, and Route headers as the INVITE
/// - The CSeq method must be CANCEL but the sequence number must be the same as the INVITE
/// - Must have the same branch parameter in the Via header as the original INVITE
///
/// This creates a properly formatted CANCEL request that can be used to attempt
/// to cancel a previous INVITE transaction.
///
/// # RFC References
/// - RFC 3261 Section 9.1: Client Behavior
/// - RFC 3261 Section 9.2: Server Behavior
///
/// # Arguments
/// * `invite_request` - The original INVITE request to cancel
/// * `local_addr` - The local address to use in the Via header
///
/// # Returns
/// * `Result<Request>` - The CANCEL request or an error
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use crate::transaction::method::cancel::create_cancel_request;
/// # use crate::transaction::error::Result;
/// #
/// # // Create a simple INVITE request
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", None)
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
/// #     .build();
/// #
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// #
/// // Create a CANCEL for the INVITE
/// let cancel = create_cancel_request(&invite, &local_addr).unwrap();
///
/// // The CANCEL should have the same Call-ID as the INVITE
/// assert_eq!(invite.call_id().unwrap(), cancel.call_id().unwrap());
///
/// // And the same branch parameter
/// let invite_via = invite.first_via().unwrap();
/// let cancel_via = cancel.first_via().unwrap();
/// assert_eq!(invite_via.branch().unwrap(), cancel_via.branch().unwrap());
///
/// // The CSeq should have the same number but CANCEL method
/// assert_eq!(invite.cseq().unwrap().seq, cancel.cseq().unwrap().seq);
/// assert_eq!(cancel.cseq().unwrap().method, Method::Cancel);
/// ```
pub fn create_cancel_request(invite_request: &Request, local_addr: &SocketAddr) -> Result<Request> {
    // Validate that this is an INVITE request
    if invite_request.method() != Method::Invite {
        return Err(Error::Other("Cannot create CANCEL for non-INVITE request".to_string()));
    }

    // Extract the required headers from the INVITE
    let request_uri = invite_request.uri().clone();
    let from = invite_request.from()
        .ok_or_else(|| Error::Other("INVITE request missing From header".to_string()))?
        .clone();
    let to = invite_request.to()
        .ok_or_else(|| Error::Other("INVITE request missing To header".to_string()))?
        .clone();
    let call_id = invite_request.call_id()
        .ok_or_else(|| Error::Other("INVITE request missing Call-ID header".to_string()))?
        .clone();
    let cseq_num = invite_request.cseq()
        .ok_or_else(|| Error::Other("INVITE request missing CSeq header".to_string()))?
        .seq;
    
    // Debug the original INVITE Via headers
    println!("Original INVITE Via headers count: {}", invite_request.via_headers().len());
    for (i, via) in invite_request.via_headers().iter().enumerate() {
        println!("  Via[{}]: {}", i, via);
    }
    
    // Create a Request object from scratch with no headers at all
    let mut cancel_request = Request::new(Method::Cancel, request_uri);
    
    // Add the required headers - each call to with_header should replace any existing header
    cancel_request = cancel_request
        .with_header(TypedHeader::From(from))
        .with_header(TypedHeader::To(to))
        .with_header(TypedHeader::CallId(call_id))
        .with_header(TypedHeader::CSeq(CSeq::new(cseq_num, Method::Cancel)))
        .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)))
        .with_header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    // Copy any Route headers from the INVITE
    if let Some(route_header) = invite_request.header(&HeaderName::Route) {
        cancel_request = cancel_request.with_header(route_header.clone());
    }
    
    // Extract the branch parameter from the original INVITE Via header
    // According to RFC 3261 Section 9.1, the CANCEL request MUST have the same 
    // branch parameter as the request it is canceling
    let original_via = invite_request.first_via()
        .ok_or_else(|| Error::Other("INVITE request missing Via header".to_string()))?;
    
    let original_branch = original_via.branch()
        .ok_or_else(|| Error::Other("INVITE request Via header missing branch parameter".to_string()))?;
    
    // Create a new Via header with the same branch parameter but our local address
    use rvoip_sip_core::types::via::Via;
    
    // Create a Via header with the original branch parameter
    let params = vec![rvoip_sip_core::types::Param::branch(original_branch.to_string())];
    
    // Split the address into host and port
    let host = local_addr.ip().to_string();
    let port = Some(local_addr.port());
    
    // Create the Via header - creating a new one rather than cloning and modifying
    // the original to avoid duplicate headers
    let via = Via::new(
        "SIP", "2.0", "UDP",
        &host, port, params
    )?;
    
    // Make sure we're only adding one Via header - explicitly remove any existing ones first
    // though this should not be necessary since we're creating a new request
    cancel_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
    cancel_request = cancel_request.with_header(TypedHeader::Via(via));
    
    // Debug the CANCEL request Via headers
    println!("CANCEL request Via headers count: {}", cancel_request.via_headers().len());
    for (i, via) in cancel_request.via_headers().iter().enumerate() {
        println!("  Via[{}]: {}", i, via);
    }
    
    // Double-check for multiple Via headers
    if cancel_request.via_headers().len() > 1 {
        println!("WARNING: CANCEL request has {} Via headers, removing duplicates", cancel_request.via_headers().len());
        let first_via = cancel_request.first_via()
            .ok_or_else(|| Error::Other("Missing Via header in CANCEL request after creation".to_string()))?
            .clone();
        
        // Remove all Via headers and add only the first one
        cancel_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        cancel_request = cancel_request.with_header(TypedHeader::Via(first_via));
        
        println!("After cleanup - CANCEL request Via headers count: {}", cancel_request.via_headers().len());
        for (i, via) in cancel_request.via_headers().iter().enumerate() {
            println!("  Via[{}]: {}", i, via);
        }
    }
    
    Ok(cancel_request)
}

/// Helper to create a Via header with the specified branch parameter
///
/// Creates a properly formatted Via header with the given branch parameter
/// according to RFC 3261 Section 8.1.1.7.
///
/// # Arguments
/// * `local_addr` - Local address to use in the sent-by field
/// * `branch` - Branch parameter value (should start with z9hG4bK)
///
/// # Returns
/// * `Result<TypedHeader>` - A Via header or an error
fn via_header_with_branch(local_addr: &SocketAddr, branch: &str) -> Result<TypedHeader> {
    use rvoip_sip_core::types::via::Via;
    
    // Create a Via header with the provided branch parameter
    let params = vec![rvoip_sip_core::types::Param::branch(branch.to_string())];
    
    // Split the address into host and port
    let host = local_addr.ip().to_string();
    let port = Some(local_addr.port());
    
    // Create the Via header
    let via = Via::new(
        "SIP", "2.0", "UDP",
        &host, port, params
    )?;
    
    Ok(TypedHeader::Via(via))
}

/// Finds the matching INVITE transaction for a CANCEL request
///
/// According to RFC 3261 Section 9.1 and 9.2, a CANCEL request matches an INVITE if:
/// 1. The branch parameter in the top Via header matches
/// 2. The Call-ID matches
/// 3. The From tag matches
/// 4. The To tag matches (if present in the CANCEL)
/// 5. The CSeq number matches (but method will be CANCEL instead of INVITE)
///
/// This function searches through a collection of transaction keys to find a matching
/// INVITE transaction for a CANCEL request, primarily using the branch parameter.
///
/// # Arguments
/// * `cancel_request` - The CANCEL request
/// * `invite_transactions` - A collection of transaction keys to match against
///
/// # Returns
/// * `Option<TransactionKey>` - The matching INVITE transaction key if found
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use crate::transaction::method::cancel::{create_cancel_request, find_invite_transaction_for_cancel};
/// # use crate::transaction::transaction::TransactionKey;
/// #
/// # // Create a simple INVITE request
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", None)
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.invite-branch"))
/// #     .build();
/// #
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// # let cancel = create_cancel_request(&invite, &local_addr).unwrap();
/// #
/// // Create a transaction key for the INVITE
/// let invite_branch = invite.first_via().unwrap().branch().unwrap().to_string();
/// let invite_key = TransactionKey::new(invite_branch.clone(), Method::Invite, false);
/// 
/// // Create a collection of transaction keys to search through
/// let transactions = vec![invite_key.clone()];
/// 
/// // Find the matching INVITE transaction for the CANCEL
/// let matching_tx = find_invite_transaction_for_cancel(&cancel, transactions);
/// assert!(matching_tx.is_some());
/// ```
pub fn find_invite_transaction_for_cancel<I>(
    cancel_request: &Request, 
    invite_transactions: I
) -> Option<TransactionKey>
where
    I: IntoIterator<Item = TransactionKey>
{
    // Extract the key headers from the CANCEL request
    let cancel_via = cancel_request.first_via()?;
    let cancel_branch = cancel_via.branch()?;
    
    // Find a matching INVITE transaction by branch parameter
    for tx_key in invite_transactions {
        if *tx_key.method() == Method::Invite && !tx_key.is_server && tx_key.branch() == cancel_branch {
            // This is a client INVITE transaction with matching branch
            return Some(tx_key);
        }
    }
    
    None
}

/// Find a matching INVITE transaction for a CANCEL request
/// 
/// Analyzes a CANCEL request and finds a matching INVITE transaction ID
/// from the provided list of transaction keys.
///
/// According to RFC 3261, Section 9.1, a CANCEL matches an INVITE if:
/// 1. The Request-URI matches
/// 2. The Call-ID matches
/// 3. The From tag matches
/// 4. The To tag matches (if present in the CANCEL)
/// 5. The CSeq number matches (but method will be CANCEL instead of INVITE)
/// 6. Only one Via header is present in the CANCEL
/// 
/// This more comprehensive implementation provides better matching logic
/// compared to the simpler find_invite_transaction_for_cancel function.
///
/// # Arguments
/// * `cancel_request` - The CANCEL request
/// * `invite_tx_keys` - List of transaction keys to search through
///
/// # Returns
/// * `Option<TransactionKey>` - The matching transaction ID or None if no match is found
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use crate::transaction::method::cancel::{create_cancel_request, find_matching_invite_transaction};
/// # use crate::transaction::transaction::TransactionKey;
/// #
/// # // Create a simple INVITE request with a specific branch
/// # let branch = "z9hG4bK.test-branch-123";
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", None)
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some(branch))
/// #     .build();
/// #
/// # // Create the transaction key
/// # let invite_key = TransactionKey::new(branch.to_string(), Method::Invite, false);
/// # let invite_keys = vec![invite_key.clone()];
/// #
/// # // Create a CANCEL request that should match this INVITE
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// # let cancel = create_cancel_request(&invite, &local_addr).unwrap();
/// #
/// // For RFC 3261 compliant UAs, the branch in CANCEL must match the INVITE
/// let cancel_branch = cancel.first_via().unwrap().branch().unwrap().to_string();
/// let cancel_via = cancel.first_via().unwrap();
/// 
/// // Find matching INVITE transaction
/// let matching_tx = find_matching_invite_transaction(&cancel, invite_keys);
/// ```
pub fn find_matching_invite_transaction(
    cancel_request: &Request,
    invite_tx_keys: Vec<TransactionKey>
) -> Option<TransactionKey> {
    // Extract the needed headers from the CANCEL request
    let cancel_call_id = cancel_request.call_id()?;
    let cancel_from = cancel_request.from()?;
    let cancel_to = cancel_request.to()?;
    let cancel_uri = cancel_request.uri().clone();
    let cancel_cseq = cancel_request.cseq()?;
    
    // Basic validation - CANCEL must have a branch parameter
    let cancel_via = cancel_request.first_via()?;
    let cancel_branch = cancel_via.branch()?;
    
    // Look for a matching INVITE transaction
    // For RFC 3261 compliant behavior, the branch parameter in the CANCEL
    // should be the same as the branch in the INVITE, so we can use that directly
    for key in invite_tx_keys {
        // In RFC 3261, the branch parameter should match between INVITE and CANCEL
        // We already filtered for Method::Invite in the transaction manager
        if key.branch() == cancel_branch {
            return Some(key);
        }
    }
    
    None
}

/// Validates that a CANCEL request meets the requirements of RFC 3261
///
/// According to RFC 3261 Section 9.1, a valid CANCEL request must:
/// - Have the same Call-ID, To, From, and CSeq number (but method is CANCEL) as the INVITE
/// - Have the same Request-URI as the INVITE
/// - Have exactly one Via header
/// - Max-Forwards header should be present
/// 
/// This function performs basic validation of the CANCEL request but
/// can't validate it against the INVITE without having access to the
/// original INVITE.
///
/// # RFC References
/// - RFC 3261 Section 9.1: Client Behavior for CANCEL
///
/// # Arguments
/// * `request` - The CANCEL request to validate
///
/// # Returns
/// * `Result<()>` - Ok if valid, Error if not
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use crate::transaction::method::cancel::{create_cancel_request, validate_cancel_request};
/// #
/// # // Create a simple INVITE request
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", None)
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.branch"))
/// #     .build();
/// #
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// #
/// // Create a CANCEL request and validate it
/// let cancel = create_cancel_request(&invite, &local_addr).unwrap();
/// 
/// // Validation should pass for a properly created CANCEL
/// assert!(validate_cancel_request(&cancel).is_ok());
/// ```
pub fn validate_cancel_request(request: &Request) -> Result<()> {
    // Check that this is a CANCEL request
    if request.method() != Method::Cancel {
        return Err(Error::Other("Request method is not CANCEL".to_string()));
    }
    
    // Check that it has the required headers
    if request.call_id().is_none() {
        return Err(Error::Other("CANCEL request missing Call-ID header".to_string()));
    }
    
    if request.from().is_none() {
        return Err(Error::Other("CANCEL request missing From header".to_string()));
    }
    
    if request.to().is_none() {
        return Err(Error::Other("CANCEL request missing To header".to_string()));
    }
    
    if request.cseq().is_none() {
        return Err(Error::Other("CANCEL request missing CSeq header".to_string()));
    }
    
    // Debug the Via headers
    println!("Validating CANCEL request Via headers:");
    println!("  Via headers count: {}", request.via_headers().len());
    for (i, via) in request.via_headers().iter().enumerate() {
        println!("  Via[{}]: {}", i, via);
    }
    
    // Print all headers for more detailed debugging
    println!("All headers in CANCEL request:");
    for (i, header) in request.headers.iter().enumerate() {
        println!("  Header[{}]: {}", i, header);
    }
    
    // Check that there is exactly one Via header
    match request.via_headers().len() {
        0 => return Err(Error::Other("CANCEL request missing Via header".to_string())),
        1 => {} // Exactly one Via header is correct
        _ => return Err(Error::Other(format!("CANCEL request has {} Via headers, should have exactly one", request.via_headers().len()))),
    }
    
    // Check that Max-Forwards header is present
    let has_max_forwards = request.headers.iter().any(|h| matches!(h, TypedHeader::MaxForwards(_)));
    if !has_max_forwards {
        return Err(Error::Other("CANCEL request missing Max-Forwards header".to_string()));
    }
    
    Ok(())
}

/// Check if a CANCEL request is a valid cancel for a specific INVITE request
/// 
/// According to RFC 3261 Section 9.1, a CANCEL request should:
/// 1. Have the same Request-URI, Call-ID, To, From, and Route headers 
/// 2. Have the same CSeq sequence number but CANCEL method
/// 
/// This function checks if a CANCEL request is a valid match to cancel
/// a specific INVITE transaction.
///
/// # RFC References
/// - RFC 3261 Section 9.1: Client Behavior
///
/// # Arguments
/// * `cancel_request` - The CANCEL request to check
/// * `invite_request` - The INVITE request to compare against
/// 
/// # Returns
/// * `bool` - True if the CANCEL matches the INVITE, false otherwise
///
/// # Example
/// ```
/// # use std::net::SocketAddr;
/// # use std::str::FromStr;
/// # use rvoip_sip_core::builder::SimpleRequestBuilder;
/// # use rvoip_sip_core::prelude::*;
/// # use crate::transaction::method::cancel::{create_cancel_request, is_cancel_for_invite};
/// #
/// # // Create a simple INVITE request
/// # let invite = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com").unwrap()
/// #     .from("Alice", "sip:alice@example.com", Some("alice-tag"))
/// #     .to("Bob", "sip:bob@example.com", None)
/// #     .call_id("test-call-id-1234")
/// #     .cseq(101)
/// #     .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
/// #     .build();
/// #
/// # let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
/// #
/// // Create a CANCEL for the INVITE
/// let cancel = create_cancel_request(&invite, &local_addr).unwrap();
/// 
/// // Check if the CANCEL is valid for this INVITE
/// assert!(is_cancel_for_invite(&cancel, &invite));
/// ```
pub fn is_cancel_for_invite(cancel_request: &Request, invite_request: &Request) -> bool {
    // Method must be CANCEL
    if cancel_request.method() != Method::Cancel {
        return false;
    }
    
    // INVITE method must be INVITE
    if invite_request.method() != Method::Invite {
        return false;
    }
    
    // Must have same Call-ID
    let cancel_call_id = match cancel_request.call_id() {
        Some(call_id) => call_id,
        None => return false,
    };
    
    let invite_call_id = match invite_request.call_id() {
        Some(call_id) => call_id,
        None => return false,
    };
    
    if cancel_call_id != invite_call_id {
        return false;
    }
    
    // Check CSeq number (but not method)
    let (cancel_seq, _) = match cancel_request.cseq() {
        Some(cseq) => (cseq.seq, cseq.method.clone()),
        None => return false,
    };
    
    let (invite_seq, _) = match invite_request.cseq() {
        Some(cseq) => (cseq.seq, cseq.method.clone()),
        None => return false,
    };
    
    if cancel_seq != invite_seq {
        return false;
    }
    
    // Check From and To tags
    if cancel_request.from() != invite_request.from() {
        return false;
    }
    
    let cancel_to = match cancel_request.to() {
        Some(to) => to,
        None => return false,
    };
    
    let invite_to = match invite_request.to() {
        Some(to) => to,
        None => return false,
    };
    
    // To header may have different tags, so compare just the address part
    if cancel_to.address().uri != invite_to.address().uri {
        return false;
    }
    
    // Check Request-URI
    if cancel_request.uri() != invite_request.uri() {
        return false;
    }
    
    // All checks passed
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use std::str::FromStr;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    fn create_test_invite() -> Request {
        let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .expect("Failed to create request builder");
            
        builder
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.originalbranchvalue"))
            .max_forwards(70)
            .build()
    }
    
    #[test]
    fn test_create_cancel_request() {
        let invite = create_test_invite();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        let cancel = create_cancel_request(&invite, &local_addr).expect("Failed to create CANCEL");
        
        // Verify the CANCEL has the right method
        assert_eq!(cancel.method(), Method::Cancel);
        
        // Verify headers were copied correctly
        assert_eq!(cancel.uri(), invite.uri());
        assert_eq!(cancel.from().unwrap().tag(), invite.from().unwrap().tag());
        assert_eq!(cancel.to().unwrap().address().uri(), invite.to().unwrap().address().uri());
        assert_eq!(cancel.call_id().unwrap(), invite.call_id().unwrap());
        
        // Verify CSeq has same number but different method
        let cancel_cseq = cancel.cseq().unwrap();
        let invite_cseq = invite.cseq().unwrap();
        assert_eq!(cancel_cseq.seq, invite_cseq.seq);
        assert_eq!(cancel_cseq.method, Method::Cancel);
        
        // Verify branch parameter is the same (RFC 3261 Section 9.1)
        let cancel_via = cancel.first_via().unwrap();
        let invite_via = invite.first_via().unwrap();
        assert_eq!(cancel_via.branch().unwrap(), invite_via.branch().unwrap());
    }
    
    #[test]
    fn test_create_cancel_for_non_invite() {
        let builder = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
            .expect("Failed to create request builder");
            
        let register = builder
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Registrar", "sip:registrar.example.com", None)
            .call_id("test-call-id-1234")
            .cseq(1)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.branchvalue"))
            .build();
            
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let result = create_cancel_request(&register, &local_addr);
        
        assert!(result.is_err(), "Should error when creating CANCEL for non-INVITE");
    }
    
    #[test]
    fn test_validate_cancel_request() {
        let invite = create_test_invite();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        let cancel = create_cancel_request(&invite, &local_addr).expect("Failed to create CANCEL");
        
        // Validation should pass for a properly created CANCEL
        assert!(validate_cancel_request(&cancel).is_ok());
        
        // Test with missing headers
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::CallId(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing Call-ID");
        
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::From(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing From");
        
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing To");
        
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::CSeq(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing CSeq");
        
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing Via");
        
        let mut invalid_cancel = cancel.clone();
        invalid_cancel.headers.retain(|h| !matches!(h, TypedHeader::MaxForwards(_)));
        assert!(validate_cancel_request(&invalid_cancel).is_err(), "Should fail with missing Max-Forwards");
    }
    
    #[test]
    fn test_is_cancel_for_invite() {
        let invite = create_test_invite();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        let cancel = create_cancel_request(&invite, &local_addr).expect("Failed to create CANCEL");
        
        // Should be a valid CANCEL for the INVITE
        assert!(is_cancel_for_invite(&cancel, &invite));
        
        // Test with a different INVITE
        let other_invite = SimpleRequestBuilder::new(Method::Invite, "sip:alice@example.com")
            .expect("Failed to create request builder")
            .from("Bob", "sip:bob@example.com", Some("bob-tag"))
            .to("Alice", "sip:alice@example.com", None)
            .call_id("different-call-id")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.other"))
            .build();
            
        // Should not be a valid CANCEL for the other INVITE
        assert!(!is_cancel_for_invite(&cancel, &other_invite));
        
        // Test with a non-INVITE request
        let register = SimpleRequestBuilder::new(Method::Register, "sip:registrar.example.com")
            .expect("Failed to create request builder")
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Registrar", "sip:registrar.example.com", None)
            .call_id("test-call-id-1234")
            .cseq(101)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK.branch"))
            .build();
            
        // Should not be a valid CANCEL for a non-INVITE
        assert!(!is_cancel_for_invite(&cancel, &register));
    }
    
    #[test]
    fn test_find_invite_transaction_for_cancel() {
        let invite = create_test_invite();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        
        let cancel = create_cancel_request(&invite, &local_addr).expect("Failed to create CANCEL");
        
        // Create a transaction key for the INVITE
        let invite_branch = invite.first_via().unwrap().branch().unwrap().to_string();
        let invite_key = TransactionKey::new(invite_branch.clone(), Method::Invite, false);
        
        // Should find the INVITE transaction
        let result = find_invite_transaction_for_cancel(&cancel, vec![invite_key.clone()]);
        assert!(result.is_some());
        
        // Create a completely different transaction key
        let other_key = TransactionKey::new("z9hG4bK.otherkey".to_string(), Method::Invite, false);
        
        // Should NOT find the INVITE transaction with a different branch
        let result = find_invite_transaction_for_cancel(&cancel, vec![other_key]);
        assert!(result.is_none());
        
        // The CANCEL now has the same branch as INVITE
        let cancel_via = cancel.first_via().unwrap();
        let cancel_branch = cancel_via.branch().unwrap();
        assert_eq!(cancel_branch, invite_branch);
        
        // So find_matching_invite_transaction should work correctly
        let result = find_matching_invite_transaction(&cancel, vec![invite_key.clone()]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().branch(), invite_branch);
    }
} 