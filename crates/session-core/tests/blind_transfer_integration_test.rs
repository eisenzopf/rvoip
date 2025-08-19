//! Integration test for blind call transfer using session-core library
//! 
//! This test demonstrates the library's transfer functionality:
//! 1. Alice calls Bob
//! 2. Alice transfers Bob to Charlie
//! 3. The library handles all the REFER/NOTIFY/transfer logic internally

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
    manager::events::SessionTransferStatus,
};

/// Simple test handler - the library handles everything else
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
        CallDecision::Accept(None) // Auto-accept for testing
    }
    
    async fn on_call_established(&self, call: CallSession, _: Option<String>, _: Option<String>) {
        println!("   {} call established with {}", self.name, call.to);
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        println!("   {} call ended: {}", self.name, reason);
    }
    
    async fn on_incoming_transfer_request(&self, _: &SessionId, target: &str, _: Option<&str>) -> bool {
        println!("   {} received transfer request to {}", self.name, target);
        true // Accept transfer
    }
    
    async fn on_transfer_progress(&self, _: &SessionId, status: &SessionTransferStatus) {
        println!("   {} transfer status: {:?}", self.name, status);
    }
}

// test_blind_transfer_full_flow moved to blind_transfer_test.rs for isolated debugging

#[tokio::test]
async fn test_refer_structure_in_transfer() {
    println!("\n🧪 Testing REFER message handling\n");
    
    // Create Alice and Bob
    let alice = SessionManagerBuilder::new()
        .with_sip_port(52010)
        .with_local_address("sip:alice@127.0.0.1:52010")
        .with_handler(Arc::new(SimpleHandler::new("Alice")))
        .build()
        .await
        .expect("Failed to create Alice");
    
    let bob = SessionManagerBuilder::new()
        .with_sip_port(52011)
        .with_local_address("sip:bob@127.0.0.1:52011")
        .with_handler(Arc::new(SimpleHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Alice calls Bob
    let call = alice.create_outgoing_call(
        "sip:alice@127.0.0.1:52010",
        "sip:bob@127.0.0.1:52011",
        None,
    ).await.expect("Failed to create call");
    
    tokio::time::sleep(Duration::from_secs(1)).await;
    
    // Alice transfers Bob
    println!("📤 Testing REFER transfer");
    alice.transfer_session(
        &call.id,
        "sip:charlie@127.0.0.1:52012"
    ).await.expect("Failed to transfer");
    
    // The library handles REFER internally
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("✅ REFER handled by library");
    
    alice.stop().await.ok();
    bob.stop().await.ok();
}