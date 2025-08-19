//! Simple test to verify basic calling works before testing transfer

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
async fn test_simple_call() {
    println!("\n🧪 Testing simple call Alice -> Bob\n");
    
    // Create Alice
    println!("Creating Alice...");
    let alice = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@127.0.0.1:5060")
        .with_handler(Arc::new(SimpleHandler::new("Alice")))
        .build()
        .await
        .expect("Failed to create Alice");
    
    // Create Bob
    println!("Creating Bob...");
    let bob = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:bob@127.0.0.1:5061")
        .with_handler(Arc::new(SimpleHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    
    // Wait for initialization
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Alice calls Bob
    println!("\n📞 Alice calling Bob...");
    let call = alice.create_outgoing_call(
        "sip:alice@127.0.0.1:5060",
        "sip:bob@127.0.0.1:5061",
        None,
    ).await.expect("Failed to create call");
    
    println!("Call created with ID: {}", call.id);
    
    // Wait for call to establish
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    println!("✅ Test complete");
    
    alice.stop().await.ok();
    bob.stop().await.ok();
}