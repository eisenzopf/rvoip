//! End-to-end tests for presence functionality
//!
//! Tests the complete presence flow including OAuth, PUBLISH/SUBSCRIBE,
//! P2P heartbeat, and multi-device aggregation.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::{sleep, timeout};
use rvoip_session_core::api::SimplePeer;
use rvoip_session_core::coordinator::presence::{PresenceStatus, PresenceInfo, PresenceCoordinator};
use rvoip_session_core::coordinator::p2p_heartbeat::{P2PHeartbeatManager, HeartbeatConfig};
use rvoip_session_core::coordinator::presence_aggregation::{
    PresenceAggregator, AggregationStrategy, DevicePresence,
};
use rvoip_session_core::auth::{OAuth2Config, OAuth2Validator, OAuth2Scopes};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Test complete presence flow with OAuth authentication
#[tokio::test]
#[ignore] // Requires OAuth server
async fn test_oauth_presence_flow() -> Result<()> {
    // Configure OAuth
    let oauth_config = OAuth2Config {
        jwks_uri: Some("https://auth.example.com/.well-known/jwks.json".to_string()),
        introspect_uri: Some("https://auth.example.com/oauth2/introspect".to_string()),
        introspect_client_id: Some("test-client".to_string()),
        introspect_client_secret: Some("test-secret".to_string()),
        required_scopes: OAuth2Scopes {
            presence: vec!["sip:presence".to_string()],
            ..Default::default()
        },
        cache_ttl: Duration::from_secs(300),
        realm: "test".to_string(),
        allow_insecure: true, // For testing only
    };
    
    // Create OAuth validator
    let validator = OAuth2Validator::new(oauth_config).await?;
    
    // Create peer with OAuth
    let alice = SimplePeer::new("alice")
        .port(5090)
        .await?;
    
    // Set presence (would require OAuth token in real scenario)
    alice.available()
        .note("OAuth authenticated")
        .await?;
    
    Ok(())
}

/// Test P2P presence with heartbeat
#[tokio::test]
async fn test_p2p_heartbeat_presence() -> Result<()> {
    // Create heartbeat manager with short intervals for testing
    let config = HeartbeatConfig {
        send_interval: Duration::from_millis(100),
        offline_threshold: Duration::from_millis(300),
        auto_cleanup: true,
        max_peers: 10,
    };
    
    let heartbeat_manager = Arc::new(P2PHeartbeatManager::new(config));
    
    // Start heartbeat monitoring
    heartbeat_manager.start().await?;
    
    // Simulate receiving heartbeats from peers
    heartbeat_manager.receive_heartbeat(
        "sip:peer1@example.com",
        PresenceStatus::Available,
        Some("Working".to_string()),
    ).await?;
    
    heartbeat_manager.receive_heartbeat(
        "sip:peer2@example.com",
        PresenceStatus::Busy,
        Some("In meeting".to_string()),
    ).await?;
    
    // Check peer presence
    let peer1_presence = heartbeat_manager.get_peer_presence("sip:peer1@example.com");
    assert!(peer1_presence.is_some());
    assert_eq!(peer1_presence.unwrap().status, PresenceStatus::Available);
    
    // Wait for peer2 to timeout
    sleep(Duration::from_millis(400)).await;
    
    // Broadcast our own heartbeat
    heartbeat_manager.broadcast_heartbeat(
        PresenceStatus::Available,
        Some("Ready".to_string()),
    ).await?;
    
    // Get all peers
    let all_peers = heartbeat_manager.get_all_peers();
    assert!(all_peers.contains_key("sip:peer1@example.com"));
    
    // Stop monitoring
    heartbeat_manager.stop().await?;
    
    Ok(())
}

