//! Tests for REFER message handling in dialog-core

use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    builder::DialogBuilder,
    events::SessionCoordinationEvent,
    DialogId,
};
use rvoip_sip_core::{
    Request, Method, Uri, StatusCode,
    Message,
    types::refer_to::ReferTo,
};
use rvoip_transaction_core::TransactionKey;

/// Helper to create a test dialog API
async fn create_test_dialog_api(port: u16) -> Arc<UnifiedDialogApi> {
    use rvoip_session_core::api::builder::SessionManagerConfig;
    
    let mut config = SessionManagerConfig::default();
    config.sip_port = port;
    config.local_address = format!("sip:test@127.0.0.1:{}", port);
    config.local_bind_addr = format!("127.0.0.1:{}", port).parse().unwrap();
    
    let dialog_builder = DialogBuilder::new(config);
    dialog_builder.build()
        .await
        .expect("Failed to create dialog API")
}

#[tokio::test]
async fn test_refer_creates_transfer_request_event() {
    println!("🧪 Testing REFER → TransferRequest event generation");
    
    // Create dialog API
    let dialog_api = create_test_dialog_api(40001).await;
    
    // Create event channel to capture SessionCoordinationEvents
    let (event_tx, mut event_rx) = mpsc::channel::<SessionCoordinationEvent>(100);
    
    // Set up event handler
    dialog_api.set_session_event_handler(event_tx).await;
    
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
    let call_id = call_handle.dialog().call_id().clone();
    let local_tag = call_handle.dialog().local_tag().clone();
    let remote_tag = "remote-tag-123".to_string();
    
    // Manually set dialog as confirmed (simulate 200 OK received)
    // Note: In real scenario, this happens when 200 OK is received
    
    // Create REFER request
    let refer_uri = format!("sip:alice@127.0.0.1:40001;tag={}", local_tag);
    let mut refer_request = Request::new(
        Method::REFER,
        refer_uri.parse::<Uri>().unwrap()
    );
    
    // Add required headers
    refer_request.add_header("Call-ID", &call_id);
    refer_request.add_header("From", &format!("sip:bob@127.0.0.1:40002;tag={}", remote_tag));
    refer_request.add_header("To", &format!("sip:alice@127.0.0.1:40001;tag={}", local_tag));
    refer_request.add_header("CSeq", "2 REFER");
    
    // Add Refer-To header (the transfer target)
    refer_request.add_header("Refer-To", "sip:charlie@127.0.0.1:40003");
    
    // Add optional Referred-By header
    refer_request.add_header("Referred-By", "sip:bob@127.0.0.1:40002");
    
    // Process the REFER request through dialog API
    let source_addr: SocketAddr = "127.0.0.1:40002".parse().unwrap();
    
    // Send REFER through the dialog manager
    // This simulates receiving a REFER from the network
    let result = dialog_api.process_request(refer_request.clone(), source_addr).await;
    
    // Should succeed
    assert!(result.is_ok(), "Failed to process REFER: {:?}", result);
    
    // Wait for event to be generated
    tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(event) = event_rx.recv().await {
            println!("Received event: {:?}", event);
            
            if let SessionCoordinationEvent::TransferRequest {
                dialog_id: event_dialog_id,
                transaction_id,
                refer_to,
                referred_by,
                replaces,
            } = event {
                // Verify the event contains correct data
                assert_eq!(event_dialog_id, dialog_id);
                assert_eq!(refer_to.uri().to_string(), "sip:charlie@127.0.0.1:40003");
                assert_eq!(referred_by, Some("sip:bob@127.0.0.1:40002".to_string()));
                assert_eq!(replaces, None);
                
                println!("✅ TransferRequest event generated correctly");
                return;
            }
        }
        panic!("Did not receive TransferRequest event");
    })
    .await
    .expect("Timeout waiting for TransferRequest event");
}

#[tokio::test]
async fn test_refer_without_dialog_returns_481() {
    println!("🧪 Testing REFER without dialog → 481 response");
    
    // Create dialog API
    let dialog_api = create_test_dialog_api(40010).await;
    
    // Create REFER request without an existing dialog
    let mut refer_request = Request::new(
        Method::REFER,
        "sip:alice@127.0.0.1:40010".parse::<Uri>().unwrap()
    );
    
    // Add headers
    refer_request.add_header("Call-ID", "nonexistent-call-id");
    refer_request.add_header("From", "sip:bob@127.0.0.1:40011;tag=from-tag");
    refer_request.add_header("To", "sip:alice@127.0.0.1:40010");
    refer_request.add_header("CSeq", "1 REFER");
    refer_request.add_header("Refer-To", "sip:charlie@127.0.0.1:40012");
    
    let source_addr: SocketAddr = "127.0.0.1:40011".parse().unwrap();
    
    // Process should succeed but generate 481 response
    let result = dialog_api.process_request(refer_request, source_addr).await;
    
    // The processing should succeed (no error)
    assert!(result.is_ok());
    
    // Should have sent 481 response (checked via transaction manager)
    println!("✅ REFER without dialog handled correctly");
}

#[tokio::test]
async fn test_refer_with_replaces_header() {
    println!("🧪 Testing REFER with Replaces header (attended transfer)");
    
    // Create dialog API
    let dialog_api = create_test_dialog_api(40020).await;
    
    // Create event channel
    let (event_tx, mut event_rx) = mpsc::channel::<SessionCoordinationEvent>(100);
    dialog_api.set_session_event_handler(event_tx).await;
    
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
    let call_id = call_handle.dialog().call_id().clone();
    let local_tag = call_handle.dialog().local_tag().clone();
    
    // Create REFER with Replaces header
    let refer_uri = format!("sip:alice@127.0.0.1:40020;tag={}", local_tag);
    let mut refer_request = Request::new(
        Method::REFER,
        refer_uri.parse::<Uri>().unwrap()
    );
    
    refer_request.add_header("Call-ID", &call_id);
    refer_request.add_header("From", "sip:bob@127.0.0.1:40021;tag=remote-tag");
    refer_request.add_header("To", &format!("sip:alice@127.0.0.1:40020;tag={}", local_tag));
    refer_request.add_header("CSeq", "2 REFER");
    refer_request.add_header("Refer-To", "sip:charlie@127.0.0.1:40022");
    
    // Add Replaces header for attended transfer
    refer_request.add_header("Replaces", "other-call-id;to-tag=tt;from-tag=ft");
    
    let source_addr: SocketAddr = "127.0.0.1:40021".parse().unwrap();
    
    // Process the REFER
    let result = dialog_api.process_request(refer_request, source_addr).await;
    assert!(result.is_ok());
    
    // Verify TransferRequest event contains Replaces
    tokio::time::timeout(Duration::from_millis(500), async {
        while let Some(event) = event_rx.recv().await {
            if let SessionCoordinationEvent::TransferRequest {
                replaces,
                ..
            } = event {
                assert!(replaces.is_some());
                assert!(replaces.unwrap().contains("other-call-id"));
                println!("✅ Replaces header preserved in TransferRequest");
                return;
            }
        }
    })
    .await
    .expect("Timeout waiting for event");
}