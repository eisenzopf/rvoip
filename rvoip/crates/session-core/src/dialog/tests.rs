use std::str::FromStr;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, TypedHeader,
    types::{
        call_id::CallId,
        from::From as FromHeader,
        to::To as ToHeader,
        cseq::CSeq,
        address::Address,
        param::Param,
        contact::Contact,
        contact::ContactParamInfo,
    }
};

use super::dialog_state::DialogState;
use super::dialog_id::DialogId;
use super::dialog_impl::Dialog;

/// Create a mock INVITE request for testing
pub fn create_mock_invite_request() -> Request {
    let mut request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    request.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag using proper API
    let from_uri = Uri::sip("alice@example.com");
    let from_addr = Address::new(from_uri).with_tag("alice-tag");
    let from = FromHeader(from_addr);
    request.headers.push(TypedHeader::From(from));
    
    // Add To
    let to_uri = Uri::sip("bob@example.com");
    let to = ToHeader::new(Address::new(to_uri));
    request.headers.push(TypedHeader::To(to));
    
    // Add CSeq
    let cseq = CSeq::new(1, Method::Invite);
    request.headers.push(TypedHeader::CSeq(cseq));
    
    request
}

/// Create a mock response for testing
pub fn create_mock_response(status: StatusCode, with_to_tag: bool) -> Response {
    let mut response = Response::new(status);
    
    // Add Call-ID
    let call_id = CallId("test-call-id".to_string());
    response.headers.push(TypedHeader::CallId(call_id));
    
    // Add From with tag using proper API
    let from_uri = Uri::sip("alice@example.com");
    let from_addr = Address::new(from_uri).with_tag("alice-tag");
    let from = FromHeader(from_addr);
    response.headers.push(TypedHeader::From(from));
    
    // Add To, optionally with tag using proper API
    let to_uri = Uri::sip("bob@example.com");
    let to_addr = if with_to_tag {
        Address::new(to_uri).with_tag("bob-tag")
    } else {
        Address::new(to_uri)
    };
    let to = ToHeader(to_addr);
    response.headers.push(TypedHeader::To(to));
    
    // Add Contact
    let contact_uri = Uri::sip("bob@192.168.1.2");
    let contact_addr = Address::new(contact_uri);

    // Create contact header using the correct API
    let contact_param = ContactParamInfo { address: contact_addr };
    let contact = Contact::new_params(vec![contact_param]);
    response.headers.push(TypedHeader::Contact(contact));
    
    response
}

#[test]
fn test_integrated_dialog_creation() {
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