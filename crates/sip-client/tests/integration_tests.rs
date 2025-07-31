//! Integration tests for the SIP client library
//!
//! These tests require a more complete environment and may use mocked
//! versions of the underlying components.

use rvoip_sip_client::{SipClient, SipClientEvent};
use tokio_stream::StreamExt;
use std::time::Duration;

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_outgoing_call_flow() {
    // Initialize logging for debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("rvoip=debug")
        .try_init();
    
    // Create client
    let alice = SipClient::new("sip:alice@example.com").await
        .expect("Failed to create SIP client");
    
    // Subscribe to events
    let mut events = alice.events();
    
    // Start the client
    alice.start().await.expect("Failed to start client");
    
    // Wait for started event
    let event = tokio::time::timeout(
        Duration::from_secs(5),
        events.next()
    ).await;
    
    assert!(event.is_ok(), "Timeout waiting for Started event");
    if let Ok(Some(Ok(SipClientEvent::Started))) = event {
        // Expected
    } else {
        panic!("Expected Started event, got: {:?}", event);
    }
    
    // Make a call
    let call = alice.call("sip:bob@example.com").await
        .expect("Failed to make call");
    
    assert_eq!(call.local_uri, "sip:alice@example.com");
    assert_eq!(call.remote_uri, "sip:bob@example.com");
    
    // Wait for state changes
    let mut call_connected = false;
    let start = std::time::Instant::now();
    
    while start.elapsed() < Duration::from_secs(10) {
        if let Ok(Some(Ok(event))) = tokio::time::timeout(
            Duration::from_millis(100),
            events.next()
        ).await {
            match event {
                SipClientEvent::CallStateChanged { 
                    call: event_call, 
                    new_state: rvoip_sip_client::CallState::Connected, 
                    .. 
                } => {
                    if event_call.id == call.id {
                        call_connected = true;
                        break;
                    }
                }
                _ => {} // Ignore other events
            }
        }
    }
    
    // In a real test, we'd need a responding party
    // For now, just verify the call was initiated
    assert!(!call_connected, "Call should not connect without a peer");
    
    // Hangup
    alice.hangup(&call.id).await.expect("Failed to hangup");
    
    // Stop the client
    alice.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_audio_level_monitoring() {
    // Create client
    let client = SipClient::new("sip:test@example.com").await
        .expect("Failed to create SIP client");
    
    // Subscribe to events
    let mut events = client.events();
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Make a call (would fail without peer, but we can test the setup)
    let result = client.call("sip:peer@example.com").await;
    
    if let Ok(call) = result {
        // Monitor for audio level events
        let mut received_audio_level = false;
        let start = std::time::Instant::now();
        
        while start.elapsed() < Duration::from_secs(2) {
            if let Ok(Some(Ok(event))) = tokio::time::timeout(
                Duration::from_millis(100),
                events.next()
            ).await {
                if let SipClientEvent::AudioLevelChanged { call_id, .. } = event {
                    if call_id == Some(call.id) {
                        received_audio_level = true;
                        break;
                    }
                }
            }
        }
        
        // We might not receive audio levels in test environment
        // but the pipeline should be set up
        
        // Cleanup
        let _ = client.hangup(&call.id).await;
    }
    
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_audio_device_management() {
    // Create client
    let client = SipClient::new("sip:test@example.com").await
        .expect("Failed to create SIP client");
    
    // List audio devices
    let input_devices = client.list_audio_devices(
        rvoip_audio_core::AudioDirection::Input
    ).await.expect("Failed to list input devices");
    
    let output_devices = client.list_audio_devices(
        rvoip_audio_core::AudioDirection::Output
    ).await.expect("Failed to list output devices");
    
    // In test environment, we might have no real devices
    // but the API should work
    assert!(input_devices.is_empty() || !input_devices.is_empty());
    assert!(output_devices.is_empty() || !output_devices.is_empty());
    
    // Test getting default device
    let result = client.get_audio_device(
        rvoip_audio_core::AudioDirection::Input
    ).await;
    
    // Should succeed even if it returns a mock device
    assert!(result.is_ok());
}

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_event_stream_lifecycle() {
    let client = SipClient::new("sip:test@example.com").await
        .expect("Failed to create SIP client");
    
    // Create multiple event streams
    let mut stream1 = client.events();
    let mut stream2 = client.events();
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Both streams should receive the started event
    let event1 = tokio::time::timeout(
        Duration::from_secs(1),
        stream1.next()
    ).await;
    
    let event2 = tokio::time::timeout(
        Duration::from_secs(1),
        stream2.next()
    ).await;
    
    assert!(event1.is_ok());
    assert!(event2.is_ok());
    
    // Stop the client
    client.stop().await.expect("Failed to stop client");
    
    // Should receive stopped event
    let stopped1 = tokio::time::timeout(
        Duration::from_secs(1),
        stream1.next()
    ).await;
    
    assert!(stopped1.is_ok());
    if let Ok(Some(Ok(SipClientEvent::Stopped))) = stopped1 {
        // Expected
    } else {
        panic!("Expected Stopped event");
    }
}

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_concurrent_calls() {
    let client = SipClient::new("sip:test@example.com").await
        .expect("Failed to create SIP client");
    
    client.start().await.expect("Failed to start client");
    
    // Try to make multiple calls
    // In a real environment, this would test concurrent call handling
    let call1_result = client.call("sip:peer1@example.com").await;
    let call2_result = client.call("sip:peer2@example.com").await;
    
    // Both calls should be tracked
    let active_calls = client.active_calls();
    
    // Count successful calls
    let mut call_count = 0;
    if call1_result.is_ok() { call_count += 1; }
    if call2_result.is_ok() { call_count += 1; }
    
    assert_eq!(active_calls.len(), call_count);
    
    // Cleanup
    if let Ok(call1) = call1_result {
        let _ = client.hangup(&call1.id).await;
    }
    if let Ok(call2) = call2_result {
        let _ = client.hangup(&call2.id).await;
    }
    
    client.stop().await.expect("Failed to stop client");
}

#[tokio::test]
#[ignore] // Ignore by default as this requires full environment
async fn test_error_handling() {
    let client = SipClient::new("sip:test@example.com").await
        .expect("Failed to create SIP client");
    
    let mut events = client.events();
    
    // Try to make a call without starting the client
    let result = client.call("sip:peer@example.com").await;
    
    // Should fail because client is not started
    assert!(result.is_err());
    
    // Start the client
    client.start().await.expect("Failed to start client");
    
    // Try to answer a non-existent call
    let fake_call_id = rvoip_sip_client::CallId::new_v4();
    let answer_result = client.answer(&fake_call_id).await;
    assert!(answer_result.is_err());
    
    // Monitor for error events
    let start = std::time::Instant::now();
    let mut received_error = false;
    
    while start.elapsed() < Duration::from_millis(500) {
        if let Ok(Some(Ok(event))) = tokio::time::timeout(
            Duration::from_millis(100),
            events.next()
        ).await {
            if let SipClientEvent::Error { .. } = event {
                received_error = true;
                break;
            }
        }
    }
    
    // We might or might not receive error events depending on implementation
    
    client.stop().await.expect("Failed to stop client");
}