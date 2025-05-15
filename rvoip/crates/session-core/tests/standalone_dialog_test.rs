use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, HeaderName, TypedHeader,
    types::{call_id::CallId, from::From, to::To}
};
use rvoip_session_core::{
    dialog::{Dialog, DialogId, DialogState},
};
use std::str::FromStr;

// Test helper function to create a mock request
fn create_mock_invite_request() -> Request {
    let mut request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    request.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag using proper API
    let from_uri = Uri::sip("alice@example.com");
    let from_addr = rvoip_sip_core::types::address::Address::new(from_uri).with_tag("alice-tag");
    let from = From(from_addr);
    request.headers.push(TypedHeader::From(from));
    
    // Add To
    let to_uri = Uri::sip("bob@example.com");
    let to = To::new(rvoip_sip_core::types::address::Address::new(to_uri));
    request.headers.push(TypedHeader::To(to));
    
    // Add CSeq
    let cseq = rvoip_sip_core::types::cseq::CSeq::new(1, Method::Invite);
    request.headers.push(TypedHeader::CSeq(cseq));
    
    request
}

// Test helper function to create a mock response
fn create_mock_response(status: StatusCode, with_to_tag: bool) -> Response {
    let mut response = Response::new(status);
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    response.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag using proper API
    let from_uri = Uri::sip("alice@example.com");
    let from_addr = rvoip_sip_core::types::address::Address::new(from_uri).with_tag("alice-tag");
    let from = From(from_addr);
    response.headers.push(TypedHeader::From(from));
    
    // Add To, optionally with tag using proper API
    let to_uri = Uri::sip("bob@example.com");
    let to_addr = if with_to_tag {
        rvoip_sip_core::types::address::Address::new(to_uri).with_tag("bob-tag")
    } else {
        rvoip_sip_core::types::address::Address::new(to_uri)
    };
    let to = To(to_addr);
    response.headers.push(TypedHeader::To(to));
    
    // Add Contact
    let contact_uri = Uri::sip("bob@192.168.1.2");
    let contact_addr = rvoip_sip_core::types::address::Address::new(contact_uri);
    
    // Add contact using the correct API
    let contact_param = rvoip_sip_core::types::contact::ContactParamInfo { address: contact_addr };
    let contact = rvoip_sip_core::types::contact::Contact::new_params(vec![contact_param]);
    response.headers.push(TypedHeader::Contact(contact));
    
    response
}

#[test]
fn test_dialog_creation_from_2xx() {
    // Create a mock INVITE request
    let request = create_mock_invite_request();
    
    // Create a mock 200 OK response with to-tag
    let response = create_mock_response(StatusCode::Ok, true);
    
    // Create dialog as UAC (initiator)
    let dialog = Dialog::from_2xx_response(&request, &response, true);
    assert!(dialog.is_some(), "Dialog creation failed");
    
    let dialog = dialog.unwrap();
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert_eq!(dialog.call_id, "test-call-id");
    assert_eq!(dialog.local_tag, Some("alice-tag".to_string()));
    assert_eq!(dialog.remote_tag, Some("bob-tag".to_string()));
    assert_eq!(dialog.local_seq, 1);
    assert_eq!(dialog.remote_seq, 0);
    assert_eq!(dialog.is_initiator, true);
}

#[test]
fn test_dialog_create_request() {
    // Create a mock INVITE request
    let request = create_mock_invite_request();
    
    // Create a mock 200 OK response with to-tag
    let response = create_mock_response(StatusCode::Ok, true);
    
    // Create dialog
    let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
    
    // Create a BYE request
    let bye_request = dialog.create_request(Method::Bye);
    
    // Verify the request
    assert_eq!(bye_request.method, Method::Bye);
    assert_eq!(dialog.local_seq, 2); // Should be incremented
    
    // Check headers
    assert!(bye_request.header(&HeaderName::CallId).is_some());
    assert!(bye_request.header(&HeaderName::From).is_some());
    assert!(bye_request.header(&HeaderName::To).is_some());
    assert!(bye_request.header(&HeaderName::CSeq).is_some());
} 