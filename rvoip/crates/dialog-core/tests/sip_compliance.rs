//! SIP RFC 3261 compliance tests for dialog-core
//!
//! Tests dialog behavior against RFC 3261 specifications.

use std::sync::Arc;

use rvoip_dialog_core::{DialogManager, DialogError, Dialog, DialogState};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Request, Response, Method, StatusCode, HeaderName, TypedHeader, Uri};

/// Test RFC 3261 dialog creation requirements
#[tokio::test]
async fn test_rfc3261_dialog_creation_from_2xx() {
    // Test that dialogs are created correctly from 2xx responses to INVITE
    // per RFC 3261 Section 12.1.1
    
    // Create a mock INVITE request
    let invite_request = create_mock_invite();
    
    // Create a mock 200 OK response with proper headers
    let ok_response = create_mock_200_ok_response();
    
    // Test dialog creation for initiator (UAC)
    let dialog_uac = Dialog::from_2xx_response(&invite_request, &ok_response, true);
    assert!(dialog_uac.is_some());
    
    let dialog = dialog_uac.unwrap();
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(dialog.is_initiator);
    assert!(dialog.local_tag.is_some());
    assert!(dialog.remote_tag.is_some());
    
    // Test dialog creation for recipient (UAS)
    let dialog_uas = Dialog::from_2xx_response(&invite_request, &ok_response, false);
    assert!(dialog_uas.is_some());
    
    let dialog = dialog_uas.unwrap();
    assert_eq!(dialog.state, DialogState::Confirmed);
    assert!(!dialog.is_initiator);
}

/// Test RFC 3261 early dialog creation
#[tokio::test]
async fn test_rfc3261_early_dialog_creation() {
    // Test early dialog creation from provisional responses
    // per RFC 3261 Section 12.1.1
    
    let invite_request = create_mock_invite();
    let ringing_response = create_mock_180_ringing_response();
    
    // Test early dialog creation
    let early_dialog = Dialog::from_provisional_response(&invite_request, &ringing_response, true);
    assert!(early_dialog.is_some());
    
    let dialog = early_dialog.unwrap();
    assert_eq!(dialog.state, DialogState::Early);
    assert!(dialog.is_initiator);
    
    // Test that provisional responses without To tag don't create dialogs
    let no_tag_response = create_mock_180_no_tag_response();
    let no_dialog = Dialog::from_provisional_response(&invite_request, &no_tag_response, true);
    assert!(no_dialog.is_none());
}

/// Test RFC 3261 sequence number handling
#[tokio::test]
async fn test_rfc3261_sequence_number_handling() {
    // Test CSeq handling per RFC 3261 Section 12.2.1.1
    
    let mut dialog = Dialog::new(
        "cseq-test-call-id".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    // Initial sequence number should be 0
    assert_eq!(dialog.local_seq, 0);
    
    // Creating a request should increment sequence number
    let _request1 = dialog.create_request(Method::Bye);
    assert_eq!(dialog.local_seq, 1);
    
    // Another request should increment further
    let _request2 = dialog.create_request(Method::Info);
    assert_eq!(dialog.local_seq, 2);
    
    // ACK requests should NOT increment sequence number (RFC 3261)
    let _ack_request = dialog.create_request(Method::Ack);
    assert_eq!(dialog.local_seq, 2); // Should remain the same
}

/// Test RFC 3261 dialog ID requirements
#[tokio::test]
async fn test_rfc3261_dialog_id_requirements() {
    // Test dialog ID tuple per RFC 3261 Section 12
    // Dialog ID = Call-ID + local tag + remote tag
    
    let dialog = Dialog::new(
        "dialog-id-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag-123".to_string()),
        Some("remote-tag-456".to_string()),
        true,
    );
    
    let dialog_tuple = dialog.dialog_id_tuple().unwrap();
    assert_eq!(dialog_tuple.0, "dialog-id-test");
    assert_eq!(dialog_tuple.1, "local-tag-123");
    assert_eq!(dialog_tuple.2, "remote-tag-456");
    
    // Test that incomplete dialog tuples return None
    let incomplete_dialog = Dialog::new(
        "incomplete-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        None, // No remote tag
        true,
    );
    
    assert!(incomplete_dialog.dialog_id_tuple().is_none());
}

/// Test RFC 3261 dialog state transitions
#[tokio::test]
async fn test_rfc3261_dialog_state_transitions() {
    // Test proper dialog state transitions per RFC 3261
    
    let mut dialog = Dialog::new(
        "state-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("local-tag".to_string()),
        None, // Will be set by response
        true,
    );
    
    // Should start in Initial state
    assert_eq!(dialog.state, DialogState::Initial);
    
    // Simulate receiving a 200 OK response to transition to Confirmed
    let ok_response = create_mock_200_ok_response();
    
    // This would normally be handled by the dialog manager
    // For this test, we manually set the state to Early first
    dialog.state = DialogState::Early;
    dialog.remote_tag = Some("remote-tag-from-response".to_string());
    
    // Now test the 2xx update
    let updated = dialog.update_from_2xx(&ok_response);
    assert!(updated);
    assert_eq!(dialog.state, DialogState::Confirmed);
}

