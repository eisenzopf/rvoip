//! Test to verify session coordination between dialog-core and session-core

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use rvoip_session_core::{
    SessionManager,
    api::{
        handlers::CallHandler,
        builder::SessionManagerBuilder,
        types::{IncomingCall, CallSession, CallDecision},
    },
};

/// Simple handler for testing
#[derive(Debug)]
struct TestHandler;

#[async_trait::async_trait]
impl CallHandler for TestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("🔔 TEST HANDLER: Received incoming call {}", call.id);
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("☎️ TEST HANDLER: Call ended {} ({})", call.id(), reason);
    }
}

#[tokio::test]
async fn test_session_coordination_setup() {
    println!("🧪 Testing session coordination setup...");

    // Create a session manager
    let manager = SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(7000)
        .with_from_uri("sip:test@localhost")
        .with_handler(Arc::new(TestHandler))
        .build()
        .await
        .expect("Failed to build session manager");

    // Start the manager
    manager.start().await.expect("Failed to start manager");
    
    println!("✅ Session manager created and started");
    
    // Wait a moment for initialization
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check that we can get basic info
    let addr = manager.get_bound_address();
    println!("📍 Manager bound to: {}", addr);
    
    let stats = manager.get_stats().await.expect("Failed to get stats");
    println!("📊 Initial stats: {} active sessions", stats.active_sessions);
    
    // Try to create an outgoing call (this should work)
    let call_result = manager.create_outgoing_call(
        "sip:test@127.0.0.1",
        "sip:remote@example.com",
        Some("v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\n".to_string())
    ).await;
    
    match call_result {
        Ok(call) => {
            println!("✅ Outgoing call created: {} -> {} (state: {:?})", 
                     call.from, call.to, call.state());
        },
        Err(e) => {
            println!("❌ Failed to create outgoing call: {}", e);
        }
    }
    
    // Check stats again
    let stats = manager.get_stats().await.expect("Failed to get stats");
    println!("📊 Final stats: {} active sessions", stats.active_sessions);
    
    // Clean up
    manager.stop().await.expect("Failed to stop manager");
    println!("🏁 Test completed");
} 