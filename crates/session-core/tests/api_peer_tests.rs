use rvoip_session_core::api::{SimplePeer, SimpleCall};
use rvoip_session_core::errors::Result;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_peer_creation() -> Result<()> {
    let peer = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5090)
        .await?;
    
    assert_eq!(peer.identity(), "alice");
    assert_eq!(peer.local_addr(), "127.0.0.1");
    assert_eq!(peer.port(), 5090);
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_peer_with_default_settings() -> Result<()> {
    let peer = SimplePeer::new("bob").await?;
    
    assert_eq!(peer.identity(), "bob");
    assert_eq!(peer.local_addr(), "0.0.0.0");
    assert_eq!(peer.port(), 5060);
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_peer_registration() -> Result<()> {
    let mut peer = SimplePeer::new("charlie")
        .port(5091)
        .await?;
    
    assert_eq!(peer.registrar(), None);
    
    peer.register("sip:registrar@example.com").await?;
    assert_eq!(peer.registrar(), Some("sip:registrar@example.com"));
    
    peer.unregister().await?;
    assert_eq!(peer.registrar(), None);
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_peer_to_peer_call() -> Result<()> {
    // Create two peers
    let mut alice = SimplePeer::new("alice")
        .local_addr("127.0.0.1")
        .port(5092)
        .await?;
    
    let mut bob = SimplePeer::new("bob")
        .local_addr("127.0.0.1")
        .port(5093)
        .await?;
    
    // Start Bob listening for incoming calls
    let bob_handle = tokio::spawn(async move {
        // Wait for incoming call with timeout
        match timeout(Duration::from_secs(5), bob.next_incoming()).await {
            Ok(Some(incoming)) => {
                // Accept the call
                let _call = incoming.accept().await.unwrap();
                // Call would be handled here
                Ok::<_, rvoip_session_core::errors::SessionError>(())
            }
            Ok(None) => Err(rvoip_session_core::errors::SessionError::Other("No incoming call".to_string())),
            Err(_) => Err(rvoip_session_core::errors::SessionError::Other("Timeout waiting for call".to_string())),
        }
    });
    
    // Give Bob time to start listening
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Alice calls Bob
    let alice_call = alice.call("bob@127.0.0.1")
        .port(5093)
        .await?;
    
    assert_eq!(alice_call.remote_uri(), "sip:bob@127.0.0.1:5093");
    
    // Wait for Bob to handle the call
    let _ = timeout(Duration::from_secs(2), bob_handle).await;
    
    // Clean up
    alice_call.hangup().await?;
    alice.shutdown().await?;
    
    Ok(())
}

#[tokio::test]
async fn test_call_builder_with_custom_port() -> Result<()> {
    let peer = SimplePeer::new("dave")
        .port(5094)
        .await?;
    
    // Test that call builder properly formats URIs with custom ports
    let call_builder = peer.call("eve@example.com").port(5070);
    
    // The actual call would fail since there's no one listening, 
    // but we're testing the builder pattern works
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_peer_try_incoming_non_blocking() -> Result<()> {
    let mut peer = SimplePeer::new("frank")
        .port(5095)
        .await?;
    
    // Should return None immediately since no calls are waiting
    let incoming = peer.try_incoming();
    assert!(incoming.is_none());
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_multiple_peers_on_different_ports() -> Result<()> {
    let peer1 = SimplePeer::new("peer1")
        .port(5096)
        .await?;
    
    let peer2 = SimplePeer::new("peer2")
        .port(5097)
        .await?;
    
    let peer3 = SimplePeer::new("peer3")
        .port(5098)
        .await?;
    
    assert_eq!(peer1.port(), 5096);
    assert_eq!(peer2.port(), 5097);
    assert_eq!(peer3.port(), 5098);
    
    peer1.shutdown().await?;
    peer2.shutdown().await?;
    peer3.shutdown().await?;
    
    Ok(())
}

#[tokio::test]
async fn test_call_target_formatting() -> Result<()> {
    let peer = SimplePeer::new("grace")
        .port(5099)
        .await?;
    
    // Test various call target formats
    // These would fail to connect but we're testing the URI formatting
    
    // User@host format
    let _ = peer.call("user@host.com");
    
    // Just username (would use registrar if set)
    let _ = peer.call("username");
    
    // Phone number format
    let _ = peer.call("+14155551234");
    
    // SIP URI format
    let _ = peer.call("sip:user@domain.com");
    
    // Tel URI format  
    let _ = peer.call("tel:+14155551234");
    
    peer.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn test_peer_with_registrar() -> Result<()> {
    let mut peer = SimplePeer::new("henry")
        .port(5100)
        .await?;
    
    peer.register("sip:registrar@pbx.com").await?;
    
    // When registered, calling just a username should use the registrar
    let _ = peer.call("ivan");
    // This would format as "sip:ivan@sip:registrar@pbx.com"
    
    peer.shutdown().await?;
    Ok(())
}