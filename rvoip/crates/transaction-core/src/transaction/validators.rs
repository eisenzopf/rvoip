use std::sync::Arc;
use tracing::{trace, warn};

use rvoip_sip_core::prelude::*;

use crate::error::{Error, Result};
use crate::transaction::TransactionKey;

/// Validate that a response matches a transaction by checking Via and CSeq headers
/// 
/// # Arguments
/// * `response` - The SIP response to validate
/// * `tx_id` - The transaction ID to match against
/// * `original_method` - The original method from the request that created this transaction
/// 
/// # Returns
/// * `Ok(())` if the response matches the transaction
/// * `Err(Error)` if validation fails
pub fn validate_response_matches_transaction(
    response: &Response,
    tx_id: &TransactionKey,
    original_method: &Method,
) -> Result<()> {
    // Check Via headers
    if let Some(TypedHeader::Via(via_header_vec)) = response.header(&HeaderName::Via) {
        if let Some(via_header) = via_header_vec.0.first() {
            if let Some(branch) = via_header.branch() {
                if branch != tx_id.branch.as_str() {
                    warn!(
                        id=%tx_id, 
                        received_branch=?via_header.branch(), 
                        expected_branch=%tx_id.branch, 
                        "Received response with mismatched Via branch"
                    );
                    return Err(Error::Other("Mismatched Via branch parameter".to_string()));
                }
            } else {
                warn!(id=%tx_id, "Received response Via without branch parameter");
                return Err(Error::Other("Missing Via branch parameter".to_string()));
            }
        } else {
            warn!(id=%tx_id, "Received response with empty Via header value");
            return Err(Error::Other("Empty Via header value".to_string()));
        }
    } else {
        warn!(id=%tx_id, "Received response without Via header");
        return Err(Error::Other("Missing Via header".to_string()));
    }

    // Check CSeq method matches
    if let Some(TypedHeader::CSeq(cseq_header)) = response.header(&HeaderName::CSeq) {
        if &cseq_header.method != original_method {
            warn!(
                id=%tx_id, 
                received_cseq_method=?cseq_header.method, 
                expected_method=?original_method, 
                "Received response with mismatched CSeq method"
            );
            return Err(Error::Other("Mismatched CSeq method".to_string()));
        }
    } else {
        warn!(id=%tx_id, "Received response without CSeq header");
        return Err(Error::Other("Missing CSeq header".to_string()));
    }

    // All checks passed
    trace!(id=%tx_id, "Response passed transaction validation checks");
    Ok(())
}

/// Check if a message is a valid response and extract it
/// 
/// # Arguments
/// * `message` - The SIP message to check
/// * `tx_id` - The transaction ID for logging
/// 
/// # Returns
/// * `Ok(Response)` if the message is a valid response
/// * `Err(Error)` if it's not a response
pub fn extract_response(message: &Message, tx_id: &TransactionKey) -> Result<Response> {
    match message {
        Message::Response(r) => Ok(r.clone()),
        Message::Request(_) => {
            warn!(id=%tx_id, "Client transaction received a Request, ignoring");
            Err(Error::Other("Client transaction received a Request".to_string()))
        }
    }
}

/// Get the original method from a request stored in a transaction
/// 
/// # Arguments
/// * `request` - The original SIP request
/// 
/// # Returns
/// * The Method from the request
pub fn get_method_from_request(request: &Request) -> Method {
    request.method().clone()
}

/// Extract the status type from a response (provisional, success, or failure)
/// 
/// # Arguments
/// * `response` - The SIP response
/// 
/// # Returns
/// * A tuple of (is_provisional, is_success, is_failure)
pub fn categorize_response_status(response: &Response) -> (bool, bool, bool) {
    let status = response.status();
    let is_provisional = status.is_provisional();
    let is_success = status.is_success();
    let is_failure = !is_provisional && !is_success;
    
    (is_provisional, is_success, is_failure)
} 