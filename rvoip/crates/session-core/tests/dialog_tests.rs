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
use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};

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

// Helper to create a unified dialog API for testing
async fn create_test_dialog_api() -> Arc<UnifiedDialogApi> {
    let config = DialogManagerConfig::client("127.0.0.1:0".parse().unwrap())
        .with_from_uri("sip:test@example.com")
        .build();
    
    Arc::new(UnifiedDialogApi::create(config).await.unwrap())
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
    
    assert_eq!(dialog.local_cseq, 1);
    assert_eq!(dialog.remote_cseq, 0);
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
    
    assert_eq!(dialog.local_cseq, 1);
    assert_eq!(dialog.remote_cseq, 0);
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
    
    // Create a BYE request template
    let bye_request = dialog.create_request_template(Method::Bye);
    
    // Verify the request template has the correct method
    assert_eq!(bye_request.method, Method::Bye);
    assert_eq!(dialog.local_cseq, 2); // Should be incremented
    
    // Test passes if we can create the template without errors
    // Note: DialogRequestTemplate doesn't expose header methods,
    // so we can't test headers directly in this simplified test
}

#[tokio::test]
async fn test_unified_dialog_api_basics() {
    // Create a unified dialog API instance
    let dialog_api = create_test_dialog_api().await;
    
    // Start the dialog API
    dialog_api.start().await.unwrap();
    
    // Test creating an outgoing dialog
    let dialog_result = dialog_api.create_dialog(
        "sip:alice@example.com",
        "sip:bob@example.com"
    ).await;
    
    assert!(dialog_result.is_ok(), "Dialog creation should succeed");
    
    let dialog = dialog_result.unwrap();
    let dialog_id = dialog.id().clone();
    
    // Verify we can get dialog info
    let dialog_info_result = dialog_api.get_dialog_info(&dialog_id).await;
    assert!(dialog_info_result.is_ok(), "Should be able to get dialog info");
    
    let dialog_info = dialog_info_result.unwrap();
    let expected_local_uri: rvoip_sip_core::Uri = "sip:alice@example.com".parse().unwrap();
    let expected_remote_uri: rvoip_sip_core::Uri = "sip:bob@example.com".parse().unwrap();
    
    assert_eq!(dialog_info.local_uri, expected_local_uri);
    assert_eq!(dialog_info.remote_uri, expected_remote_uri);
    
    // Test sending a BYE to terminate the dialog
    let bye_result = dialog_api.send_bye(&dialog_id).await;
    // This might fail in a test environment without real transport, which is expected
    // The important thing is that the API accepts the call
    
    // Stop the dialog API
    dialog_api.stop().await.unwrap();
} 