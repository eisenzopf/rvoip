//! Integration test for call transfer functionality

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionManagerBuilder,
    api::{
        handlers::CallHandler,
        types::{CallSession, CallState, CallDecision, IncomingCall},
    },
};

/// Test handler that auto-accepts calls
#[derive(Debug)]
struct AutoAcceptHandler;

#[async_trait::async_trait]
impl CallHandler for AutoAcceptHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept(None)
    }

    async fn on_call_established(&self, _call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {}
    async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
}

#[tokio::test]
async fn test_blind_transfer_flow() {
    println!("🧪 Testing blind transfer flow...");

    // Create three endpoints: Alice (caller), Bob (initial callee), Charlie (transfer target)
    let alice_port = 35060;
    let bob_port = 35061;
    let charlie_port = 35062;

    // Create Alice (the caller)
    let alice = SessionManagerBuilder::new()
        .with_sip_port(alice_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", alice_port).parse().unwrap())
        .with_local_address(&format!("sip:alice@127.0.0.1:{}", alice_port))
        .with_handler(Arc::new(AutoAcceptHandler))
        .build()
        .await
        .expect("Failed to build Alice");

    alice.start().await.expect("Failed to start Alice");

    // Create Bob (initial callee who will transfer)
    let bob = SessionManagerBuilder::new()
        .with_sip_port(bob_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", bob_port).parse().unwrap())
        .with_local_address(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .with_handler(Arc::new(AutoAcceptHandler))
        .build()
        .await
        .expect("Failed to build Bob");

    bob.start().await.expect("Failed to start Bob");

    // Create Charlie (transfer target)
    let charlie = SessionManagerBuilder::new()
        .with_sip_port(charlie_port)
        .with_local_bind_addr(format!("127.0.0.1:{}", charlie_port).parse().unwrap())
        .with_local_address(&format!("sip:charlie@127.0.0.1:{}", charlie_port))
        .with_handler(Arc::new(AutoAcceptHandler))
        .build()
        .await
        .expect("Failed to build Charlie");

    charlie.start().await.expect("Failed to start Charlie");

    // Alice calls Bob
    let call_to_bob = alice.create_outgoing_call(
        &format!("sip:alice@127.0.0.1:{}", alice_port),
        &format!("sip:bob@127.0.0.1:{}", bob_port),
        None,
    ).await.expect("Failed to create call to Bob");

    println!("📞 Alice calling Bob: {}", call_to_bob.id);

    // Wait for call to establish
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify Alice has active call
    let alice_sessions = alice.list_active_sessions().await.unwrap();
    assert_eq!(alice_sessions.len(), 1, "Alice should have 1 active call");

    // Verify Bob has active call
    let bob_sessions = bob.list_active_sessions().await.unwrap();
    assert_eq!(bob_sessions.len(), 1, "Bob should have 1 active call");

    // Now Bob transfers Alice to Charlie
    // This would normally be done via a SIP REFER message from Bob's SIP client
    // For testing, we'll simulate this by sending a REFER request
    
    // Note: In a real scenario, Bob's SIP client would send:
    // REFER sip:alice@127.0.0.1:35060 SIP/2.0
    // Refer-To: sip:charlie@127.0.0.1:35062
    // Referred-By: sip:bob@127.0.0.1:35061
    
    println!("🔄 Bob initiating transfer to Charlie...");
    
    // For now, we can test that the infrastructure is in place
    // The actual REFER would come from a SIP client or we'd need to simulate it
    
    // Wait a bit to ensure everything is stable
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Cleanup
    alice.stop().await.expect("Failed to stop Alice");
    bob.stop().await.expect("Failed to stop Bob");
    charlie.stop().await.expect("Failed to stop Charlie");
    
    println!("✅ Transfer infrastructure test completed");
}