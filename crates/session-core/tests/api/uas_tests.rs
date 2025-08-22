//! UAS API Tests

use rvoip_session_core::api::uas::{
    SimpleUasServer, UasServer, UasBuilder, 
    UasCallHandler, UasCallDecision, CallController
};
use rvoip_session_core::api::types::{SessionId, CallState, IncomingCall, CallSession};
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::Mutex;
use serial_test::serial;

/// Test call handler that logs all calls
#[derive(Debug, Default)]
struct TestCallHandler {
    calls_received: Arc<Mutex<Vec<String>>>,
    auto_accept: bool,
}

#[async_trait]
impl UasCallHandler for TestCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> UasCallDecision {
        let mut calls = self.calls_received.lock().await;
        calls.push(format!("Incoming: {} -> {}", call.from, call.to));
        
        if self.auto_accept {
            UasCallDecision::Accept(None)
        } else {
            UasCallDecision::Defer
        }
    }
    
    async fn on_call_established(&self, session: CallSession) {
        let mut calls = self.calls_received.lock().await;
        calls.push(format!("Established: {}", session.id));
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: String) {
        let mut calls = self.calls_received.lock().await;
        calls.push(format!("Ended: {} - {}", session.id, reason));
    }
    
    async fn on_dtmf_received(&self, session_id: SessionId, digit: char) {
        let mut calls = self.calls_received.lock().await;
        calls.push(format!("DTMF: {} - {}", session_id, digit));
    }
    
    async fn on_quality_update(&self, session_id: SessionId, mos_score: f32) {
        let mut calls = self.calls_received.lock().await;
        calls.push(format!("Quality: {} - {}", session_id, mos_score));
    }
}

/// Test controller for advanced features
#[derive(Debug, Default)]
struct TestController {
    pre_invite_called: Arc<Mutex<bool>>,
}

#[async_trait]
impl CallController for TestController {
    async fn pre_invite(&self, _call: &mut IncomingCall) -> bool {
        *self.pre_invite_called.lock().await = true;
        true // Allow the call
    }
    
    async fn post_decision(&self, _call: &IncomingCall, _decision: &mut UasCallDecision) {}
    
    async fn on_in_dialog_request(&self, _session_id: SessionId, _method: String, _body: Option<String>) {}
    
    async fn manipulate_sdp(&self, _sdp: &mut String, _is_offer: bool) {}
    
    async fn inject_headers(&self, _headers: &mut Vec<(String, String)>) {}
}

#[tokio::test]
#[serial]
async fn test_simple_uas_always_accept() {
    // Test the always-accept server
    let result = SimpleUasServer::always_accept("127.0.0.1:15060").await;
    
    assert!(result.is_ok(), "Failed to create always_accept server: {:?}", result.err());
    
    let server = result.unwrap();
    
    // Check that we can get active calls count
    let count = server.active_calls().await;
    assert!(count.is_ok());
    assert_eq!(count.unwrap(), 0, "Should have no active calls initially");
    
    // Clean shutdown
    let shutdown_result = server.shutdown().await;
    assert!(shutdown_result.is_ok(), "Failed to shutdown: {:?}", shutdown_result.err());
}

#[tokio::test]
#[serial]
async fn test_simple_uas_always_reject() {
    // Test the always-reject server
    let result = SimpleUasServer::always_reject(
        "127.0.0.1:15061",
        "Server in maintenance mode".to_string()
    ).await;
    
    assert!(result.is_ok(), "Failed to create always_reject server: {:?}", result.err());
    
    let server = result.unwrap();
    
    // Clean shutdown
    let _ = server.shutdown().await;
}

#[tokio::test]
#[serial]
async fn test_simple_uas_always_forward() {
    // Test the always-forward server
    let result = SimpleUasServer::always_forward(
        "127.0.0.1:15062",
        "sip:voicemail@example.com".to_string()
    ).await;
    
    assert!(result.is_ok(), "Failed to create always_forward server: {:?}", result.err());
    
    let server = result.unwrap();
    
    // Clean shutdown
    let _ = server.shutdown().await;
}

#[tokio::test]
#[serial]
async fn test_uas_builder() {
    // Test the UAS builder
    let handler = Arc::new(TestCallHandler::default());
    
    let result = UasBuilder::new("127.0.0.1:15063")
        .identity("sip:server@example.com")
        .user_agent("TestUAS/1.0")
        .max_concurrent_calls(100)
        .call_timeout(300)
        .handler(handler)
        .build()
        .await;
    
    assert!(result.is_ok(), "Failed to build UAS server: {:?}", result.err());
    
    let server = result.unwrap();
    
    // Verify server was created
    assert_eq!(server.pending_count().await, 0);
    
    // Clean shutdown
    let _ = server.shutdown().await;
}

#[tokio::test]
#[serial]
async fn test_uas_with_controller() {
    // Test UAS with both handler and controller
    let handler = Arc::new(TestCallHandler {
        calls_received: Arc::new(Mutex::new(Vec::new())),
        auto_accept: false,
    });
    
    let controller = Arc::new(TestController::default());
    let pre_invite_flag = controller.pre_invite_called.clone();
    
    let config = rvoip_session_core::api::uas::UasConfig {
        local_addr: "127.0.0.1:15064".to_string(),
        identity: "sip:advanced@example.com".to_string(),
        ..Default::default()
    };
    
    let result = UasServer::new_with_controller(config, handler, controller).await;
    assert!(result.is_ok(), "Failed to create UAS with controller: {:?}", result.err());
    
    let server = result.unwrap();
    
    // Server is running but no calls yet
    assert_eq!(server.pending_count().await, 0);
    assert_eq!(server.get_active_calls().await.len(), 0);
    
    // Controller pre_invite should not be called yet
    assert!(!*pre_invite_flag.lock().await);
    
    // Clean shutdown
    let _ = server.shutdown().await;
}

#[tokio::test]
#[serial]
async fn test_uas_pending_call_management() {
    // Test pending call operations
    let handler = Arc::new(TestCallHandler {
        calls_received: Arc::new(Mutex::new(Vec::new())),
        auto_accept: false, // Defer all calls
    });
    
    let config = rvoip_session_core::api::uas::UasConfig {
        local_addr: "127.0.0.1:15065".to_string(),
        ..Default::default()
    };
    
    let server = UasServer::new(config, handler).await
        .expect("Failed to create UAS server");
    
    // Initially no pending calls
    assert_eq!(server.pending_count().await, 0);
    
    // Process pending (should be no-op with no calls)
    let result = server.process_pending_calls().await;
    assert!(result.is_ok());
    
    // Note: We can't easily test actual call handling without a real SIP stack
    // but we've verified the API works
    
    // Clean shutdown
    let _ = server.shutdown().await;
}