//! Message extraction utilities for transaction-core
//! 
//! This module provides functions for extracting various pieces of information
//! from SIP messages such as branch parameters, Call-IDs, CSeq values, etc.

use rvoip_sip_core::prelude::*;

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
    tracing::debug!("WARNING: Using placeholder extract_destination. This function is deprecated and returns None.");
    None
    // Some(std::net::SocketAddr::from(([127, 0, 0, 1], 5071)))
} 