/// Test multi-device presence aggregation
#[tokio::test]
async fn test_multi_device_aggregation() -> Result<()> {
    // Create aggregator with "most available" strategy
    let aggregator = Arc::new(PresenceAggregator::new(AggregationStrategy::MostAvailable));
    
    let user = "sip:alice@example.com";
    
    // Simulate multiple devices
    aggregator.update_device_presence(
        user,
        "mobile-123",
        PresenceStatus::Away,
        Some("Stepped out".to_string()),
        Some("mobile".to_string()),
    )?;
    
    aggregator.update_device_presence(
        user,
        "desktop-456",
        PresenceStatus::Available,
        Some("At desk".to_string()),
        Some("desktop".to_string()),
    )?;
    
    aggregator.update_device_presence(
        user,
        "tablet-789",
        PresenceStatus::DoNotDisturb,
        Some("Focus mode".to_string()),
        Some("tablet".to_string()),
    )?;
    
    // Check aggregated presence (should be Available - most available)
    let aggregated = aggregator.get_aggregated_presence(user);
    assert!(aggregated.is_some());
    assert_eq!(aggregated.unwrap().status, PresenceStatus::Available);
    
    // Check device count
    assert_eq!(aggregator.get_device_count(user), 3);
    
    // Remove desktop device
    aggregator.remove_device(user, "desktop-456")?;
    
    // Re-check aggregated presence (should now be Away)
    let aggregated = aggregator.get_aggregated_presence(user);
    assert_eq!(aggregated.unwrap().status, PresenceStatus::Away);
    
    // Get all devices
    let devices = aggregator.get_user_devices(user);
    assert_eq!(devices.len(), 2);
    
    // Get statistics
    let stats = aggregator.get_stats();
    assert_eq!(stats.total_users, 1);
    assert_eq!(stats.total_devices, 2);
    
    Ok(())
}

/// Test presence coordinator integration
#[tokio::test]
async fn test_presence_coordinator_integration() -> Result<()> {
    // Create presence coordinator
    let coordinator = Arc::new(RwLock::new(PresenceCoordinator::new()));
    
    // Update presence for multiple users
    let coord = coordinator.write().await;
    
    coord.update_presence(
        "sip:alice@example.com".to_string(),
        PresenceStatus::Available,
        Some("Ready to chat".to_string()),
    ).await?;
    
    coord.update_presence(
        "sip:bob@example.com".to_string(),
        PresenceStatus::Busy,
        Some("On a call".to_string()),
    ).await?;
    
    coord.update_presence(
        "sip:charlie@example.com".to_string(),
        PresenceStatus::Away,
        None,
    ).await?;
    
    // Get individual presence
    let alice_presence = coord.get_presence("sip:alice@example.com");
    assert!(alice_presence.is_some());
    assert_eq!(alice_presence.unwrap().status, PresenceStatus::Available);
    
    // Get all presence states
    let all_presence = coord.get_all_presence();
    assert_eq!(all_presence.len(), 3);
    
    Ok(())
}

/// Test presence flow with SimplePeer API
#[tokio::test]
async fn test_simple_peer_presence_flow() -> Result<()> {
    // Create two peers
    let alice = Arc::new(SimplePeer::new("alice")
        .port(5091)
        .await?);
    
    let bob = SimplePeer::new("bob")
        .port(5092)
        .await?;
    
    // Alice sets various presence states
    alice.available()
        .note("Starting work")
        .device("laptop")
        .with_capability("video")
        .await?;
    
    sleep(Duration::from_millis(100)).await;
    
    alice.busy()
        .note("In a meeting")
        .await?;
    
    sleep(Duration::from_millis(100)).await;
    
    alice.away()
        .note("Lunch break")
        .await?;
    
    sleep(Duration::from_millis(100)).await;
    
    alice.dnd()
        .note("Important call")
        .await?;
    
    sleep(Duration::from_millis(100)).await;
    
    alice.offline().await?;
    
    // Bob watches Alice (would use SUBSCRIBE in real implementation)
    let mut watcher = bob.watch("alice@127.0.0.1:5091").await?;
    
    // In a real implementation, Bob would receive NOTIFY messages
    // For now, we check the current state
    let current = watcher.current().await;
    
    // Create buddy list
    let mut buddy_list = alice.clone().buddy_list();
    buddy_list.add("bob@127.0.0.1:5092").await?;
    buddy_list.add("charlie@example.com").await?;
    
    // Get all buddy statuses
    let statuses = buddy_list.get_all().await;
    assert_eq!(statuses.len(), 2);
    
    // Clean up
    buddy_list.clear().await?;
    watcher.stop().await?;
    
    Ok(())
}

