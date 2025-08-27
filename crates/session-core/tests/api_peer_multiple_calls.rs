//! Multiple simultaneous calls tests for SimplePeer API
//! 
//! Demonstrates:
//! - Handling multiple incoming calls
//! - Making multiple outgoing calls
//! - Call bridging

use rvoip_session_core::api::{SimplePeer, Result};
use serial_test::serial;
use tokio::time::{sleep, Duration, timeout};

#[tokio::test]
#[serial]
async fn test_multiple_incoming_calls() -> Result<()> {
    // Bob will receive calls from multiple peers
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5060)
        .await?;
    
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5061)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(5062)
        .await?;
    
    // Bob handles multiple incoming calls
    let bob_handle = tokio::spawn(async move {
        let mut call_count = 0;
        
        // First call from Alice
        if let Ok(Some(incoming)) = timeout(Duration::from_secs(5), bob.next_incoming()).await {
            println!("Bob received call 1 from: {}", incoming.from);
            assert!(incoming.from.contains("alice"));
            let _call = incoming.accept().await.unwrap();
            call_count += 1;
        }
        
        // Second call from Charlie
        if let Ok(Some(incoming)) = timeout(Duration::from_secs(5), bob.next_incoming()).await {
            println!("Bob received call 2 from: {}", incoming.from);
            assert!(incoming.from.contains("charlie"));
            let _call = incoming.accept().await.unwrap();
            call_count += 1;
        }
        
        println!("Bob handled {} calls", call_count);
        assert_eq!(call_count, 2, "Bob should receive 2 calls");
        
        sleep(Duration::from_secs(1)).await;
        bob.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5060)
        .await?;
    
    // Charlie calls Bob
    let charlie_call = charlie.call("bob@127.0.0.1")
        .port(5060)
        .await?;
    
    // Wait for calls to be active
    for _ in 0..10 {
        if alice_call.is_active().await && charlie_call.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    alice_call.hangup().await?;
    charlie_call.hangup().await?;
    
    alice.shutdown().await?;
    charlie.shutdown().await?;
    let _ = bob_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_multiple_outgoing_calls() -> Result<()> {
    // Alice makes calls to multiple peers
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5063)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5064)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(5065)
        .await?;
    
    // Bob accepts calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            println!("Bob accepting call from: {}", incoming.from);
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    // Charlie accepts calls
    let charlie_handle = tokio::spawn(async move {
        if let Some(incoming) = charlie.next_incoming().await {
            println!("Charlie accepting call from: {}", incoming.from);
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(2)).await;
        }
        charlie.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls both Bob and Charlie
    println!("Alice calling Bob...");
    let call_to_bob = alice.call("bob@127.0.0.1")
        .port(5064)
        .await?;
    
    println!("Alice calling Charlie...");
    let call_to_charlie = alice.call("charlie@127.0.0.1")
        .port(5065)
        .await?;
    
    // Wait for calls to become active
    for _ in 0..10 {
        if call_to_bob.is_active().await && call_to_charlie.is_active().await {
            println!("Both calls are active");
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Alice manages both calls
    sleep(Duration::from_secs(1)).await;
    
    call_to_bob.hangup().await?;
    call_to_charlie.hangup().await?;
    alice.shutdown().await?;
    
    let _ = bob_handle.await;
    let _ = charlie_handle.await;
    
    Ok(())
}

#[tokio::test]
#[serial]
async fn test_call_bridging() -> Result<()> {
    // Alice bridges calls between Bob and Charlie
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5066)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5067)
        .await?;
    
    let mut charlie = SimplePeer::new("charlie")
        .local_addr("127.0.0.1")
        .port(5068)
        .await?;
    
    // Bob accepts calls
    let bob_handle = tokio::spawn(async move {
        if let Some(incoming) = bob.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(3)).await;
        }
        bob.shutdown().await.unwrap();
    });
    
    // Charlie accepts calls
    let charlie_handle = tokio::spawn(async move {
        if let Some(incoming) = charlie.next_incoming().await {
            let _call = incoming.accept().await.unwrap();
            sleep(Duration::from_secs(3)).await;
        }
        charlie.shutdown().await.unwrap();
    });
    
    sleep(Duration::from_millis(500)).await;
    
    // Alice calls Bob and Charlie
    let call_to_bob = alice.call("bob@127.0.0.1")
        .port(5067)
        .await?;
    
    let call_to_charlie = alice.call("charlie@127.0.0.1")
        .port(5068)
        .await?;
    
    // Wait for calls to become active
    for _ in 0..10 {
        if call_to_bob.is_active().await && call_to_charlie.is_active().await {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    
    // Bridge the calls (Bob and Charlie can talk to each other)
    println!("Bridging calls between Bob and Charlie...");
    call_to_bob.bridge(call_to_charlie).await?;
    
    sleep(Duration::from_secs(1)).await;
    
    call_to_bob.hangup().await?;
    alice.shutdown().await?;
    
    let _ = bob_handle.await;
    let _ = charlie_handle.await;
    
    Ok(())
}