/// Test RFC 3261 request creation within dialog
#[tokio::test]
async fn test_rfc3261_request_creation_within_dialog() {
    // Test that requests created within dialog have proper headers
    // per RFC 3261 Section 12.2.1.1
    
    let mut dialog = Dialog::new(
        "request-creation-test".to_string(),
        "sip:alice@example.com".parse().unwrap(),
        "sip:bob@example.com".parse().unwrap(),
        Some("alice-tag".to_string()),
        Some("bob-tag".to_string()),
        true,
    );
    
    let request = dialog.create_request(Method::Bye);
    
    // Verify required headers are present
    assert!(request.header(&HeaderName::CallId).is_some());
    assert!(request.header(&HeaderName::From).is_some());
    assert!(request.header(&HeaderName::To).is_some());
    assert!(request.header(&HeaderName::CSeq).is_some());
    
    // Verify Call-ID matches dialog
    if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
        assert_eq!(call_id.to_string(), dialog.call_id);
    } else {
        panic!("Call-ID header missing or wrong type");
    }
    
    // Verify method and URI
    assert_eq!(request.method, Method::Bye);
    assert_eq!(request.uri, dialog.remote_target);
}

// Helper functions to create mock SIP messages

fn create_mock_invite() -> Request {
    let uri: Uri = "sip:bob@example.com".parse().unwrap();
    let mut request = Request::new(Method::Invite, uri);
    
    // Add required headers
    request.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId("mock-call-id".to_string())
    ));
    
    request.headers.push(TypedHeader::CSeq(
        rvoip_sip_core::types::cseq::CSeq::new(1, Method::Invite)
    ));
    
    // Add From header
    let from_uri: Uri = "sip:alice@example.com".parse().unwrap();
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.set_tag("alice-tag");
    request.headers.push(TypedHeader::From(
        rvoip_sip_core::types::from::From(from_addr)
    ));
    
    // Add To header (no tag initially)
    let to_uri: Uri = "sip:bob@example.com".parse().unwrap();
    let to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    request.headers.push(TypedHeader::To(
        rvoip_sip_core::types::to::To(to_addr)
    ));
    
    request
}

fn create_mock_200_ok_response() -> Response {
    let mut response = Response::new(StatusCode::Ok);
    
    // Add Call-ID
    response.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId("mock-call-id".to_string())
    ));
    
    // Add From header with tag
    let from_uri: Uri = "sip:alice@example.com".parse().unwrap();
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.set_tag("alice-tag");
    response.headers.push(TypedHeader::From(
        rvoip_sip_core::types::from::From(from_addr)
    ));
    
    // Add To header with tag (generated by UAS)
    let to_uri: Uri = "sip:bob@example.com".parse().unwrap();
    let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    to_addr.set_tag("bob-tag");
    response.headers.push(TypedHeader::To(
        rvoip_sip_core::types::to::To(to_addr)
    ));
    
    // Add Contact header
    let contact_uri: Uri = "sip:bob@192.168.1.100:5060".parse().unwrap();
    let contact_addr = rvoip_sip_core::types::address::Address::new(contact_uri);
    let contact_param = rvoip_sip_core::types::contact::ContactParamInfo {
        address: contact_addr,
        params: Vec::new(),
    };
    response.headers.push(TypedHeader::Contact(
        rvoip_sip_core::types::contact::Contact(vec![
            rvoip_sip_core::types::contact::ContactValue::Params(vec![contact_param])
        ])
    ));
    
    response
}

fn create_mock_180_ringing_response() -> Response {
    let mut response = Response::new(StatusCode::Ringing);
    
    // Add Call-ID
    response.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId("mock-call-id".to_string())
    ));
    
    // Add From header with tag
    let from_uri: Uri = "sip:alice@example.com".parse().unwrap();
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.set_tag("alice-tag");
    response.headers.push(TypedHeader::From(
        rvoip_sip_core::types::from::From(from_addr)
    ));
    
    // Add To header with tag (creates early dialog)
    let to_uri: Uri = "sip:bob@example.com".parse().unwrap();
    let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    to_addr.set_tag("bob-early-tag");
    response.headers.push(TypedHeader::To(
        rvoip_sip_core::types::to::To(to_addr)
    ));
    
    // Add Contact header
    let contact_uri: Uri = "sip:bob@192.168.1.100:5060".parse().unwrap();
    let contact_addr = rvoip_sip_core::types::address::Address::new(contact_uri);
    let contact_param = rvoip_sip_core::types::contact::ContactParamInfo {
        address: contact_addr,
        params: Vec::new(),
    };
    response.headers.push(TypedHeader::Contact(
        rvoip_sip_core::types::contact::Contact(vec![
            rvoip_sip_core::types::contact::ContactValue::Params(vec![contact_param])
        ])
    ));
    
    response
}

fn create_mock_180_no_tag_response() -> Response {
    let mut response = Response::new(StatusCode::Ringing);
    
    // Add Call-ID
    response.headers.push(TypedHeader::CallId(
        rvoip_sip_core::types::call_id::CallId("mock-call-id".to_string())
    ));
    
    // Add From header with tag
    let from_uri: Uri = "sip:alice@example.com".parse().unwrap();
    let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
    from_addr.set_tag("alice-tag");
    response.headers.push(TypedHeader::From(
        rvoip_sip_core::types::from::From(from_addr)
    ));
    
    // Add To header WITHOUT tag (should not create early dialog)
    let to_uri: Uri = "sip:bob@example.com".parse().unwrap();
    let to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
    response.headers.push(TypedHeader::To(
        rvoip_sip_core::types::to::To(to_addr)
    ));
    
    response
} 