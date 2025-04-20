use rand::{thread_rng, Rng};
use rvoip_sip_core::{Header, HeaderName, Message, Method, Request, Response, StatusCode};
use std::str::FromStr;
use tracing::debug;

use crate::error::{Error, Result};

/// Generate a random branch parameter for Via header
pub fn generate_branch() -> String {
    let mut rng = thread_rng();
    let random: u64 = rng.gen();
    format!("z9hG4bK-{:x}", random)
}

/// Extract the branch parameter from a message
pub fn extract_branch(message: &Message) -> Option<String> {
    message
        .first_via()
        .and_then(|via| via.get("branch").flatten().map(|s| s.to_string()))
}

/// Extract the Call-ID from a message
pub fn extract_call_id(message: &Message) -> Option<String> {
    message
        .header(&HeaderName::CallId)
        .map(|h| h.value.to_string_value())
}

/// Extract the CSeq from a message
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    message
        .header(&HeaderName::CSeq)
        .and_then(|h| {
            let value = h.value.to_string_value();
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.len() != 2 {
                return None;
            }
            
            let seq_num = parts[0].parse::<u32>().ok()?;
            let method = Method::from_str(parts[1]).ok()?;
            Some((seq_num, method))
        })
}

/// Create a general response to a request
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut response = Response::new(status);
    
    // Copy headers from request that should be in the response
    for header in &request.headers {
        if matches!(header.name, 
            HeaderName::Via | 
            HeaderName::From | 
            HeaderName::To | 
            HeaderName::CallId | 
            HeaderName::CSeq
        ) {
            response = response.with_header(header.clone());
        }
    }
    
    // Add Content-Length header (empty body)
    response = response.with_header(Header::integer(HeaderName::ContentLength, 0));
    
    response
}

/// Create a TRYING (100) response for an INVITE request
pub fn create_trying_response(request: &Request) -> Response {
    create_response(request, StatusCode::Trying)
}

/// Create a RINGING (180) response for an INVITE request
pub fn create_ringing_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ringing)
}

/// Create an OK (200) response for a request
pub fn create_ok_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ok)
}

/// Extract the transaction ID from a message
pub fn extract_transaction_id(message: &Message) -> Result<String> {
    let branch = extract_branch(message)
        .ok_or_else(|| Error::Other("Missing branch parameter in Via header".to_string()))?;
    
    // Determine transaction type prefix based on message type
    match message {
        Message::Request(request) => {
            if request.method == Method::Invite {
                Ok(format!("ict_{}", branch)) // Invite Client Transaction
            } else {
                Ok(format!("nict_{}", branch)) // Non-Invite Client Transaction
            }
        },
        Message::Response(_response) => {
            if let Some((_, method)) = extract_cseq(message) {
                if method == Method::Invite {
                    Ok(format!("ist_{}", branch)) // Invite Server Transaction
                } else {
                    Ok(format!("nist_{}", branch)) // Non-Invite Server Transaction
                }
            } else {
                Err(Error::Other("Missing or invalid CSeq header".to_string()))
            }
        }
    }
}

/// Extract a transaction ID from a response
/// This must match how transaction IDs are generated in client transactions
pub fn extract_transaction_id_from_response(response: &rvoip_sip_core::Response) -> Option<String> {
    // Use the structured Via implementation
    response.first_via()
        .and_then(|via| via.get("branch").map(|branch| 
            // ict_ prefix for INVITE client transactions
            format!("ict_{}", branch)
        ))
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
pub fn extract_destination(transaction_id: &str) -> Option<std::net::SocketAddr> {
    debug!("WARNING: Using placeholder extract_destination for transaction {}", transaction_id);
    debug!("This is inefficient and should be replaced with TransactionManager.get_transaction_destination");
    
    // Hard-coded destination for testing - NOT FOR PRODUCTION USE
    // In a real application, this should be:
    // 1. Retrieved from the transaction registry
    // 2. Extracted from the transaction ID if encoded there
    // 3. Or determined from the SIP URI
    Some(std::net::SocketAddr::from(([127, 0, 0, 1], 5071)))
} 