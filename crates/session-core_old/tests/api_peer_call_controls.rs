//! Call control operations tests for SimplePeer API
//! 
//! Demonstrates:
//! - Hold/resume
//! - Mute/unmute
//! - DTMF sending
//! - Call duration tracking

use rvoip_session_core::api::{SimplePeer, Result};
use serial_test::serial;
use tokio::time::{sleep, Duration};

#[tokio::test]
#[serial]
async fn test_hold_resume() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5061)
        .await?;
    
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            // Stay on call while Alice tests hold/resume
            sleep(Duration::from_secs(3)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5061)
        .await?;
    
    // Wait for call to become active
    for _ in 0..10 {
        if alice_call.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Test hold
    println!("Putting call on hold...");
    alice_call.hold().await?;
    assert!(alice_call.is_on_hold().await);
    
    sleep(Duration::from_millis(500)).await;
    
    // Test resume
    println!("Resuming call...");
    alice_call.resume().await?;
    assert!(alice_call.is_active().await);
    
    alice_call.hangup().await?;
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_mute_unmute() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5062)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5063)
        .await?;
    
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5063)
        .await?;
    
    // Wait for call to become active
    for _ in 0..10 {
        if alice_call.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Test mute
    println!("Muting audio...");
    alice_call.mute().await?;
    
    sleep(Duration::from_millis(500)).await;
    
    // Test unmute
    println!("Unmuting audio...");
    alice_call.unmute().await?;
    
    alice_call.hangup().await?;
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_dtmf_sending() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5064)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5065)
        .await?;
    
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5065)
        .await?;
    
    // Wait for call to become active
    for _ in 0..10 {
        if alice_call.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Send DTMF digits
    println!("Sending DTMF: 123");
    alice_call.send_dtmf("123").await?;
    
    println!("Sending DTMF: *#");
    alice_call.send_dtmf("*#").await?;
    
    println!("Sending DTMF: 456789");
    alice_call.send_dtmf("456789").await?;
    
    // Test invalid DTMF
    let invalid_result = alice_call.send_dtmf("XYZ").await;
    assert!(invalid_result.is_err(), "Invalid DTMF should fail");
    
    alice_call.hangup().await?;
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_call_duration() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5066)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5067)
        .await?;
    
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
            println!("Bob's call duration: {:?}", call.duration());
            call.hangup().await.unwrap();
        }
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(200)).await;
    
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5067)
        .await?;
    
    // Wait for call to become active
    for _ in 0..10 {
        if alice_call.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    let start = std::time::Instant::now();
    
    // Let the call run for a bit
    sleep(Duration::from_secs(1)).await;
    
    let duration = alice_call.duration();
    println!("Alice's call duration: {:?}", duration);
    assert!(duration >= Duration::from_millis(900), "Duration should be at least 900ms");
    
    sleep(Duration::from_secs(1)).await;
    alice.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}