//! Standalone test for blind call transfer
//! 
//! This test is isolated for debugging the transfer flow:
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
    
    async fn on_incoming_transfer_request(&self, session_id: &SessionId, target: &str, referred_by: Option<&str>) -> bool {
        println!("   {} received transfer request for session {} to {}", self.name, session_id, target);
        if let Some(referrer) = referred_by {
            println!("     Referred by: {}", referrer);
        }
        true // Accept transfer
    }
    
    async fn on_transfer_progress(&self, session_id: &SessionId, status: &SessionTransferStatus) {
        println!("   {} transfer status for {}: {:?}", self.name, session_id, status);
    }
}

#[tokio::test]
async fn test_blind_transfer_full_flow() {
    // This test uses println! for output visibility
    
    println!("\n=====================================");
    println!("🧪 BLIND TRANSFER TEST");
    println!("=====================================\n");
    
    // Create Alice
    println!("Creating Alice on port 5060...");
    let alice = SessionManagerBuilder::new()
        .with_sip_port(5060)
        .with_local_address("sip:alice@127.0.0.1:5060")
        .with_handler(Arc::new(SimpleHandler::new("Alice")))
        .build()
        .await
        .expect("Failed to create Alice");
    println!("✅ Alice created\n");
    
    // Create Bob  
    println!("Creating Bob on port 5061...");
    let bob = SessionManagerBuilder::new()
        .with_sip_port(5061)
        .with_local_address("sip:bob@127.0.0.1:5061")
        .with_handler(Arc::new(SimpleHandler::new("Bob")))
        .build()
        .await
        .expect("Failed to create Bob");
    println!("✅ Bob created\n");
    
    // Create Charlie
    println!("Creating Charlie on port 5062...");
    let charlie = SessionManagerBuilder::new()
        .with_sip_port(5062)
        .with_local_address("sip:charlie@127.0.0.1:5062")
        .with_handler(Arc::new(SimpleHandler::new("Charlie")))
        .build()
        .await
        .expect("Failed to create Charlie");
    println!("✅ Charlie created\n");
    
    // Let everything initialize
    println!("Waiting for initialization...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    println!("\n-------------------------------------");
    println!("📞 STEP 1: Alice calls Bob");
    println!("-------------------------------------");
    
    let call = alice.create_outgoing_call(
        "sip:alice@127.0.0.1:5060",
        "sip:bob@127.0.0.1:5061",
        None,
    ).await.expect("Failed to create call");
    
    println!("Call created with ID: {}", call.id);
    
    // Wait for call to establish
    println!("Waiting for call to establish...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    println!("\n-------------------------------------");
    println!("📞 STEP 2: Alice transfers Bob to Charlie");
    println!("-------------------------------------");
    
    println!("Initiating transfer to sip:charlie@127.0.0.1:5062");
    
    alice.transfer_session(
        &call.id,
        "sip:charlie@127.0.0.1:5062"
    ).await.expect("Failed to initiate transfer");
    
    println!("Transfer initiated, waiting for completion...");
    
    // Let the library handle the transfer
    tokio::time::sleep(Duration::from_secs(5)).await;
    
    println!("\n-------------------------------------");
    println!("✅ TEST COMPLETE");
    println!("-------------------------------------");
    println!("The library should have:");
    println!("  1. Sent REFER from Alice to Bob");
    println!("  2. Bob accepted with 202");
    println!("  3. Bob called Charlie");
    println!("  4. Bob sent NOTIFY updates to Alice");
    println!("  5. Original call terminated or held");
    
    // Cleanup - properly stop all sessions
    println!("\nCleaning up...");
    
    // Use tokio::join to stop all managers concurrently with timeout
    let stop_timeout = Duration::from_secs(2);
    let stop_result = tokio::time::timeout(
        stop_timeout,
        async {
            tokio::join!(
                alice.stop(),
                bob.stop(),
                charlie.stop()
            )
        }
    ).await;
    
    match stop_result {
        Ok(_) => println!("All session managers stopped cleanly"),
        Err(_) => {
            println!("Warning: Session managers did not stop within timeout");
            println!("Forcing shutdown...");
        }
    }
    
    // Give a brief moment for any remaining async tasks to complete
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    println!("Test finished");
}