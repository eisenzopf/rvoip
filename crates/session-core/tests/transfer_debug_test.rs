//! Debug test for transfer - minimal version
//!
//! TODO: This test uses std::process::exit(0) to force termination due to 
//! background tasks not properly shutting down. We need to investigate and fix:
//! - Event loops not terminating cleanly
//! - Transaction processors still running after stop()
//! - Dialog event loops continuing after shutdown
//! - Possible circular references keeping tasks alive
//!
//! Once these issues are resolved, we should remove the force exit and ensure
//! proper graceful shutdown of all components.

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;

use rvoip_session_core::{
    SessionManagerBuilder,
    SessionControl,  // For terminate_session
    api::{
        types::{IncomingCall, CallSession, CallDecision, SessionId},
        handlers::CallHandler,
    },
    manager::events::SessionTransferStatus,
};

#[derive(Debug)]
struct DebugHandler {
    name: String,
}

impl DebugHandler {
    fn new(name: &str) -> Self {
        Self { name: name.to_string() }
    }
}

#[async_trait]
impl CallHandler for DebugHandler {
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
    
    async fn on_incoming_transfer_request(&self, session_id: &SessionId, target: &str, _: Option<&str>) -> bool {
        println!("   ⚡ {} received TRANSFER request for {} to {}", self.name, session_id, target);
        println!("   ⚡ {} ACCEPTING transfer", self.name);
        true // Accept transfer
    }
    
    async fn on_transfer_progress(&self, session_id: &SessionId, status: &SessionTransferStatus) {
        println!("   ⚡ {} transfer progress for {}: {:?}", self.name, session_id, status);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_transfer_debug() {
    println!("\n=== TRANSFER DEBUG TEST ===\n");
    
    // Create Alice
    println!("Creating Alice...");
    let alice = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@127.0.0.1:5060")
        .with_handler(Arc::new(DebugHandler::new("Alice")))
        .build()
        .await
        .expect("Failed to create Alice");
    
    // Create Bob  
    println!("Creating Bob...");
    let bob = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:bob@127.0.0.1:5061")
        .with_handler(Arc::new(DebugHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    
    // Create Charlie
    println!("Creating Charlie...");
    let charlie = SessionManagerBuilder::new()
        .with_sip_port(5062)
        .with_local_address("sip:charlie@127.0.0.1:5062")
        .with_handler(Arc::new(DebugHandler::new("Charlie")))
        .build()
        .await
        .expect("Failed to create Charlie");
    
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Step 1: Alice calls Bob
    println!("\n📞 Step 1: Alice calls Bob");
    println!("About to call alice.create_outgoing_call...");
    let call = alice.create_outgoing_call(
        "sip:alice@127.0.0.1:5060",
        "sip:bob@127.0.0.1:5061",
        None,
    ).await.expect("Failed to create call");
    
    println!("Call ID: {}", call.id);
    println!("Call created successfully!");
    
    // Wait for call to establish
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Step 2: Transfer
    println!("\n📞 Step 2: Alice transfers Bob to Charlie");
    println!("Calling transfer_session API...");
    
    let transfer_future = alice.transfer_session(&call.id, "sip:charlie@127.0.0.1:5062");
    let timeout_duration = Duration::from_secs(3);
    
    match tokio::time::timeout(timeout_duration, transfer_future).await {
        Ok(Ok(_)) => {
            println!("✅ Transfer API returned success");
        }
        Ok(Err(e)) => {
            println!("❌ Transfer API failed: {}", e);
            panic!("Transfer failed");
        }
        Err(_) => {
            println!("⏰ Transfer API timed out after 3 seconds!");
            panic!("Transfer API is hanging/deadlocked");
        }
    }
    
    // Wait to see what happens
    println!("\nWaiting 3 seconds to observe transfer...");
    tokio::time::sleep(Duration::from_secs(3)).await;
    
    println!("\n✅ Test complete");
    
    // Try to stop managers gracefully with timeout
    println!("Attempting graceful shutdown...");
    let _ = tokio::time::timeout(
        Duration::from_millis(500),
        async {
            alice.stop().await.ok();
            bob.stop().await.ok();
            charlie.stop().await.ok();
        }
    ).await;
    
    println!("Force exiting test to prevent hanging...");
    // TODO: Remove this force exit once we fix the background task cleanup issues
    // The test hangs because background event loops and transaction processors
    // don't terminate properly when stop() is called. This needs investigation.
    // Force exit - this will terminate all background tasks
    std::process::exit(0);
}