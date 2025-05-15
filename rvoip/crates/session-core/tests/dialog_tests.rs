use std::sync::Arc;
use std::str::FromStr;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, HeaderName, TypedHeader,
    builder::{SimpleRequestBuilder, SimpleResponseBuilder},
    types::{
        call_id::CallId,
        from::From as FromHeader,
        to::To as ToHeader,
        cseq::CSeq,
        address::Address,
        contact::Contact,
    }
};
use rvoip_transaction_core::{
    TransactionManager, TransactionEvent, TransactionKey, TransactionKind
};
use tokio::sync::mpsc;
use uuid::Uuid;

use rvoip_session_core::{
    dialog::{Dialog, DialogId, DialogManager, DialogState},
    events::EventBus
};

// Create a realistic transport for testing
#[derive(Debug, Clone)]
struct TestTransport {
    local_addr: std::net::SocketAddr,
}

impl TestTransport {
    fn new() -> Self {
        Self {
            local_addr: "127.0.0.1:5060".parse().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for TestTransport {
    fn local_addr(&self) -> std::result::Result<std::net::SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: std::net::SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        // Mock implementation: don't actually send anything
        Ok(())
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

// Utility functions to create proper SIP messages
fn create_invite_request() -> Request {
    // Generate a unique Call-ID for this test
    let call_id = format!("test-call-{}", Uuid::new_v4().as_simple());
    
    // Create a proper INVITE request with all required headers
    let request = SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
        .unwrap()
        .from("Alice", "sip:alice@example.com", Some("alice-tag"))
        .to("Bob", "sip:bob@example.com", None)
        .call_id(&call_id)
        .cseq(1)
        .contact("sip:alice@192.168.1.1", Some("Alice"))
        .via("192.168.1.1:5060", "UDP", Some(&format!("z9hG4bK-{}", Uuid::new_v4().as_simple())))
        .build();
        
    request
}

fn create_sip_response(request: &Request, status: StatusCode, with_to_tag: bool) -> Response {
    // Get original To header to extract display name and URI
    let (display_name, uri) = if let Some(TypedHeader::To(to)) = request.header(&HeaderName::To) {
        (
            to.address().display_name().unwrap_or("").to_string(),
            to.address().uri.to_string()
        )
    } else {
        ("".to_string(), "sip:unknown@example.com".to_string())
    };
    
    // Create a proper response from the request
    let mut builder = SimpleResponseBuilder::response_from_request(request, status, None);
    
    // Add a Contact header for dialog establishment
    builder = builder.contact("sip:bob@192.168.1.2", Some("Bob"));
    
    // If this is a response that should establish a dialog, add a to-tag 
    // by overriding the To header with one that includes a tag
    if with_to_tag {
        let to_tag = format!("bob-tag-{}", Uuid::new_v4().as_simple());
        builder = builder.to(&display_name, &uri, Some(&to_tag));
    }
    
    // Build the response
    builder.build()
}

#[test]
fn test_dialog_creation_from_2xx() {
    // Create a proper INVITE request
    let request = create_invite_request();
    
    // Create a proper 200 OK response with to-tag
    let response = create_sip_response(&request, StatusCode::Ok, true);
    
    // Verify the tag is present in the To header
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        assert!(to.tag().is_some(), "To header should have a tag in response");
    } else {
        panic!("Response is missing To header");
    }
    
    // Create dialog as UAC (initiator)
    let dialog = Dialog::from_2xx_response(&request, &response, true);
    assert!(dialog.is_some(), "Dialog creation failed");
    
    let dialog = dialog.unwrap();
    assert_eq!(dialog.state, DialogState::Confirmed);
    
    // Verify the dialog has the correct Call-ID
    if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
        assert_eq!(dialog.call_id, call_id.to_string());
    } else {
        panic!("Request is missing Call-ID header");
    }
    
    // Verify From tag is set as local tag
    if let Some(TypedHeader::From(from)) = request.header(&HeaderName::From) {
        assert_eq!(dialog.local_tag, from.tag().map(|s| s.to_string()));
    } else {
        panic!("Request is missing From header");
    }
    
    // Verify To tag is set as remote tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        assert_eq!(dialog.remote_tag, to.tag().map(|s| s.to_string()));
    } else {
        panic!("Response is missing To header");
    }
    
    assert_eq!(dialog.local_seq, 1);
    assert_eq!(dialog.remote_seq, 0);
    assert_eq!(dialog.is_initiator, true);
}

#[test]
fn test_dialog_creation_from_provisional() {
    // Create a proper INVITE request
    let request = create_invite_request();
    
    // Create a proper 180 Ringing response with to-tag
    let response = create_sip_response(&request, StatusCode::Ringing, true);
    
    // Create dialog as UAC (initiator)
    let dialog = Dialog::from_provisional_response(&request, &response, true);
    assert!(dialog.is_some(), "Dialog creation failed");
    
    let dialog = dialog.unwrap();
    assert_eq!(dialog.state, DialogState::Early);
    
    // Verify the dialog has the correct Call-ID
    if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
        assert_eq!(dialog.call_id, call_id.to_string());
    }
    
    // Verify From tag is set as local tag
    if let Some(TypedHeader::From(from)) = request.header(&HeaderName::From) {
        assert_eq!(dialog.local_tag, from.tag().map(|s| s.to_string()));
    }
    
