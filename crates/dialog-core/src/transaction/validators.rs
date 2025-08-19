/// # Transaction Validators
///
/// This module provides validation functions that ensure SIP messages conform to the
/// transaction matching rules defined in RFC 3261. These validators help enforce the
/// correct association between requests, responses, and transactions.
///
/// ## RFC 3261 Context
///
/// RFC 3261 Section 17.1.3 defines strict rules for matching responses to client transactions:
///
/// > When the transport layer receives a response, it has to determine which client
/// > transaction will handle the response, so that the processing of Sections 17.1.1
/// > and 17.1.2 can take place. The branch parameter in the top Via header field
/// > is used for this purpose. A response matches a client transaction under two
/// > conditions:
/// >
/// > 1. If the response has the same value of the branch parameter in the top Via
/// >    header field as the branch parameter in the top Via header field of the
/// >    request that created the transaction.
/// >
/// > 2. If the method parameter in the CSeq header field matches the method of
/// >    the request that created the transaction. The method is needed since a
/// >    CANCEL request constitutes a different transaction, but shares the same
/// >    value of the branch parameter.
///
/// This module implements these rules to ensure proper transaction matching.

use std::sync::Arc;
use tracing::{trace, warn};

use rvoip_sip_core::prelude::*;

use crate::transaction::error::{Error, Result};
use crate::transaction::TransactionKey;

/// Validate that a response matches a transaction by checking Via and CSeq headers
/// 
/// This function implements the transaction matching rules from RFC 3261 Section 17.1.3
/// to ensure that a response is correctly associated with its client transaction.
/// 
/// # RFC 3261 Context
/// 
/// Per RFC 3261 Section 17.1.3, a response matches a client transaction if:
/// 1. The branch parameter in the top Via header matches the transaction's branch
/// 2. The method in the CSeq header matches the transaction's original request method
/// 
/// These checks prevent responses from being delivered to the wrong transaction,
/// which is particularly important for distinguishing between CANCEL and the
/// request being canceled, as they share the same branch parameter.
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
/// Client transactions should only process response messages. This function
/// verifies the message type and extracts the response if valid.
/// 
/// # RFC 3261 Context
/// 
/// RFC 3261 Section 17.1 defines client transactions as handling responses
/// to requests. If a client transaction receives a request instead of a response,
/// it's an error condition that should be handled gracefully.
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
/// This method is used to extract the SIP method from the original request
/// that created the transaction, which is needed for response matching.
/// 
/// # RFC 3261 Context
/// 
/// RFC 3261 Section 17.1.3 requires that the method in a response's CSeq header
/// matches the method of the request that created the transaction. This function
/// helps retrieve that original method.
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
/// This function categorizes a SIP response based on its status code, which
/// is essential for determining the appropriate transaction state transitions.
/// 
/// # RFC 3261 Context
/// 
/// RFC 3261 defines three categories of responses that affect transaction state:
/// - Provisional (1xx): Indicate request processing is continuing
/// - Success (2xx): Indicate the request was successful
/// - Failure (3xx-6xx): Indicate the request failed or was redirected
///
/// Different response categories trigger different state transitions in the
/// transaction state machines defined in RFC 3261 Sections 17.1.1.2, 17.1.2.2,
/// 17.2.1, and 17.2.2.
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