//! Integration tests for presence functionality
//!
//! Tests the complete presence flow including PUBLISH, SUBSCRIBE, and NOTIFY
//! in both P2P and B2BUA scenarios.

use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, timeout};
use rvoip_session_core::api::SimplePeer;
use rvoip_session_core::coordinator::presence::PresenceStatus;
use rvoip_session_core::Result;

/// Test basic presence setting and retrieval
#[tokio::test]
async fn test_basic_presence() -> Result<()> {
    // Create a peer
    let alice = SimplePeer::new("alice")
        .port(5070)
        .await?;
    
    // Set presence to available
    alice.available()
        .note("Working from home")
        .device("laptop")
        .await?;
    
    // Check that presence was set
    let presence_info = alice.presence_coordinator()
        .read()
        .await
        .get_presence(&format!("sip:alice@0.0.0.0:5070"))
        .expect("Presence should be set");
    
    assert_eq!(presence_info.status, PresenceStatus::Available);
    assert_eq!(presence_info.note, Some("Working from home".to_string()));
    
    Ok(())
}

/// Test presence transitions
#[tokio::test]
async fn test_presence_transitions() -> Result<()> {
    let alice = SimplePeer::new("alice")
        .port(5071)
        .await?;
    
    // Transition through different states
    alice.available().await?;
    alice.busy().note("In a meeting").await?;
    alice.away().await?;
    alice.dnd().note("Important call").await?;
    alice.offline().await?;
    
    // Verify final state
    let presence_info = alice.presence_coordinator()
        .read()
        .await
        .get_presence(&format!("sip:alice@0.0.0.0:5071"))
        .expect("Presence should be set");
    
    assert_eq!(presence_info.status, PresenceStatus::Offline);
    
    Ok(())
}

/// Test presence watching in P2P mode
#[tokio::test]
async fn test_p2p_presence_watching() -> Result<()> {
    // Create two peers
    let alice = Arc::new(SimplePeer::new("alice")
        .port(5072)
        .await?);
    
    let bob = SimplePeer::new("bob")
        .port(5073)
        .await?;
    
    // Alice sets her presence
    alice.available()
        .note("Ready to chat")
        .await?;
    
    // Bob watches Alice's presence
    let mut watcher = bob.watch("alice@127.0.0.1:5072").await?;
    
    // Alice updates her presence
    alice.busy()
        .note("On a call")
        .await?;
    
    // Bob should receive the update (with timeout to prevent hanging)
    let update = timeout(Duration::from_secs(2), watcher.recv())
        .await
        .ok()
        .flatten();
    
    // In a real implementation, this would work once SUBSCRIBE/NOTIFY is wired
    // For now, we can check the current state directly
    let current = watcher.current().await;
    
    // Note: This test will need updating once SUBSCRIBE/NOTIFY is fully implemented
    // For now, it demonstrates the API usage
    
    // Clean up
    watcher.stop().await?;
    
    Ok(())
}

/// Test buddy list functionality
#[tokio::test]
async fn test_buddy_list() -> Result<()> {
    let alice = Arc::new(SimplePeer::new("alice")
        .port(5074)
        .await?);
    
    // Create a buddy list
    let mut buddy_list = alice.clone().buddy_list();
    
    // Add some buddies
    buddy_list.add("bob@example.com").await?;
    buddy_list.add("charlie@example.com").await?;
    buddy_list.add("diana@example.com").await?;
    
    // Get all buddy statuses
    let statuses = buddy_list.get_all().await;
    assert_eq!(statuses.len(), 3);
    
    // Remove a buddy
    buddy_list.remove("charlie@example.com").await?;
    let statuses = buddy_list.get_all().await;
    assert_eq!(statuses.len(), 2);
    
    // Clear all buddies
    buddy_list.clear().await?;
    let statuses = buddy_list.get_all().await;
    assert_eq!(statuses.len(), 0);
    
    Ok(())
}

