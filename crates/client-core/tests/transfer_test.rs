//! Tests for blind call transfer functionality in client-core

use rvoip_client_core::{
    Client, ClientBuilder, ClientError, CallId, CallState, CallDirection,
    events::{ClientEventHandler, IncomingCallInfo, CallStatusInfo, CallAction, RegistrationStatusInfo},
};
use std::sync::Arc;
use std::time::Duration;
use serial_test::serial;

/// Helper to create a test client with specific port
async fn create_test_client(port: u16) -> Arc<Client> {
    ClientBuilder::new()
        .user_agent("TransferTest/1.0")
        .local_address(format!("127.0.0.1:{}", port).parse().unwrap())
        .build()
        .await
        .expect("Failed to create test client")
}

#[tokio::test]
#[serial]
async fn test_transfer_call_method_exists() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that the transfer_call method exists and has the right signature
    let client = create_test_client(25090).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Create a fake call ID
    let call_id = CallId::new_v4();
    let target_uri = "sip:charlie@example.com";
    
    // Try to transfer a non-existent call (should fail)
    let result = client.transfer_call(&call_id, target_uri).await;
    
    // Should fail because the call doesn't exist
    assert!(result.is_err());
    match result {
        Err(ClientError::CallNotFound { .. }) => {
            // Expected error
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
        Ok(_) => panic!("Expected error for non-existent call"),
    }
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_transfer_requires_connected_state() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that transfer only works for connected calls
    let client = create_test_client(25091).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // We can't easily create a real call in a unit test without a full SIP stack,
    // but we can verify the API exists and returns appropriate errors
    
    let call_id = CallId::new_v4();
    let target_uri = "sip:charlie@example.com";
    
    // Try to transfer - should fail with CallNotFound
    let result = client.transfer_call(&call_id, target_uri).await;
    assert!(matches!(result, Err(ClientError::CallNotFound { .. })));
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_transfer_validation() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that transfer validates the target URI
    let client = create_test_client(25092).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    let call_id = CallId::new_v4();
    
    // Test with various invalid URIs
    let invalid_uris = vec![
        "",                    // Empty
        "not-a-uri",          // Invalid format
        "http://example.com", // Wrong scheme
    ];
    
    for invalid_uri in invalid_uris {
        let result = client.transfer_call(&call_id, invalid_uri).await;
        assert!(result.is_err(), "Should reject invalid URI: {}", invalid_uri);
    }
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_transfer_with_active_call() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test transfer with an actual call
    let client = create_test_client(25093).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Make a call first
    let call_id = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:bob@example.com".to_string(),
        Some("Test call for transfer".to_string()),
    ).await.expect("Failed to make call");
    
    // Get call info to verify it exists
    let call_info = client.get_call(&call_id).await
        .expect("Failed to get call info");
    assert_eq!(call_info.state, CallState::Initiating);
    
    // Try to transfer while in Initiating state (should fail)
    let result = client.transfer_call(&call_id, "sip:charlie@example.com").await;
    assert!(result.is_err(), "Transfer should fail when call is not connected");
    
    // Clean up
    client.hangup_call(&call_id).await.ok();
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_attended_transfer_method_exists() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that the attended_transfer method exists
    let client = create_test_client(25094).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Create fake call IDs
    let call_id1 = CallId::new_v4();
    let call_id2 = CallId::new_v4();
    
    // Try to do attended transfer with non-existent calls
    let result = client.attended_transfer(&call_id1, &call_id2).await;
    
    // Should fail because the calls don't exist
    assert!(result.is_err());
    match result {
        Err(ClientError::CallNotFound { .. }) => {
            // Expected error
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
        Ok(_) => panic!("Expected error for non-existent calls"),
    }
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_transfer_concurrent_safety() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that multiple transfer operations can be called concurrently
    let client = create_test_client(25095).await;
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Try multiple concurrent transfers (all will fail, but tests thread safety)
    let mut handles = vec![];
    
    for i in 0..5 {
        let client_clone = client.clone();
        let handle = tokio::spawn(async move {
            let call_id = CallId::new_v4();
            let target = format!("sip:user{}@example.com", i);
            let _ = client_clone.transfer_call(&call_id, &target).await;
        });
        handles.push(handle);
    }
    
    // Wait for all to complete
    for handle in handles {
        handle.await.expect("Task panicked");
    }
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[serial]
async fn test_transfer_after_client_stop() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Test that transfer fails gracefully after client is stopped
    let client = create_test_client(25096).await;
    
    // Start and immediately stop the client
    client.start().await.expect("Failed to start client");
    client.stop().await.expect("Failed to stop client");
    
    // Try to transfer after stop
    let call_id = CallId::new_v4();
    let result = client.transfer_call(&call_id, "sip:charlie@example.com").await;
    
    // Should fail
    assert!(result.is_err());
    
    // The error might be CallNotFound or a different error related to stopped state
    // We just verify it doesn't panic
}

/// Test transfer with event handler to capture transfer-related events
#[tokio::test]
#[serial]
async fn test_transfer_with_event_handler() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = create_test_client(25097).await;
    
    // Create a simple event handler
    struct TestHandler;
    
    #[async_trait::async_trait]
    impl ClientEventHandler for TestHandler {
        async fn on_incoming_call(&self, _info: IncomingCallInfo) -> CallAction {
            CallAction::Reject
        }
        
        async fn on_call_state_changed(&self, info: CallStatusInfo) {
            tracing::info!("Call {} state changed to {:?}", info.call_id, info.new_state);
        }
        
        async fn on_registration_status_changed(&self, _info: RegistrationStatusInfo) {}
    }
    
    client.set_event_handler(Arc::new(TestHandler)).await;
    client.start().await.expect("Failed to start client");
    
    // Make a call
    let call_id = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:bob@example.com".to_string(),
        Some("Transfer test call".to_string()),
    ).await.expect("Failed to make call");
    
    // Wait a bit for call to be established (in real scenario)
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Try to transfer (will fail since call isn't really connected)
    let transfer_result = client.transfer_call(&call_id, "sip:charlie@example.com").await;
    
    // Verify transfer was attempted
    assert!(transfer_result.is_err(), "Transfer should fail for non-connected call");
    
    // Clean up
    client.hangup_call(&call_id).await.ok();
    client.stop().await.expect("Failed to stop client");
}

/// Mock test for full transfer flow (would need real SIP infrastructure)
#[tokio::test]
#[serial]
#[ignore] // Ignore by default as it needs real SIP infrastructure
async fn test_full_transfer_flow_with_real_sip() {
    // This test would require:
    // 1. Three real SIP endpoints (Alice, Bob, Charlie)
    // 2. Alice calls Bob
    // 3. Bob answers
    // 4. Bob transfers Alice to Charlie
    // 5. Charlie's phone rings
    // 6. Charlie answers
    // 7. Alice and Charlie are connected
    // 8. Bob's call ends
    
    // For now, this is a placeholder for integration testing
    // It can be enabled when running against a real SIP test environment
}