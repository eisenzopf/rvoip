//! Minimal test to verify the transfer API is called

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;

use rvoip_session_core::{
    SessionManagerBuilder,
    SessionControl,
    api::{
        types::{IncomingCall, CallSession, CallDecision, SessionId},
        handlers::CallHandler,
    },
};

#[derive(Debug)]
struct TestHandler {
    name: String,
}

impl TestHandler {
    fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

#[async_trait]
impl CallHandler for TestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("   {} received call", self.name);
        CallDecision::Accept(None)
    }
    
    async fn on_call_ended(&self, _: CallSession, reason: &str) {
        println!("   {} call ended: {}", self.name, reason);
    }
    
    async fn on_incoming_transfer_request(&self, id: &SessionId, target: &str, _: Option<&str>) -> bool {
        println!("   {} received transfer to {}", self.name, target);
        true
    }
}

#[tokio::test]
async fn test_transfer_api_called() {
    println!("\n🧪 Testing if transfer API is invoked\n");
    
    // Create Alice and Bob
    let alice = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@127.0.0.1:5060")
        .with_handler(Arc::new(TestHandler::new("Alice")))
        .build()
        .await
        .expect("Failed to create Alice");
    
    let bob = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:bob@127.0.0.1:5061")
        .with_handler(Arc::new(TestHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Alice calls Bob
    println!("Creating call...");
    let call = alice.create_outgoing_call(
        "sip:alice@127.0.0.1:5060",
        "sip:bob@127.0.0.1:5061",
        None,
    ).await.expect("Failed to create call");
    
    println!("Call ID: {}", call.id);
    
    // Wait a bit for call to be in progress
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Try to transfer even if not fully established
    println!("\n📞 Attempting transfer (may fail if not Active)...");
    match alice.transfer_session(&call.id, "sip:charlie@127.0.0.1:5062").await {
        Ok(_) => println!("✅ Transfer API call succeeded"),
        Err(e) => println!("❌ Transfer API call failed: {}", e),
    }
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    alice.stop().await.ok();
    bob.stop().await.ok();
}