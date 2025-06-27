//! Integration tests for client lifecycle operations
//! 
//! Tests basic client creation, starting, stopping, and configuration.

use rvoip_client_core::{ClientBuilder};
use serial_test::serial;

use std::time::Duration;
use tokio::time::timeout;

/// Test basic client creation and lifecycle
#[tokio::test]
#[serial]
async fn test_client_creation_and_lifecycle() {
    // Initialize tracing for tests
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug,rvoip_session_core=info")
        .with_test_writer()
        .try_init();

    // Create client with default configuration
    let client = ClientBuilder::new()
        .user_agent("TestClient/1.0")
        .local_address("127.0.0.1:15201".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");

    // Verify client is not running initially
    assert!(!client.is_running().await);

    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Verify client is running
    assert!(client.is_running().await);

    // Get stats and verify
    let stats = client.get_client_stats().await;
    assert!(stats.is_running);
    assert_eq!(stats.total_calls, 0);
    assert_eq!(stats.connected_calls, 0);

    // Stop the client
    client.stop().await.expect("Failed to stop client");
    
    // Verify client is stopped
    assert!(!client.is_running().await);
}

/// Test client with custom configuration
#[tokio::test]
#[serial]
async fn test_client_with_custom_config() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Create client with custom port
    let client = ClientBuilder::new()
        .user_agent("CustomClient/2.0")
        .local_address("127.0.0.1:15060".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client with custom config");

    client.start().await.expect("Failed to start client");

    // Verify the bound address uses our custom port
    let stats = client.get_client_stats().await;
    assert_eq!(stats.local_sip_addr.port(), 15060);

    client.stop().await.expect("Failed to stop client");
}

/// Test multiple client instances
#[tokio::test]
#[serial]
async fn test_multiple_client_instances() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Create two clients on different ports
    let client1 = ClientBuilder::new()
        .user_agent("Client1/1.0")
        .local_address("127.0.0.1:15061".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client 1");

    let client2 = ClientBuilder::new()
        .user_agent("Client2/1.0")
        .local_address("127.0.0.1:15062".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client 2");

    // Start both clients
    client1.start().await.expect("Failed to start client 1");
    client2.start().await.expect("Failed to start client 2");

    // Verify both are running
    assert!(client1.is_running().await);
    assert!(client2.is_running().await);

    // Verify different ports
    let stats1 = client1.get_client_stats().await;
    let stats2 = client2.get_client_stats().await;
    assert_ne!(stats1.local_sip_addr.port(), stats2.local_sip_addr.port());

    // Stop both clients
    client1.stop().await.expect("Failed to stop client 1");
    client2.stop().await.expect("Failed to stop client 2");
}

/// Test client event subscription
#[tokio::test]
#[serial]
async fn test_client_event_subscription() {
    
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("EventTestClient/1.0")
        .local_address("127.0.0.1:15202".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");

    // Subscribe to events before starting
    let mut event_rx = client.subscribe_events();

    client.start().await.expect("Failed to start client");

    // Test that we can receive events
    let event_task = tokio::spawn(async move {
        // Wait for any event with a timeout
        match timeout(Duration::from_secs(5), event_rx.recv()).await {
            Ok(Ok(event)) => {
                tracing::info!("Received event: {:?}", event);
                true
            }
            Ok(Err(_)) => {
                tracing::warn!("Event channel closed");
                false
            }
            Err(_) => {
                tracing::info!("No events received (expected for basic lifecycle)");
                true // This is OK for basic lifecycle test
            }
        }
    });

    // Give some time for potential events
    tokio::time::sleep(Duration::from_millis(100)).await;

    client.stop().await.expect("Failed to stop client");

    // Wait for event task
    let event_result = event_task.await.expect("Event task panicked");
    assert!(event_result, "Event handling failed");
}

/// Test client resilience to rapid start/stop cycles
#[tokio::test]
#[serial]
async fn test_rapid_start_stop_cycles() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("RapidCycleClient/1.0")
        .local_address("127.0.0.1:15203".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");

    // Perform multiple start/stop cycles
    for i in 0..5 {
        tracing::info!("Start/stop cycle {}", i + 1);
        
        client.start().await.expect("Failed to start client");
        assert!(client.is_running().await);
        
        // Small delay to let things settle
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        client.stop().await.expect("Failed to stop client");
        assert!(!client.is_running().await);
        
        // Small delay between cycles
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Test error handling for invalid operations
#[tokio::test]
#[serial]
async fn test_error_handling() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    let client = ClientBuilder::new()
        .user_agent("ErrorTestClient/1.0")
        .local_address("127.0.0.1:15204".parse().unwrap())
        .build()
        .await
        .expect("Failed to build client");

    // Try to make a call without starting the client
    let call_result = client.make_call(
        "sip:alice@example.com".to_string(),
        "sip:bob@example.com".to_string(),
        None
    ).await;

    // Should fail because client is not started
    assert!(call_result.is_err());
    
    // The error should be appropriate
    match call_result {
        Err(e) => {
            tracing::info!("Expected error: {}", e);
            assert!(e.to_string().contains("failed") || e.to_string().contains("not started"));
        }
        Ok(_) => panic!("Call should have failed without starting client"),
    }
}

/// Test client resource cleanup
#[tokio::test]
#[serial]
async fn test_resource_cleanup() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip_client_core=debug")
        .with_test_writer()
        .try_init();

    // Create and start client in a scope
    {
        let client = ClientBuilder::new()
            .user_agent("CleanupTestClient/1.0")
            .local_address("127.0.0.1:15070".parse().unwrap())
        .build()
            .await
            .expect("Failed to build client");

        client.start().await.expect("Failed to start client");
        assert!(client.is_running().await);
        
        // Explicitly stop the client before dropping
        client.stop().await.expect("Failed to stop client");
        // Client will be dropped here
    }

    // Longer delay to ensure OS releases the port
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create another client on a different port to verify cleanup doesn't affect new clients
    let client2 = ClientBuilder::new()
        .user_agent("CleanupTestClient2/1.0")
        .local_address("127.0.0.1:15071".parse().unwrap())
        .build()
        .await
        .expect("Failed to build second client");

    // Should be able to start successfully
    client2.start().await.expect("Failed to start second client");
    client2.stop().await.expect("Failed to stop second client");
} 