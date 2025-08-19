//! Test Bob calling Charlie directly (no transfer)

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;

use rvoip_session_core::{
    SessionManagerBuilder,
    api::{
        types::{IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
    },
};

#[derive(Debug)]
struct SimpleHandler {
    name: String,
}

impl SimpleHandler {
    fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

#[async_trait]
impl CallHandler for SimpleHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        println!("   {} received call from: {}", self.name, call.from);
        CallDecision::Accept(None)
    }
    
    async fn on_call_established(&self, _: CallSession, _: Option<String>, _: Option<String>) {
        println!("   {} call established", self.name);
    }
    
    async fn on_call_ended(&self, _: CallSession, reason: &str) {
        println!("   {} call ended: {}", self.name, reason);
    }
}

#[tokio::test]
async fn test_bob_calls_charlie() {
    println!("\n🧪 Testing Bob -> Charlie call directly\n");
    
    // Create Bob
    println!("Creating Bob...");
    let bob = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:bob@127.0.0.1:5061")
        .with_handler(Arc::new(SimpleHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    
    // Create Charlie
    println!("Creating Charlie...");
    let charlie = SessionManagerBuilder::new()
        .with_sip_port(5062)
        .with_local_address("sip:charlie@127.0.0.1:5062")
        .with_handler(Arc::new(SimpleHandler::new("Charlie")))
        .build()
        .await
        .expect("Failed to create Charlie");
    
    // Wait for initialization
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Bob calls Charlie
    println!("\n📞 Bob calling Charlie...");
    let call = bob.create_outgoing_call(
        "sip:bob@127.0.0.1:5061",
        "sip:charlie@127.0.0.1:5062",
        None,
    ).await.expect("Failed to create call");
    
    println!("Call created with ID: {}", call.id);
    
    // Wait for call to establish
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    println!("✅ Test complete");
    
    bob.stop().await.ok();
    charlie.stop().await.ok();
}