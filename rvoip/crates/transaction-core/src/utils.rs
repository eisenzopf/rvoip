use rand::{thread_rng, Rng};
// Use the prelude for easier access to common types
use rvoip_sip_core::prelude::*;
use std::str::FromStr;
use tracing::debug;

use crate::error::{self, Error, Result};
use crate::transaction::TransactionKey; // Import TransactionKey
use crate::transaction::TransactionKind; // Import Kind

use uuid::Uuid;

/// Generate a random branch parameter for Via header (RFC 3261 magic cookie + random string)
pub fn generate_branch() -> String {
    format!("z9hG4bK-{}", Uuid::new_v4().simple())
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
        .header(&HeaderName::CallId)
        .and_then(|h| if let TypedHeader::CallId(cid) = h { Some(cid.to_string()) } else { None })
}

/// Extract the CSeq sequence number and method from a message
pub fn extract_cseq(message: &Message) -> Option<(u32, Method)> {
    message
        .header(&HeaderName::CSeq)
        .and_then(|h| if let TypedHeader::CSeq(cseq) = h { Some((cseq.sequence(), cseq.method().clone())) } else { None })
}

/// Create a response based on a request
pub fn create_response(request: &Request, status: StatusCode) -> Response {
    let mut builder = ResponseBuilder::new(status);
    
    // Copy needed headers from request to response using the header method
    if let Some(header) = request.header(&HeaderName::Via) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::From) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::To) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CallId) {
        builder = builder.header(header.clone());
    }
    if let Some(header) = request.header(&HeaderName::CSeq) {
        builder = builder.header(header.clone());
    }
    
    // Add Content-Length: 0
    builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
    
    builder.build()
}

/// Convenience method to create a 100 Trying response
pub fn create_trying_response(request: &Request) -> Response {
    create_response(request, StatusCode::Trying)
}

/// Convenience method to create a 180 Ringing response
pub fn create_ringing_response(request: &Request) -> Response {
    create_response(request, StatusCode::Ringing)
}

/// Convenience method to create a 200 OK response
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

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::types::Contact;
    use rvoip_sip_core::types::uri::Uri;
    use rvoip_sip_core::types::Scheme;
    use rvoip_sip_core::types::Host;
    use rvoip_sip_core::types::address::Address;

    #[test]
    fn test_create_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);

        // Check status code
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check version (as string instead of enum constant)
        assert_eq!(response.version.to_string(), "SIP/2.0");
        
        // Check headers were copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // Branch should exist but we don't check its exact value since it's generated
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        if let Some(TypedHeader::From(from)) = response.header(&HeaderName::From) {
            assert_eq!(from.address().uri.host, Host::Domain("example.com".to_string()));
        } else {
            panic!("Missing From header in response");
        }
        
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.address().uri.host, Host::Domain("example.net".to_string()));
            assert!(to.tag().is_none(), "To tag should not be present");
        } else {
            panic!("Missing To header in response");
        }
        
        if let Some(TypedHeader::CallId(_)) = response.header(&HeaderName::CallId) {
            // Call-ID exists, which is what we care about
        } else {
            panic!("Missing Call-ID header in response");
        }
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
            assert_eq!(cseq.sequence(), 1);
        } else {
            panic!("Missing CSeq header in response");
        }
        
        if let Some(TypedHeader::ContentLength(content_length)) = response.header(&HeaderName::ContentLength) {
            assert_eq!(content_length.0, 0);
        } else {
            panic!("Missing Content-Length header in response");
        }
    }

    #[test]
    fn test_create_response_with_to_tag() {
        let request = create_test_request(Method::Invite);
        let mut response = create_response(&request, StatusCode::Ok);

        // Add a tag to the To header
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            // Create a new To header with a tag
            let new_to = to.clone().with_tag("totag");
            
            // Replace the To header
            response.headers.retain(|h| !matches!(h, TypedHeader::To(_)));
            response.headers.push(TypedHeader::To(new_to));
        }

        // Check status code is correct
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check Via header was copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // We only check that a branch exists, not its specific value
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        // Check the To header now has a tag
        if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
            assert_eq!(to.tag().unwrap(), "totag");
        } else {
            panic!("Missing To header in response");
        }
    }

    #[test]
    fn test_create_trying_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Trying);
        assert_eq!(response.status(), StatusCode::Trying);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ringing_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ringing);
        assert_eq!(response.status(), StatusCode::Ringing);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_ok_response() {
        let request = create_test_request(Method::Invite);
        let response = create_response(&request, StatusCode::Ok);
        assert_eq!(response.status(), StatusCode::Ok);
        
        if let Some(TypedHeader::CSeq(cseq)) = response.header(&HeaderName::CSeq) {
            assert_eq!(*cseq.method(), Method::Invite);
        } else {
            panic!("Missing CSeq header in response");
        }
    }

    #[test]
    fn test_create_response_with_contact() {
        let request = create_test_request(Method::Invite);
        let mut response = create_response(&request, StatusCode::Ok);

        // Create a Contact with proper API according to documentation example:
        // 1. Create a URI
        let uri = Uri::new(Scheme::Sip, Host::Domain("example.org".to_string()));
        
        // 2. Create an Address with the URI
        let address = Address::new_with_display_name("Test User", uri);
        
        // 3. Create a ContactParamInfo with the Address
        let contact_info = ContactParamInfo { address };
        
        // 4. Create a Contact with the params
        let contact = Contact::new_params(vec![contact_info]);
        
        // Add the contact header
        response.headers.push(TypedHeader::Contact(contact.clone()));

        // Check status code is correct
        assert_eq!(response.status(), StatusCode::Ok);
        
        // Check Via header was copied correctly
        if let Some(TypedHeader::Via(via)) = response.header(&HeaderName::Via) {
            // Use proper accessors for Via
            let via_header = via.headers().first().unwrap();
            assert_eq!(via_header.sent_by_host, Host::Domain("example.com".to_string()));
            // We only check that a branch exists, not its specific value
            assert!(via.branch().is_some());
        } else {
            panic!("Missing Via header in response");
        }
        
        // Check the Contact header was added and has the correct address
        if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            // Get the first address and verify its properties
            if let Some(addr) = contact.address() {
                assert_eq!(addr.uri.host, Host::Domain("example.org".to_string()));
                // Verify display name
                assert_eq!(addr.display_name, Some("Test User".to_string()));
            } else {
                panic!("Contact has no address");
            }
        } else {
            panic!("Missing Contact header in response");
        }
    }
} 