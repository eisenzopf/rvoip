use rand::{thread_rng, Rng};
// Use the prelude for easier access to common types
use rvoip_sip_core::prelude::*;
use std::str::FromStr;
use tracing::debug;

use crate::error::{self, Error, Result};
use crate::transaction::TransactionKey; // Import TransactionKey
use crate::transaction::TransactionKind; // Import Kind

/// Generate a random branch parameter for Via header (RFC 3261 magic cookie + random string)
pub fn generate_branch() -> String {
    let mut rng = thread_rng();
    // Generate a secure random string using UUID v4 for better uniqueness
    let random_suffix = uuid::Uuid::new_v4().simple().to_string();
    format!("z9hG4bK-{}", random_suffix)
}

/// Extract the branch parameter from the first Via header of a message
pub fn extract_branch(message: &Message) -> Option<String> {
    message
        .first_via() // Use the dedicated helper method
        .and_then(|via| via.branch().map(|s| s.to_string())) // Access the branch parameter directly
}

/// Extract the Call-ID value from a message
pub fn extract_call_id(message: &Message) -> Option<String> {
    message
        .call_id() // Use the dedicated helper method
        .map(|call_id| call_id.to_string()) // CallId implements Display
}

/// Extract the CSeq sequence number and method from a message
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    message
        .cseq() // Use the dedicated helper method
        .map(|cseq| (cseq.sequence(), cseq.method().clone())) // Access sequence and method directly
}

/// Create a general response to a request, copying essential headers
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut builder = ResponseBuilder::new(status);

    if let Some(via) = request.first_via() {
        builder = builder.header(TypedHeader::Via(via.clone()));
    }
    if let Some(from) = request.from() {
        builder = builder.header(TypedHeader::From(from.clone()));
    }
    if let Some(to) = request.to() {
        builder = builder.header(TypedHeader::To(to.clone()));
    }
    if let Some(call_id) = request.call_id() {
        builder = builder.header(TypedHeader::CallId(call_id.clone()));
    }
    if let Some(cseq) = request.cseq() {
        builder = builder.header(TypedHeader::CSeq(cseq.clone()));
    }

    builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));

    builder.build()
}