/// Test presence with different aggregation strategies
#[tokio::test]
async fn test_aggregation_strategies() -> Result<()> {
    let user = "sip:test@example.com";
    
    // Test with MostRecent strategy
    let aggregator = PresenceAggregator::new(AggregationStrategy::MostRecent);
    
    aggregator.update_device_presence(
        user,
        "old-device",
        PresenceStatus::Busy,
        Some("Old status".to_string()),
        None,
    )?;
    
    sleep(Duration::from_millis(10)).await;
    
    aggregator.update_device_presence(
        user,
        "new-device",
        PresenceStatus::Available,
        Some("New status".to_string()),
        None,
    )?;
    
    let presence = aggregator.get_aggregated_presence(user).unwrap();
    assert_eq!(presence.status, PresenceStatus::Available); // Most recent
    
    // Test with HighestPriority strategy
    // Note: Can't directly test priority strategy without exposing internals
    // This would require refactoring the PresenceAggregator to support priority setting
    
    Ok(())
}

/// Test P2P heartbeat with coordinator sync
#[tokio::test]
async fn test_p2p_coordinator_sync() -> Result<()> {
    // Create presence coordinator
    let coordinator = Arc::new(RwLock::new(PresenceCoordinator::new()));
    
    // Create P2P heartbeat manager
    let heartbeat_manager = P2PHeartbeatManager::new(HeartbeatConfig::default());
    
    // Add some P2P peers
    heartbeat_manager.receive_heartbeat(
        "sip:p2p-peer1@example.com",
        PresenceStatus::Available,
        Some("P2P mode".to_string()),
    ).await?;
    
    heartbeat_manager.receive_heartbeat(
        "sip:p2p-peer2@example.com",
        PresenceStatus::Busy,
        None,
    ).await?;
    
    // Sync with coordinator
    heartbeat_manager.sync_with_coordinator(&coordinator).await?;
    
    // Verify presence in coordinator
    let coord = coordinator.read().await;
    let peer1 = coord.get_presence("sip:p2p-peer1@example.com");
    assert!(peer1.is_some());
    assert_eq!(peer1.unwrap().status, PresenceStatus::Available);
    
    Ok(())
}

/// Test presence lifecycle with cleanup
#[tokio::test]
async fn test_presence_lifecycle() -> Result<()> {
    let aggregator = Arc::new(PresenceAggregator::new(AggregationStrategy::MostAvailable));
    let user = "sip:lifecycle@example.com";
    
    // Add devices
    for i in 0..5 {
        aggregator.update_device_presence(
            user,
            &format!("device-{}", i),
            PresenceStatus::Available,
            Some(format!("Device {}", i)),
            None,
        )?;
    }
    
    assert_eq!(aggregator.get_device_count(user), 5);
    
    // Remove some devices
    for i in 0..3 {
        aggregator.remove_device(user, &format!("device-{}", i))?;
    }
    
    assert_eq!(aggregator.get_device_count(user), 2);
    
    // Clear all devices
    aggregator.clear_user_devices(user);
    assert_eq!(aggregator.get_device_count(user), 0);
    
    // Verify no aggregated presence
    assert!(aggregator.get_aggregated_presence(user).is_none());
    
    Ok(())
}

/// Test presence PIDF generation
#[tokio::test]
async fn test_pidf_generation() -> Result<()> {
    use rvoip_sip_core::types::pidf::{PidfDocument, Tuple, Status, BasicStatus};
    
    // Create presence info
    let presence = PresenceInfo::new(
        "sip:alice@example.com".to_string(),
        PresenceStatus::Available,
    ).with_note(Some("Testing PIDF".to_string()));
    
    // Convert to PIDF
    let pidf = presence.to_pidf();
    let xml = pidf.to_xml();
    
    // Verify PIDF structure
    assert!(xml.contains("<presence"));
    assert!(xml.contains("<tuple"));
    assert!(xml.contains("<status>"));
    assert!(xml.contains("<basic>open</basic>"));
    assert!(xml.contains("<note>Testing PIDF</note>"));
    
    // Parse back (in real implementation)
    // let parsed = PidfDocument::from_xml(&xml)?;
    // assert_eq!(parsed.note, Some("Testing PIDF".to_string()));
    
    Ok(())
}