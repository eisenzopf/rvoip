//! Basic call establishment tests for SimplePeer API
//! 
//! Demonstrates:
//! - Creating peers
//! - Making calls
//! - Accepting calls
//! - Call state transitions
//! - Hanging up

use rvoip_session_core::api::{SimplePeer, Result};
use serial_test::serial;
use tokio::time::{sleep, Duration};

#[tokio::test]
#[serial]
async fn test_basic_call_establishment() -> Result<()> {
    // Create two peers on standard SIP ports
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5061)
        .await?;
    
    // Bob waits for incoming call
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            println!("Bob received call from: {}", incoming.from);
            
            // Accept the call
            let call = incoming.accept().await.unwrap();
            println!("Bob accepted call");
            
            // Stay on call for a bit
            sleep(Duration::from_secs(1)).await;
            
            // Hang up
            call.hangup().await.unwrap();
            println!("Bob hung up");
        }
        
        bob.shutdown().await.unwrap();
    });
    
    // Give Bob time to start listening
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls Bob
    println!("Alice calling Bob...");
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5061)
        .await?;
    
    // Wait for call to become active
    let mut active = false;
    for _ in 0..10 {
        if alice_call.is_active().await {
            active = true;
            println!("Alice's call is active");
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(active, "Call did not become active");
    
    // Wait for Bob to hang up
    sleep(Duration::from_secs(2)).await;
    
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_call_rejection() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5062)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5063)
        .await?;
    
    // Bob rejects incoming calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            println!("Bob rejecting call from: {}", incoming.from);
            incoming.reject("Busy").await.unwrap();
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice tries to call Bob (will fail or be rejected)
    let result = alice.call("bob@127.0.0.1")
        .port(5063)
        .await;
    
    println!("Call result: {:?}", result.is_ok());
    
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}