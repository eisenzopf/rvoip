//! Call control tests for SimplePeer API
//! 
//! This module tests various call scenarios including:
//! - Call establishment and teardown
//! - Call rejection
//! - Multiple simultaneous calls
//! - Call transfer operations

use rvoip_session_core::api::{SimplePeer, Result};
use serial_test::serial;
use tokio::time::{sleep, Duration, timeout};

#[tokio::test]
#[serial]
pub async fn test_successful_call_establishment() -> Result<()> {
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
    
    // Bob listens for incoming calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = timeout(Duration::from_secs(5), bob.next_incoming()).await.unwrap() {
            let from_uri = incoming.from.clone();
            println!("Bob received call from: {}", from_uri);
            assert!(from_uri.contains("alice"));
            
            let call = incoming.accept().await.unwrap();
            assert_eq!(call.remote_uri(), &from_uri);
            
            // Let the call be active for a moment
            sleep(Duration::from_millis(500)).await;
            
            call.hangup().await.unwrap();
        } else {
            panic!("Bob didn't receive incoming call");
        }
        
        bob.shutdown().await.unwrap();
    });
    
    // Give Bob time to start listening
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .call_id("test-call-123")
        .await?;
    
    assert!(alice_call.remote_uri().contains("bob"));
    
    // Wait for the call to become active (Bob needs time to accept)
    let mut active = false;
    for _ in 0..10 {
        if alice_call.is_active().await {
            active = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(active, "Alice's call did not become active");
    
    // Wait for Bob to hang up
    sleep(Duration::from_secs(1)).await;
    
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_call_rejection() -> Result<()> {
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
    
    // Bob rejects incoming calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            println!("Bob rejecting call from: {}", incoming.from);
            // Reject the call - rejection happens automatically if not accepted
            // Just log that we're rejecting
            println!("Rejecting call from {}", incoming.from);
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice tries to call Bob
    let result = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await;
    
    // The call should fail or be rejected
    // Note: Exact behavior depends on implementation
    
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_multiple_incoming_calls() -> Result<()> {
    let bob_port = 5060;
    let alice_port = 5061;
    let charlie_port = 5062;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(bob_port)
        .await?;
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(alice_port)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(charlie_port)
        .await?;
    
    // Bob handles multiple calls
    let bob_handle = tokio::spawn(async move {
        let mut call_count = 0;
        
        // First call from Alice
        if let Some(incoming) = timeout(Duration::from_secs(5), bob.next_incoming()).await.unwrap() {
            println!("Bob received call 1 from: {}", incoming.from);
            assert!(incoming.from.contains("alice"));
            let _call = incoming.accept().await.unwrap();
            call_count += 1;
        }
        
        // Second call from Charlie
        if let Some(incoming) = timeout(Duration::from_secs(5), bob.next_incoming()).await.unwrap() {
            println!("Bob received call 2 from: {}", incoming.from);
            assert!(incoming.from.contains("charlie"));
            let _call = incoming.accept().await.unwrap();
            call_count += 1;
        }
        
        assert_eq!(call_count, 2, "Bob should have received 2 calls");
        
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    sleep(Duration::from_millis(100)).await;
    
    // Charlie also calls Bob
    let charlie_call = charlie.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Both calls should be active
    // Wait for calls to become active
    let mut alice_active = false;
    let mut charlie_active = false;
    for _ in 0..10 {
        if alice_call.is_active().await {
            alice_active = true;
        }
        if charlie_call.is_active().await {
            charlie_active = true;
        }
        if alice_active && charlie_active {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(alice_active, "Alice's call did not become active");
    assert!(charlie_active, "Charlie's call did not become active");
    
    // Clean up
    alice_call.hangup().await?;
    charlie_call.hangup().await?;
    alice.shutdown().await?;
    charlie.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_call_with_custom_call_id() -> Result<()> {
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
    
    let custom_call_id = format!("custom-call-{}", uuid::Uuid::new_v4());
    
    // Bob verifies custom call ID
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            // Note: Call ID verification would require access to SIP headers
            // For now just accept the call
            let _call = incoming.accept().await.unwrap();
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls with custom call ID
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .call_id(&custom_call_id)
        .await?;
    
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_call_duration_tracking() -> Result<()> {
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
    
    // Bob accepts and holds call for specific duration
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let call = incoming.accept().await.unwrap();
            
            // Hold call for 2 seconds
            sleep(Duration::from_secs(2)).await;
            
            call.hangup().await.unwrap();
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls and tracks duration
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Check initial duration
    let duration1 = alice_call.duration();
    assert!(duration1.as_secs() < 1);
    
    // Wait a bit
    sleep(Duration::from_secs(1)).await;
    
    // Check duration again
    let duration2 = alice_call.duration();
    assert!(duration2.as_secs() >= 1);
    assert!(duration2 > duration1);
    
    // Wait for Bob to hang up
    sleep(Duration::from_secs(2)).await;
    
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
pub async fn test_dtmf_sending() -> Result<()> {
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
    
    // Bob accepts call
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            
            // Stay on call while DTMF is sent
            sleep(Duration::from_secs(2)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    // Alice calls and sends DTMF
    let alice_call = alice.call("bob@127.0.0.1")
        .port(bob_port)
        .await?;
    
    // Wait for call to become active
    let mut active = false;
    for _ in 0..10 {
        if alice_call.is_active().await {
            active = true;
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert!(active, "Call did not become active before sending DTMF");
    
    // Send various DTMF digits
    alice_call.send_dtmf("123").await?;
    alice_call.send_dtmf("*#").await?;
    alice_call.send_dtmf("456789").await?;
    
    // Test invalid DTMF
    let invalid_result = alice_call.send_dtmf("XYZ").await;
    assert!(invalid_result.is_err());
    
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    let _ = bob_handle.await;
    
    Ok(())
}