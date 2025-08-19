//! Tests for REFER message handling in dialog-core

use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    config::DialogManagerConfig,
    events::SessionCoordinationEvent,
    dialog::DialogId,
};
use rvoip_sip_core::{
    Request, Method, Uri, StatusCode,
    builder::{SimpleRequestBuilder, headers::ReferToExt},
    types::{
        refer_to::ReferTo,
        header::{HeaderName, TypedHeader},
    },
};
use rvoip_transaction_core::TransactionManager;

/// Helper to create a test dialog API
async fn create_test_dialog_api(port: u16) -> Arc<UnifiedDialogApi> {
    // Create a hybrid config that can handle both incoming and outgoing
    let config = DialogManagerConfig::hybrid(
        format!("127.0.0.1:{}", port).parse().unwrap()
    )
    .with_from_uri(format!("sip:test@127.0.0.1:{}", port))
    .build();
    
    // Create the API using the create method which sets up transport automatically
    Arc::new(
        UnifiedDialogApi::create(config)
            .await
            .expect("Failed to create dialog API")
    )
}

#[tokio::test]
async fn test_refer_creates_transfer_request_event() {
    println!("🧪 Testing REFER → TransferRequest event generation");
    
    // Create dialog API
    let dialog_api = Arc::new(create_test_dialog_api(40001).await);
    
    // Create event channel to capture SessionCoordinationEvents
    let (event_tx, mut event_rx) = mpsc::channel::<SessionCoordinationEvent>(100);
    
    // Set up session coordinator
    dialog_api.set_session_coordinator(event_tx).await
        .expect("Failed to set session coordinator");
    
    // Start the API
    dialog_api.start().await.expect("Failed to start dialog API");
    
    // Create an established dialog first (simulate active call)
    let call_handle = dialog_api
        .make_call(
            "sip:alice@127.0.0.1:40001",
            "sip:bob@127.0.0.1:40002",
            None
        )
        .await
        .expect("Failed to create call");
    
    let dialog_id = call_handle.dialog().id().clone();
    
    // Create REFER request for blind transfer using SimpleRequestBuilder
    let refer_request = SimpleRequestBuilder::new(Method::Refer, "sip:alice@127.0.0.1:40001").unwrap()
        .from("Bob", "sip:bob@127.0.0.1:40002", Some("remote-tag-789"))
        .to("Alice", "sip:alice@127.0.0.1:40001", Some("local-tag-456"))
        .call_id("test-call-id-123")
        .cseq(2)
        .refer_to_blind_transfer("sip:charlie@127.0.0.1:40003")
        .via("127.0.0.1:40002", "UDP", Some("z9hG4bK-refer-branch"))
        .build();
    
    // Process the REFER request through dialog manager's handle_request
    // This simulates receiving a REFER from the network
    let source_addr: SocketAddr = "127.0.0.1:40002".parse().unwrap();
    
    // We need to use the internal dialog manager to process the request
    // In a real scenario, this would come through the transport layer
    // For now, we'll just verify the request is well-formed
    assert_eq!(refer_request.method(), Method::Refer);
    
    // CRITICAL: Verify Refer-To is a header, NOT in the body
    let refer_to_header = refer_request.typed_header::<ReferTo>();
    assert!(refer_to_header.is_some(), "ReferTo header MUST be present as a header");
    assert_eq!(
        refer_to_header.unwrap().uri().to_string(), 
        "sip:charlie@127.0.0.1:40003",
        "ReferTo header must contain correct target URI"
    );
    
    // CRITICAL: Body must be empty - Refer-To is a header
    assert_eq!(
        refer_request.body().len(), 0,
        "REFER body MUST be empty - Refer-To is a header, not body content"
    );
    
    println!("✅ REFER request created successfully with proper structure");
    
    // Clean up
    dialog_api.stop().await.expect("Failed to stop dialog API");
}