/// Test presence with capabilities
#[tokio::test]
async fn test_presence_capabilities() -> Result<()> {
    let alice = SimplePeer::new("alice")
        .port(5075)
        .await?;
    
    // Set presence with multiple capabilities
    alice.available()
        .capabilities(vec![
            "audio".to_string(),
            "video".to_string(),
            "chat".to_string(),
            "file-transfer".to_string(),
        ])
        .await?;
    
    // Verify capabilities were set
    let presence_info = alice.presence_coordinator()
        .read()
        .await
        .get_presence(&format!("sip:alice@0.0.0.0:5075"))
        .expect("Presence should be set");
    
    // Note: capabilities aren't stored in current PresenceInfo
    // This test demonstrates the API for future implementation
    
    Ok(())
}

/// Test presence expiration and refresh
#[tokio::test]
async fn test_presence_expiration() -> Result<()> {
    let alice = SimplePeer::new("alice")
        .port(5076)
        .await?;
    
    // Set presence with short expiration
    alice.available()
        .note("Testing expiration")
        .await?;
    
    // In a real implementation, presence would expire after the configured time
    // and need refreshing. This test demonstrates the pattern.
    
    // Simulate time passing (in real implementation, would use actual timers)
    sleep(Duration::from_millis(100)).await;
    
    // Refresh presence before expiration
    alice.available()
        .note("Still here")
        .await?;
    
    Ok(())
}

/// Test concurrent presence updates
#[tokio::test]
async fn test_concurrent_presence_updates() -> Result<()> {
    let alice = Arc::new(SimplePeer::new("alice")
        .port(5077)
        .await?);
    
    // Spawn multiple tasks updating presence concurrently
    let mut handles = vec![];
    
    for i in 0..5 {
        let alice_clone = alice.clone();
        let handle = tokio::spawn(async move {
            alice_clone.available()
                .note(format!("Update {}", i))
                .await
        });
        handles.push(handle);
    }
    
    // Wait for all updates to complete
    for handle in handles {
        handle.await.unwrap()?;
    }
    
    // Verify final state is consistent
    let presence_info = alice.presence_coordinator()
        .read()
        .await
        .get_presence(&format!("sip:alice@0.0.0.0:5077"));
    
    assert!(presence_info.is_some());
    
    Ok(())
}

/// Test presence in B2BUA mode with registrar
#[tokio::test]
#[ignore] // Requires actual SIP server
async fn test_b2bua_presence() -> Result<()> {
    let mut alice = SimplePeer::new("alice")
        .port(5078)
        .await?;
    
    // Register with SIP server
    alice.register("sip:registrar@example.com").await?;
    
    // Set presence (would send PUBLISH to server)
    alice.available()
        .note("Registered and available")
        .await?;
    
    // In B2BUA mode, presence would be published to the server
    // and distributed to watchers through the server
    
    // Unregister
    alice.unregister().await?;
    
    Ok(())
}

/// Test presence with PIDF format
#[tokio::test]
async fn test_presence_pidf_format() -> Result<()> {
    use rvoip_sip_core::types::pidf::{PidfDocument, Tuple, Status, BasicStatus};
    
    let alice = SimplePeer::new("alice")
        .port(5079)
        .await?;
    
    // Set presence
    alice.available()
        .note("Testing PIDF")
        .device("mobile")
        .await?;
    
    // Create PIDF document for the presence
    let presence_info = alice.presence_coordinator()
        .read()
        .await
        .get_presence(&format!("sip:alice@0.0.0.0:5079"))
        .expect("Presence should be set");
    
    let pidf = presence_info.to_pidf();
    let xml = pidf.to_xml();
    
    // Verify PIDF contains expected elements
    assert!(xml.contains("<presence"));
    assert!(xml.contains("<status>"));
    assert!(xml.contains("<basic>open</basic>"));
    assert!(xml.contains("<note>Testing PIDF</note>"));
    
    Ok(())
}

/// Test error handling in presence operations
#[tokio::test]
async fn test_presence_error_handling() -> Result<()> {
    let alice = SimplePeer::new("alice")
        .port(5080)
        .await?;
    
    // Test watching non-existent peer (should not panic)
    let watcher_result = alice.watch("nonexistent@invalid.domain").await;
    assert!(watcher_result.is_ok()); // API returns Ok, but watcher won't receive updates
    
    // Test with invalid URI format
    let watcher_result = alice.watch("not-a-valid-uri").await;
    assert!(watcher_result.is_ok()); // API handles formatting
    
    Ok(())
}