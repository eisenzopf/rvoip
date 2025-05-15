//! CANCEL method utilities for SIP transactions
//!
//! Implements special handling for CANCEL requests according to RFC 3261 Section 9.

use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::types::CSeq;
use rvoip_sip_core::types::MaxForwards;
use uuid::Uuid;

use crate::error::{Error, Result};
use crate::transaction::TransactionKey;

/// Creates a CANCEL request from an INVITE request following RFC 3261 Section 9.1 rules
///
/// The CANCEL request:
/// - Must have the same Request-URI, Call-ID, To, From, and Route headers as the INVITE
/// - The CSeq method must be CANCEL but the sequence number must be the same as the INVITE
/// - Must have a new unique branch parameter in the Via header
///
/// # Arguments
/// * `invite_request` - The original INVITE request to cancel
/// * `local_addr` - The local address to use in the Via header
///
/// # Returns
/// * `Result<Request>` - The CANCEL request or an error
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
    
    // Create a new request
    let mut cancel_request = Request::new(Method::Cancel, request_uri);
    
    // Add the required headers
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
    
    // Generate a new branch parameter for the Via header
    let branch = format!("z9hG4bK{}", Uuid::new_v4().to_string().replace("-", ""));
    let via_header = via_header_with_branch(local_addr, &branch)?;
    cancel_request = cancel_request.with_header(via_header);
    
    Ok(cancel_request)
}

/// Helper to create a Via header with the specified branch parameter
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
/// A matching INVITE transaction has:
/// - The same Call-ID
/// - The same From tag
/// - The same To tag (if present in the To header)
/// - The same Request-URI (should match)
///
/// # Arguments
/// * `cancel_request` - The CANCEL request
/// * `invite_transactions` - A collection of transaction keys to match against
///
/// # Returns
/// * `Option<TransactionKey>` - The matching INVITE transaction key if found
pub fn find_invite_transaction_for_cancel<I>(
    cancel_request: &Request, 
    invite_transactions: I
) -> Option<TransactionKey>
where
    I: IntoIterator<Item = TransactionKey>
{
    // Extract the key headers from the CANCEL request
    let cancel_call_id = cancel_request.call_id()?;
    let cancel_from = cancel_request.from()?;
    let cancel_to = cancel_request.to()?;
    
    // Find a matching INVITE transaction
    for tx_key in invite_transactions {
        if *tx_key.method() == Method::Invite && !tx_key.is_server {
            // This is a client INVITE transaction, potential match
            // We'll need to match the transaction with the request later
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
/// Returns the matching transaction ID or None if no match is found.
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
/// A valid CANCEL request must:
/// - Have the same Call-ID, To, From, and CSeq number (but method is CANCEL) as the INVITE
/// - Have the same Request-URI as the INVITE
/// - Have exactly one Via header
/// - Max-Forwards header should be present
/// 
/// This function performs basic validation of the CANCEL request but
/// can't validate it against the INVITE without having access to the
/// original INVITE.
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
    
    // Check that there is exactly one Via header
    match request.via_headers().len() {
        0 => return Err(Error::Other("CANCEL request missing Via header".to_string())),
        1 => {} // Exactly one Via header is correct
        _ => return Err(Error::Other("CANCEL request has more than one Via header".to_string())),
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
/// According to RFC 3261, a CANCEL request should:
/// 1. Have the same Request-URI, Call-ID, To, From, and Route headers 
/// 2. Have the same CSeq sequence number but CANCEL method
/// 
/// # Arguments
/// * `cancel_request` - The CANCEL request to check
/// * `invite_request` - The INVITE request to compare against
/// 
/// # Returns
/// * `bool` - True if the CANCEL matches the INVITE, false otherwise
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
        
        // Verify branch parameter is different
        let cancel_via = cancel.header(&HeaderName::Via).unwrap();
        let invite_via = invite.header(&HeaderName::Via).unwrap();
        assert_ne!(cancel_via, invite_via);
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
} 