/// Create a TRYING (100) response for an INVITE request
pub fn create_trying_response(request: &Request) -> Response {
    // Manual build is better here as Response::trying() doesn't copy headers.
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

/// Extract the transaction classification (prefix) and branch from a message
/// Used by manager to determine transaction type and potentially match.
pub fn extract_transaction_parts(message: &Message) -> Result<(TransactionKind, String)> {
    let branch = extract_branch(message)
        .ok_or_else(|| Error::Other("Missing branch parameter in Via header".to_string()))?;

    let kind = match message {
        Message::Request(req) => {
            match req.method() {
                 Method::Invite => TransactionKind::InviteServer,
                 Method::Ack => TransactionKind::InviteServer, // Matches existing IST
                 Method::Cancel => TransactionKind::InviteServer, // Matches existing IST
                 _ => TransactionKind::NonInviteServer,
             }
        }
        Message::Response(_) => {
            let (_, cseq_method) = extract_cseq(message)
                .ok_or_else(|| Error::Other("Missing or invalid CSeq header in Response".to_string()))?;

            if cseq_method == Method::Invite {
                TransactionKind::InviteClient
            } else {
                TransactionKind::NonInviteClient
            }
        }
    };

    Ok((kind, branch))
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
    debug!("WARNING: Using placeholder extract_destination. This function is deprecated and returns None.");
    None
    // Some(std::net::SocketAddr::from(([127, 0, 0, 1], 5071)))
}

/// Creates a transaction key from a SIP message based on RFC 3261 rules.
///
/// This is a simplified placeholder and needs refinement for full RFC compliance.
pub fn transaction_key_from_message(message: &Message) -> Result<TransactionKey> {
    let branch = extract_branch(message)
        .ok_or_else(|| Error::Other("Missing branch in Via for key generation".to_string()))?;
    let method = match message {
        Message::Request(req) => req.method().clone(),
        Message::Response(_) => extract_cseq(message)
                                    .ok_or(Error::Other("Missing or invalid CSeq in Response".to_string()))?
                                    .1, // Get the Method part
    };
    // TODO: Refine key generation according to RFC 3261 Section 17.1.3 and 17.2.3 rigorously.
    Ok(format!("{}-{}", branch, method))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::prelude::*;
    use crate::transaction::TransactionKind;
    use bytes::Bytes;
    use std::str::FromStr;

    // Helper to create a basic request for testing
    fn create_test_request(method: Method, branch: &str) -> Request {
        let uri = "sip:bob@example.com";
        let from_uri = "sip:alice@atlanta.com";
        let via_str = format!("SIP/2.0/UDP pc33.atlanta.com;branch={}", branch);

        // Create header structs first
        let via = Via::parse(&via_str).expect("Failed to parse Via");
        let from = From::new(Address::new(Uri::parse(from_uri).unwrap()).with_tag("fromtag").unwrap());
        let to = To::new(Address::new(Uri::parse(uri).unwrap()));
        let call_id = CallId::new("test-call-id");
        let cseq = CSeq::new(1, method);
        let content_length = ContentLength::new(0);

        RequestBuilder::new(method.clone(), uri).unwrap()
            .header(TypedHeader::Via(via))
            .header(TypedHeader::From(from))
            .header(TypedHeader::To(to))
            .header(TypedHeader::CallId(call_id))
            .header(TypedHeader::CSeq(cseq))
            .header(TypedHeader::ContentLength(content_length))
            .build()
    }

     // Helper to create a basic response for testing
    fn create_test_response(status: StatusCode, branch: &str, cseq_method: Method, to_tag: Option<&str>) -> Response {
        let uri = "sip:bob@example.com";
        let from_uri = "sip:alice@atlanta.com";
        let via_str = format!("SIP/2.0/UDP pc33.atlanta.com;branch={}", branch);

        let via = Via::parse(&via_str).expect("Failed to parse Via");
        let from = From::new(Address::new(Uri::parse(from_uri).unwrap()).with_tag("fromtag").unwrap());
        let call_id = CallId::new("test-call-id");
        let cseq = CSeq::new(1, cseq_method);
        let content_length = ContentLength::new(0);

        let mut to_addr = Address::new(Uri::parse(uri).unwrap());
        if let Some(tag) = to_tag {
            to_addr = to_addr.with_tag(tag).unwrap();
        }
        let to = To::new(to_addr);

        ResponseBuilder::new(status)
            .header(TypedHeader::Via(via))
            .header(TypedHeader::From(from))
            .header(TypedHeader::CallId(call_id))
            .header(TypedHeader::CSeq(cseq))
            .header(TypedHeader::To(to))
            .header(TypedHeader::ContentLength(content_length))
            .build()
    }

    #[test]
    fn test_generate_branch() {
        let branch1 = generate_branch();
        let branch2 = generate_branch();
        assert!(branch1.starts_with("z9hG4bK-"));
        assert!(branch2.starts_with("z9hG4bK-"));
        assert_ne!(branch1, branch2);
        // Check length - z9hG4bK- + 32 hex chars (UUID simple)
        assert_eq!(branch1.len(), 8 + 32);
    }

    #[test]
    fn test_extract_branch() {
        let branch = "z9hG4bK-testbranch";
        let request = create_test_request(Method::Invite, branch);
        let message = Message::Request(request);
        assert_eq!(extract_branch(&message), Some(branch.to_string()));

        let response = create_test_response(StatusCode::Ok, branch, Method::Invite, Some("totag"));
        let message = Message::Response(response);
         assert_eq!(extract_branch(&message), Some(branch.to_string()));
    }

     #[test]
    fn test_extract_call_id() {
        let request = create_test_request(Method::Register, "branch1");
        let message = Message::Request(request);
        assert_eq!(extract_call_id(&message), Some("test-call-id".to_string()));

        let response = create_test_response(StatusCode::Ok, "branch2", Method::Register, Some("tag"));
         let message = Message::Response(response);
        assert_eq!(extract_call_id(&message), Some("test-call-id".to_string()));
    }


    #[test]
    fn test_extract_cseq() {
        let request = create_test_request(Method::Options, "branch3");
        let message = Message::Request(request);
        assert_eq!(extract_cseq(&message), Some((1, Method::Options)));

        let response = create_test_response(StatusCode::Ok, "branch4", Method::Options, Some("tag"));
        let message = Message::Response(response);
        assert_eq!(extract_cseq(&message), Some((1, Method::Options)));
    }


    #[test]
    fn test_create_response() {
        let branch = "z9hG4bK-respbranch";
        let request = create_test_request(Method::Invite, branch);

        let response = create_response(&request, StatusCode::Forbidden);

        assert_eq!(response.status(), StatusCode::Forbidden);
        assert_eq!(response.first_via().unwrap().branch().unwrap(), branch);
        // Use specific accessors in assertions
        assert_eq!(response.from().unwrap().address().tag().unwrap(), "fromtag");
        assert!(response.to().unwrap().address().tag().is_none()); // No tag added by default
        assert_eq!(response.call_id().unwrap().to_string(), "test-call-id");
        assert_eq!(response.cseq().unwrap().sequence(), 1);
        assert_eq!(response.cseq().unwrap().method(), Method::Invite);
        assert_eq!(response.content_length().unwrap().value(), 0);
    }

     #[test]
    fn test_create_response_with_to_tag() {
        let branch = "z9hG4bK-respbranch-tagged";
        let mut request = create_test_request(Method::Invite, branch);
        // Simulate request having a To tag already (e.g., mid-dialog request)
        // Use header_mut() which returns the mutable TypedHeader reference
        if let Some(to_header) = request.header_mut::<To>() {
            let new_addr = to_header.address().clone().with_tag("existingtag").unwrap();
            *to_header = To::new(new_addr);
        } else {
            panic!("Failed to get mutable To header");
        }

        let response = create_response(&request, StatusCode::Ok);

        assert_eq!(response.to().unwrap().address().tag().unwrap(), "existingtag");
    }


    #[test]
    fn test_create_trying_response() {
        let request = create_test_request(Method::Invite, "branch-trying");
        let response = create_trying_response(&request);
         assert_eq!(response.status(), StatusCode::Trying);
         assert_eq!(response.cseq().unwrap().method(), Method::Invite); // CSeq method matches request
    }

     #[test]
    fn test_create_ringing_response() {
        let request = create_test_request(Method::Invite, "branch-ringing");
        let response = create_ringing_response(&request);
         assert_eq!(response.status(), StatusCode::Ringing);
         assert!(response.to().unwrap().address().tag().is_none()); // No To tag yet
    }

     #[test]
    fn test_create_ok_response() {
        let request = create_test_request(Method::Register, "branch-ok");
        let response = create_ok_response(&request);
        assert_eq!(response.status(), StatusCode::Ok);
        assert_eq!(response.cseq().unwrap().method(), Method::Register);
    }

    #[test]
    fn test_extract_transaction_parts_request() {
        let invite_req = create_test_request(Method::Invite, "branch-invite");
        let (kind, branch) = extract_transaction_parts(&Message::Request(invite_req)).unwrap();
        assert_eq!(kind, TransactionKind::InviteServer);
        assert_eq!(branch, "branch-invite");

        let options_req = create_test_request(Method::Options, "branch-options");
        let (kind, branch) = extract_transaction_parts(&Message::Request(options_req)).unwrap();
         assert_eq!(kind, TransactionKind::NonInviteServer);
         assert_eq!(branch, "branch-options");

        let ack_req = create_test_request(Method::Ack, "branch-ack");
         let (kind, branch) = extract_transaction_parts(&Message::Request(ack_req)).unwrap();
        assert_eq!(kind, TransactionKind::InviteServer); // ACK targets IST
        assert_eq!(branch, "branch-ack");

        let cancel_req = create_test_request(Method::Cancel, "branch-cancel");
        let (kind, branch) = extract_transaction_parts(&Message::Request(cancel_req)).unwrap();
        assert_eq!(kind, TransactionKind::InviteServer); // CANCEL targets IST
        assert_eq!(branch, "branch-cancel");
    }

     #[test]
    fn test_extract_transaction_parts_response() {
        let ok_invite_res = create_test_response(StatusCode::Ok, "branch-res-invite", Method::Invite, Some("tag"));
        let (kind, branch) = extract_transaction_parts(&Message::Response(ok_invite_res)).unwrap();
        assert_eq!(kind, TransactionKind::InviteClient);
        assert_eq!(branch, "branch-res-invite");

        let ok_options_res = create_test_response(StatusCode::Ok, "branch-res-options", Method::Options, Some("tag"));
        let (kind, branch) = extract_transaction_parts(&Message::Response(ok_options_res)).unwrap();
        assert_eq!(kind, TransactionKind::NonInviteClient);
         assert_eq!(branch, "branch-res-options");

        let trying_invite_res = create_test_response(StatusCode::Trying, "branch-res-trying", Method::Invite, None);
         let (kind, branch) = extract_transaction_parts(&Message::Response(trying_invite_res)).unwrap();
        assert_eq!(kind, TransactionKind::InviteClient);
        assert_eq!(branch, "branch-res-trying");
    }

     #[test]
    fn test_extract_client_branch_from_response() {
        let ok_invite_res = create_test_response(StatusCode::Ok, "branch-client-res", Method::Invite, Some("tag"));
        let branch = extract_client_branch_from_response(&ok_invite_res);
        assert_eq!(branch, Some("branch-client-res".to_string()));

         let not_found_res = create_test_response(StatusCode::NotFound, "branch-client-nf", Method::Register, None);
        let branch = extract_client_branch_from_response(&not_found_res);
        assert_eq!(branch, Some("branch-client-nf".to_string()));
    }

    #[test]
    fn test_transaction_key_from_message() {
        let invite_req = create_test_request(Method::Invite, "branch-invite");
        let key = transaction_key_from_message(&Message::Request(invite_req)).unwrap();
        assert_eq!(key, "branch-invite-INVITE");

        let ok_invite_res = create_test_response(StatusCode::Ok, "branch-res-invite", Method::Invite, Some("tag"));
        let key = transaction_key_from_message(&Message::Response(ok_invite_res)).unwrap();
        assert_eq!(key, "branch-res-invite-INVITE");

        let options_req = create_test_request(Method::Options, "branch-options");
        let key = transaction_key_from_message(&Message::Request(options_req)).unwrap();
        assert_eq!(key, "branch-options-OPTIONS");

        let not_found_res = create_test_response(StatusCode::NotFound, "branch-client-nf", Method::Register, None);
        let key = transaction_key_from_message(&Message::Response(not_found_res)).unwrap();
        assert_eq!(key, "branch-client-nf-REGISTER");
    }
} 