    // Verify To tag is set as remote tag
    if let Some(TypedHeader::To(to)) = response.header(&HeaderName::To) {
        assert_eq!(dialog.remote_tag, to.tag().map(|s| s.to_string()));
    }
    
    assert_eq!(dialog.local_seq, 1);
    assert_eq!(dialog.remote_seq, 0);
    assert_eq!(dialog.is_initiator, true);
}

#[test]
fn test_dialog_update_from_2xx() {
    // Create a proper INVITE request
    let request = create_invite_request();
    
    // Create a proper 180 Ringing response with to-tag
    let provisional = create_sip_response(&request, StatusCode::Ringing, true);
    
    // Create early dialog
    let mut dialog = Dialog::from_provisional_response(&request, &provisional, true).unwrap();
    assert_eq!(dialog.state, DialogState::Early);
    
    // Create a proper 200 OK response with to-tag (possibly different from provisional)
    let final_response = create_sip_response(&request, StatusCode::Ok, true);
    
    // Update the dialog
    let updated = dialog.update_from_2xx(&final_response);
    assert!(updated, "Dialog update failed");
    assert_eq!(dialog.state, DialogState::Confirmed);
}

#[test]
fn test_dialog_create_request() {
    // Create a proper INVITE request
    let request = create_invite_request();
    
    // Create a proper 200 OK response with to-tag
    let response = create_sip_response(&request, StatusCode::Ok, true);
    
    // Create dialog
    let mut dialog = Dialog::from_2xx_response(&request, &response, true).unwrap();
    
    // Create a BYE request
    let bye_request = dialog.create_request(Method::Bye);
    
    // Verify the request
    assert_eq!(bye_request.method, Method::Bye);
    assert_eq!(dialog.local_seq, 2); // Should be incremented
    
    // Check required headers
    assert!(bye_request.header(&HeaderName::CallId).is_some());
    assert!(bye_request.header(&HeaderName::From).is_some());
    assert!(bye_request.header(&HeaderName::To).is_some());
    assert!(bye_request.header(&HeaderName::CSeq).is_some());
    
    // Verify From contains local tag
    if let Some(TypedHeader::From(from)) = bye_request.header(&HeaderName::From) {
        assert_eq!(from.tag(), dialog.local_tag.as_deref());
    } else {
        panic!("BYE request missing From header");
    }
    
    // Verify To contains remote tag
    if let Some(TypedHeader::To(to)) = bye_request.header(&HeaderName::To) {
        assert_eq!(to.tag(), dialog.remote_tag.as_deref());
    } else {
        panic!("BYE request missing To header");
    }
    
    // Verify CSeq has correct number and method
    if let Some(TypedHeader::CSeq(cseq)) = bye_request.header(&HeaderName::CSeq) {
        assert_eq!(cseq.sequence(), dialog.local_seq);
        assert_eq!(cseq.method, Method::Bye);
    } else {
        panic!("BYE request missing CSeq header");
    }
}

#[tokio::test]
async fn test_dialog_manager_basics() {
    // Create a real TransactionManager with test transport
    let (transport_tx, transport_rx) = mpsc::channel(10);
    let transport = Arc::new(TestTransport::new());
    
    let (transaction_manager, _events_rx) = 
        TransactionManager::new(transport.clone(), transport_rx, Some(10)).await.unwrap();
    let transaction_manager = Arc::new(transaction_manager);
    
    let event_bus = EventBus::new(100);
    
    // Create a dialog manager with the real TransactionManager
    let dialog_manager = DialogManager::new(transaction_manager.clone(), event_bus);
    
    // Start the dialog manager
    let _events_rx = dialog_manager.start().await;
    
    // Create a proper INVITE request
    let request = create_invite_request();
    
    // Create a proper 200 OK response with to-tag
    let response = create_sip_response(&request, StatusCode::Ok, true);
    
    // Create a transaction ID for testing
    let branch = format!("z9hG4bK-{}", Uuid::new_v4().as_simple());
    let test_transaction_id = TransactionKey::new(
        branch, 
        Method::Invite,
        false // Client transaction
    );
    
    // Create dialog through the dialog manager's API
    let dialog_id = dialog_manager.create_dialog_from_transaction(
        &test_transaction_id, 
        &request, 
        &response, 
        true
    ).await.unwrap();
    
    // Associate with session
    let mock_session_id = rvoip_session_core::session::SessionId::new();
    dialog_manager.associate_with_session(&dialog_id, &mock_session_id).unwrap();
    
    // Retrieve the dialog
    let retrieved = dialog_manager.get_dialog(&dialog_id).unwrap();
    
    // Verify the dialog has the correct Call-ID
    if let Some(TypedHeader::CallId(call_id)) = request.header(&HeaderName::CallId) {
        assert_eq!(retrieved.call_id, call_id.to_string());
    }
    
    // Terminate the dialog
    dialog_manager.terminate_dialog(&dialog_id).await.unwrap();
    let terminated = dialog_manager.get_dialog(&dialog_id).unwrap();
    assert_eq!(terminated.state, DialogState::Terminated);
    
    // Clean up terminated dialogs
    let cleaned = dialog_manager.cleanup_terminated();
    assert_eq!(cleaned, 1, "Should have cleaned up 1 terminated dialog");
} 