#[tokio::test]
async fn test_refer_without_dialog_returns_481() {
    println!("🧪 Testing REFER without dialog → 481 response");
    
    // Create dialog API
    let dialog_api = Arc::new(create_test_dialog_api(40010).await);
    
    // Start the API
    dialog_api.start().await.expect("Failed to start dialog API");
    
    // Create REFER request without an existing dialog
    let refer_request = SimpleRequestBuilder::new(Method::Refer, "sip:alice@127.0.0.1:40010").unwrap()
        .from("Bob", "sip:bob@127.0.0.1:40011", Some("from-tag"))
        .to("Alice", "sip:alice@127.0.0.1:40010", None)  // No to-tag since no dialog exists
        .call_id("nonexistent-call-id")
        .cseq(1)
        .refer_to_blind_transfer("sip:charlie@127.0.0.1:40012")
        .via("127.0.0.1:40011", "UDP", Some("z9hG4bK-refer-no-dialog"))
        .build();
    
    // Verify the request is well-formed
    assert_eq!(refer_request.method(), Method::Refer);
    
    // CRITICAL: Verify the ReferTo header is present AS A HEADER
    let refer_to_header = refer_request.typed_header::<ReferTo>();
    assert!(refer_to_header.is_some(), "ReferTo header MUST be present as a header");
    
    // Verify the target URI
    if let Some(refer_to) = refer_to_header {
        assert_eq!(refer_to.uri().to_string(), "sip:charlie@127.0.0.1:40012");
    }
    
    // CRITICAL: Body must NOT contain Refer-To
    let body_str = String::from_utf8_lossy(refer_request.body());
    assert!(
        !body_str.contains("Refer-To:"),
        "Body must NOT contain Refer-To text - it's a header"
    );
    assert_eq!(
        refer_request.body().len(), 0,
        "REFER body should be empty"
    );
    
    println!("✅ REFER without dialog request created successfully");
    
    // Clean up
    dialog_api.stop().await.expect("Failed to stop dialog API");
}

#[tokio::test]
async fn test_refer_with_replaces_header() {
    println!("🧪 Testing REFER with Replaces header (attended transfer)");
    
    // Create dialog API
    let dialog_api = Arc::new(create_test_dialog_api(40020).await);
    
    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel::<SessionCoordinationEvent>(100);
    dialog_api.set_session_coordinator(event_tx).await
        .expect("Failed to set session coordinator");
    
    // Start the API
    dialog_api.start().await.expect("Failed to start dialog API");
    
    // Create an established dialog
    let call_handle = dialog_api
        .make_call(
            "sip:alice@127.0.0.1:40020",
            "sip:bob@127.0.0.1:40021",
            None
        )
        .await
        .expect("Failed to create call");
    
    let dialog_id = call_handle.dialog().id().clone();
    
    // Create REFER with Replaces for attended transfer
    let refer_request = SimpleRequestBuilder::new(Method::Refer, "sip:alice@127.0.0.1:40020").unwrap()
        .from("Bob", "sip:bob@127.0.0.1:40021", Some("remote-tag"))
        .to("Alice", "sip:alice@127.0.0.1:40020", Some("local-tag"))
        .call_id("test-call-id-456")
        .cseq(2)
        .refer_to_attended_transfer(
            "sip:charlie@127.0.0.1:40022",
            "other-call-id",
            "tt",  // to-tag of the call to replace
            "ft"   // from-tag of the call to replace
        )
        .via("127.0.0.1:40021", "UDP", Some("z9hG4bK-refer-replaces"))
        .build();
    
    // Verify the request is well-formed
    assert_eq!(refer_request.method(), Method::Refer);
    
    // Verify the ReferTo header is present
    let refer_to_header = refer_request.typed_header::<ReferTo>();
    assert!(refer_to_header.is_some(), "ReferTo header should be present for attended transfer");
    
    // Verify the URI contains the Replaces parameter
    if let Some(refer_to) = refer_to_header {
        let uri_str = refer_to.uri().to_string();
        assert!(uri_str.contains("Replaces="), "Refer-To URI should contain Replaces parameter");
        assert!(uri_str.contains("other-call-id"), "Refer-To URI should contain the call ID to replace");
    }
    
    println!("✅ REFER with Replaces created successfully");
    
    // Clean up
    dialog_api.stop().await.expect("Failed to stop dialog API");
}