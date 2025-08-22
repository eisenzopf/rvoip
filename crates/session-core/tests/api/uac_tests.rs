//! UAC API Tests

use rvoip_session_core::api::uac::{SimpleUacClient, UacClient, UacBuilder, UacEventHandler};
use rvoip_session_core::api::types::{SessionId, CallState};
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::Mutex;
use serial_test::serial;

/// Test event handler that records events
#[derive(Debug, Default)]
struct TestEventHandler {
    events: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl UacEventHandler for TestEventHandler {
    async fn on_call_state_changed(&self, session_id: SessionId, _old_state: CallState, new_state: CallState) {
        let mut events = self.events.lock().await;
        events.push(format!("Call {} state: {:?}", session_id, new_state));
    }
    
    async fn on_registration_state_changed(&self, registered: bool, reason: Option<String>) {
        let mut events = self.events.lock().await;
        events.push(format!("Registration: {} ({:?})", registered, reason));
    }
    
    async fn on_media_established(&self, session_id: SessionId) {
        let mut events = self.events.lock().await;
        events.push(format!("Media established: {}", session_id));
    }
    
    async fn on_dtmf_received(&self, session_id: SessionId, digit: char) {
        let mut events = self.events.lock().await;
        events.push(format!("DTMF {} received: {}", session_id, digit));
    }
    
    async fn on_quality_update(&self, session_id: SessionId, mos_score: f32) {
        let mut events = self.events.lock().await;
        events.push(format!("Quality {}: {}", session_id, mos_score));
    }
}

#[tokio::test]
#[serial]
async fn test_simple_uac_client_creation() {
    // Test creating a simple UAC client - using unique port 16001
    let result = SimpleUacClient::new(
        "sip:alice@example.com",
        "127.0.0.1:16001"
    ).await;
    
    assert!(result.is_ok(), "Failed to create SimpleUacClient: {:?}", result.err());
    
    let client = result.unwrap();
    
    // Test that we can access the coordinator
    let _coordinator = client.coordinator();
    
    // Clean shutdown
    let shutdown_result = client.shutdown().await;
    assert!(shutdown_result.is_ok(), "Failed to shutdown: {:?}", shutdown_result.err());
}

#[tokio::test]
#[serial]
async fn test_uac_builder() {
    // Test the builder pattern - using unique port 16002
    let result = UacBuilder::new("sip:bob@example.com")
        .server("127.0.0.1:16002")
        .local_addr("0.0.0.0:16003")
        .user_agent("TestUA/1.0")
        .call_timeout(60)
        .build()
        .await;
    
    assert!(result.is_ok(), "Failed to build UacClient: {:?}", result.err());
    
    let client = result.unwrap();
    
    // Verify configuration was applied
    let config = client.config();
    assert_eq!(config.identity, "sip:bob@example.com");
    assert_eq!(config.server_addr, "127.0.0.1:16002");
    assert_eq!(config.user_agent, "TestUA/1.0");
    assert_eq!(config.call_timeout, 60);
    
    // Clean shutdown
    let _ = client.shutdown().await;
}

#[tokio::test]
#[serial]
async fn test_uac_with_event_handler() {
    // Create a test event handler
    let handler = Arc::new(TestEventHandler::default());
    let events_clone = handler.events.clone();
    
    // Create UAC with event handler - using unique port 16004
    let config = rvoip_session_core::api::uac::UacConfig {
        identity: "sip:charlie@example.com".to_string(),
        server_addr: "127.0.0.1:16004".to_string(),
        local_addr: "0.0.0.0:16005".to_string(),
        ..Default::default()
    };
    
    let result = UacClient::new_with_handler(config, handler).await;
    assert!(result.is_ok(), "Failed to create UacClient with handler: {:?}", result.err());
    
    let client = result.unwrap();
    
    // Trigger some events
    let _ = client.register().await;
    
    // Give some time for async events
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check that events were recorded
    let events = events_clone.lock().await;
    assert!(!events.is_empty(), "No events were recorded");
    assert!(events.iter().any(|e| e.contains("Registration")), "Registration event not found");
    
    // Clean shutdown
    let _ = client.shutdown().await;
}

#[tokio::test]
async fn test_uac_builder_validation() {
    // Test that builder validates required fields
    let result = UacBuilder::new("")
        .build()
        .await;
    
    assert!(result.is_err(), "Builder should fail with empty identity");
}

#[tokio::test]
#[serial]
async fn test_uac_call_operations() {
    // Create a simple client - using unique port 16006
    let client = SimpleUacClient::new(
        "sip:test@example.com",
        "127.0.0.1:16006"
    ).await.expect("Failed to create client");
    
    // Note: This will fail to actually connect since there's no server
    // but we can test that the API works
    let call_result = client.call("sip:dest@example.com").await;
    
    // The call might fail due to no server, but the API should work
    if let Ok(call) = call_result {
        // Test call operations
        assert_eq!(call.remote_uri(), "sip:dest@example.com");
        assert!(!call.session_id().to_string().is_empty());
        
        // Test various operations (they may fail but API should be callable)
        let _ = call.mute().await;
        let _ = call.unmute().await;
        let _ = call.hold().await;
        let _ = call.unhold().await;
        let _ = call.send_dtmf('1').await;
        let _ = call.get_quality_score().await;
        let _ = call.hangup().await;
    }
    
    let _ = client.shutdown().await;
}