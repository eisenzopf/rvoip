//! Concurrent call tests for SimplePeer API
//! 
//! This module tests scenarios with multiple concurrent calls:
//! - Multiple peers calling each other simultaneously
//! - Call bridging between multiple parties
//! - Stress testing with many concurrent calls

use rvoip_session_core::api::{SimplePeer, Result};
use serial_test::serial;
use std::sync::Arc;
use tokio::sync::Barrier;
use tokio::time::{sleep, Duration, timeout};

#[tokio::test]
#[serial]
pub async fn test_concurrent_bidirectional_calls() -> Result<()> {
    // Create 4 peers using standard ports
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5061)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(5062)
        .await?;
    
    let mut dave = SimplePeer::new("dave")
        .local_addr("127.0.0.1")
        .port(5063)
        .await?;
    
    // Set up handlers for incoming calls
    let barrier = Arc::new(Barrier::new(4));
    
    // Bob handles calls
    let barrier_bob = barrier.clone();
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            println!("Bob received call from: {}", incoming.from);
            let _call = incoming.accept().await.unwrap();
            barrier_bob.wait().await;
        }
        bob.shutdown().await.unwrap();
    });
    
    // Charlie handles calls  
    let barrier_charlie = barrier.clone();
    let charlie_handle = tokio::spawn(async move {
        if let Some(incoming) = charlie.next_incoming().await {
            println!("Charlie received call from: {}", incoming.from);
            let _call = incoming.accept().await.unwrap();
            barrier_charlie.wait().await;
        }
        charlie.shutdown().await.unwrap();
    });
    
    // Dave handles calls
    let barrier_dave = barrier.clone();
    let dave_handle = tokio::spawn(async move {
        if let Some(incoming) = dave.next_incoming().await {
            println!("Dave received call from: {}", incoming.from);
            let _call = incoming.accept().await.unwrap();
            barrier_dave.wait().await;
        }
        dave.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls everyone concurrently
    let call_to_bob = alice.call("bob@127.0.0.1")
        .port(5061)
        .await?;
    
    let call_to_charlie = alice.call("charlie@127.0.0.1")
        .port(5062)
        .await?;
    
    let call_to_dave = alice.call("dave@127.0.0.1")
        .port(5063)
        .await?;
    
    // Wait for all calls to be established
    barrier.wait().await;
    
    // Wait for all calls to become active
    let mut all_active = false;
    for _ in 0..20 {
        if call_to_bob.is_active().await &&
           call_to_charlie.is_active().await &&
           call_to_dave.is_active().await {
            all_active = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(all_active, "Not all calls became active");
    
    // Clean up
    call_to_bob.hangup().await?;
    call_to_charlie.hangup().await?;
    call_to_dave.hangup().await?;
    alice.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    let _ = timeout(Duration::from_secs(2), charlie_handle).await;
    let _ = timeout(Duration::from_secs(2), dave_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_call_bridging() -> Result<()> {
    let base_port = 5060;
    
    // Create 3 peers for bridging scenario
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(base_port)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(base_port + 1)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(base_port + 2)
        .await?;
    
    // Bob and Charlie wait for calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    let charlie_handle = tokio::spawn(async move {
        if let Some(incoming) = charlie.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        charlie.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls Bob and Charlie
    let call_to_bob = alice.call("bob@127.0.0.1")
        .port(base_port + 1)
        .await?;
    
    let call_to_charlie = alice.call("charlie@127.0.0.1")
        .port(base_port + 2)
        .await?;
    
    // Bridge the two calls (3-way conference)
    call_to_bob.bridge(call_to_charlie).await?;
    
    // Keep bridge active for a moment
    sleep(Duration::from_secs(1)).await;
    
    // Clean up
    call_to_bob.hangup().await?;
    alice.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    let _ = timeout(Duration::from_secs(2), charlie_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_many_concurrent_peers() -> Result<()> {
    const NUM_PEERS: usize = 10;
    let base_port = 5060;
    
    // Create multiple peers
    let mut peers = Vec::new();
    for i in 0..NUM_PEERS {
        let peer = SimplePeer::new(&format!("peer{}", i))
            .local_addr("127.0.0.1")
            .port(base_port + i as u16)
            .await?;
        peers.push(peer);
    }
    
    // Each peer tries to call the next one in a ring
    let mut handles = Vec::new();
    
    for i in 0..NUM_PEERS {
        let next_index = (i + 1) % NUM_PEERS;
        let next_port = base_port + next_index as u16;
        let mut peer = peers.remove(0);
        
        let handle = tokio::spawn(async move {
            // Wait a bit based on peer index
            sleep(Duration::from_millis(100 * i as u64)).await;
            
            // Try to make outgoing call
            match peer.call(&format!("peer{}@127.0.0.1", next_index))
                .port(next_port)
                .await
            {
                Ok(call) => {
                    sleep(Duration::from_millis(500)).await;
                    call.hangup().await.unwrap();
                }
                Err(e) => {
                    eprintln!("Peer {} failed to call: {}", i, e);
                }
            }
            
            peer.shutdown().await.unwrap();
        });
        
        handles.push(handle);
    }
    
    // Wait for all peers to complete
    for handle in handles {
        let _ = timeout(Duration::from_secs(5), handle).await;
    }
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_rapid_call_setup_teardown() -> Result<()> {
    let alice_port = 5060;
    let bob_port = 5061;
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(alice_port)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Bob handles rapid incoming calls
    let bob_handle = tokio::spawn(async move {
        let mut call_count = 0;
        
        loop {
            match timeout(Duration::from_secs(2), bob.next_incoming()).await {
                Ok(Some(incoming)) => {
                    call_count += 1;
                    let call = incoming.accept().await.unwrap();
                    // Wait a bit for call to establish before hanging up
                    sleep(Duration::from_millis(200)).await;
                    call.hangup().await.unwrap();
                    
                    if call_count >= 5 {
                        break;
                    }
                }
                _ => break,
            }
        }
        
        println!("Bob handled {} rapid calls", call_count);
        assert!(call_count >= 5, "Should handle at least 5 rapid calls");
        
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice makes rapid calls
    for i in 0..5 {
        println!("Alice making call {}", i + 1);
        
        let call = alice.call("bob@127.0.0.1")
            .port(bob_port)
            .await?;
        
        // Wait for call to become active
        let mut active = false;
        for _ in 0..10 {
            if call.is_active().await {
                active = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        assert!(active, "Call {} did not become active", i + 1);
        
        // Hang up quickly
        call.hangup().await?;
        
        // Small delay between calls
        sleep(Duration::from_millis(50)).await;
    }
    
    alice.shutdown().await?;
    let _ = timeout(Duration::from_secs(3), bob_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_presence_coordinator_access() -> Result<()> {
    let peer = SimplePeer::new("test")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    // Verify we can access the presence coordinator
    let presence = peer.presence_coordinator();
    
    // The presence coordinator should be accessible
    let guard = presence.read().await;
    drop(guard);
    
    peer.shutdown().await?;
    Ok(())
}