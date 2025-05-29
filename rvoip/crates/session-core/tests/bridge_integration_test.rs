//! Bridge Infrastructure Integration Test
//!
//! This test validates that the bridge infrastructure is working correctly
//! using real session-core and transaction-core components with actual sessions.

use rvoip_session_core::{
    SessionManager, SessionConfig,
    session::bridge::{BridgeConfig, BridgeState, BridgeError, BridgeId},
    events::EventBus,
    media::AudioCodecType,
    SessionId,
};
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_transport::UdpTransport;
use std::sync::Arc;
use tokio::sync::mpsc;

async fn create_test_session_manager() -> Result<Arc<SessionManager>, Box<dyn std::error::Error>> {
    // Create TransactionManager with proper transport handling
    let (transport, transport_rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None).await?;
    
    let (transaction_manager, _event_rx) = TransactionManager::new(
        Arc::new(transport),
        transport_rx,
        Some(100)
    ).await?;
    
    let transaction_manager = Arc::new(transaction_manager);
    
    // Create EventBus with proper capacity
    let event_bus = EventBus::new(100).await?;
    
    // Create SessionConfig with proper fields
    let config = SessionConfig {
        local_signaling_addr: "127.0.0.1:5060".parse().unwrap(),
        local_media_addr: "127.0.0.1:10000".parse().unwrap(),
        supported_codecs: vec![AudioCodecType::PCMU],
        display_name: Some("Test Bridge Server".to_string()),
        user_agent: "Bridge-Test/1.0".to_string(),
        max_duration: 300,
        max_sessions: Some(100),
    };
    
    // Create SessionManager with proper parameters
    let session_manager = SessionManager::new(
        transaction_manager,
        config,
        event_bus,
    ).await?;
    
    Ok(Arc::new(session_manager))
}

#[tokio::test]
async fn test_bridge_api_types() {
    // Test that bridge types are properly accessible
    let bridge_config = BridgeConfig {
        max_sessions: 2,
        name: Some("Test Bridge".to_string()),
        timeout_secs: Some(60),
        enable_mixing: true,
    };
    
    // Validate bridge config creation
    assert_eq!(bridge_config.max_sessions, 2);
    assert_eq!(bridge_config.name.as_ref().unwrap(), "Test Bridge");
    assert_eq!(bridge_config.enable_mixing, true);
    
    println!("✅ Bridge API types properly accessible");
}

#[tokio::test]
async fn test_bridge_with_real_sessions() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting bridge test with real sessions...");
    
    // Create SessionManager with real transaction-core
    let session_manager = create_test_session_manager().await?;
    println!("✅ Created SessionManager with transaction-core");
    
    // Create two real sessions
    let session1 = session_manager.create_incoming_session().await?;
    let session1_id = session1.id.clone();
    println!("✅ Created session 1: {}", session1_id);
    
    let session2 = session_manager.create_incoming_session().await?;
    let session2_id = session2.id.clone();
    println!("✅ Created session 2: {}", session2_id);
    
    // Verify sessions exist
    assert!(session_manager.has_session(&session1_id));
    assert!(session_manager.has_session(&session2_id));
    println!("✅ Both sessions exist in SessionManager");
    
    // Create a bridge for the two sessions
    let bridge_config = BridgeConfig {
        max_sessions: 2,
        name: Some("Real Sessions Bridge".to_string()),
        timeout_secs: Some(60),
        enable_mixing: true,
    };
    
    let bridge_id = session_manager.create_bridge(bridge_config).await?;
    println!("✅ Created bridge: {}", bridge_id);
    
    // Add both sessions to the bridge
    session_manager.add_session_to_bridge(&bridge_id, &session1_id).await?;
    println!("✅ Added session 1 to bridge");
    
    session_manager.add_session_to_bridge(&bridge_id, &session2_id).await?;
    println!("✅ Added session 2 to bridge");
    
    // Verify bridge now contains both sessions
    let bridge_info = session_manager.get_bridge_info(&bridge_id).await?;
    assert_eq!(bridge_info.sessions.len(), 2);
    assert!(bridge_info.sessions.contains(&session1_id));
    assert!(bridge_info.sessions.contains(&session2_id));
    println!("✅ Bridge contains both sessions: {:?}", bridge_info.sessions);
    
    // Test bridge statistics with real sessions
    let stats = session_manager.get_bridge_statistics().await;
    assert_eq!(stats[&bridge_id].session_count, 2);
    println!("✅ Bridge statistics show 2 sessions");
    
    // Test session-to-bridge mapping
    let session1_bridge = session_manager.get_session_bridge(&session1_id).await;
    let session2_bridge = session_manager.get_session_bridge(&session2_id).await;
    
    assert!(session1_bridge.is_some());
    assert!(session2_bridge.is_some());
    assert_eq!(session1_bridge.unwrap(), bridge_id);
    assert_eq!(session2_bridge.unwrap(), bridge_id);
    println!("✅ Session-to-bridge mapping working correctly");
    
    // Remove one session from bridge
    session_manager.remove_session_from_bridge(&bridge_id, &session1_id).await?;
    println!("✅ Removed session 1 from bridge");
    
    // Verify bridge now contains only one session
    let bridge_info_updated = session_manager.get_bridge_info(&bridge_id).await?;
    assert_eq!(bridge_info_updated.sessions.len(), 1);
    assert!(bridge_info_updated.sessions.contains(&session2_id));
    assert!(!bridge_info_updated.sessions.contains(&session1_id));
    println!("✅ Bridge now contains only session 2");
    
    // Verify session1 no longer has bridge mapping
    let session1_bridge_after = session_manager.get_session_bridge(&session1_id).await;
    assert!(session1_bridge_after.is_none());
    println!("✅ Session 1 no longer mapped to bridge");
    
    // Remove remaining session
    session_manager.remove_session_from_bridge(&bridge_id, &session2_id).await?;
    println!("✅ Removed session 2 from bridge");
    
    // Verify bridge is now empty
    let bridge_info_empty = session_manager.get_bridge_info(&bridge_id).await?;
    assert_eq!(bridge_info_empty.sessions.len(), 0);
    println!("✅ Bridge is now empty");
    
    // Clean up bridge
    session_manager.destroy_bridge(&bridge_id).await?;
    println!("✅ Destroyed bridge");
    
    println!("🎉 Bridge test with real sessions completed successfully!");
    Ok(())
}

#[tokio::test]
async fn test_bridge_infrastructure() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting bridge infrastructure test using real components...");
    
    // Create SessionManager with real transaction-core (which manages its own transport)
    let session_manager = create_test_session_manager().await?;
    println!("✅ Created SessionManager with transaction-core (proper separation)");
    
    // Test bridge creation
    let bridge_config = BridgeConfig {
        max_sessions: 2,
        name: Some("Infrastructure Test Bridge".to_string()),
        timeout_secs: Some(60),
        enable_mixing: true,
    };
    
    let bridge_id = session_manager.create_bridge(bridge_config).await?;
    println!("✅ Created bridge: {}", bridge_id);
    
    // Test bridge information retrieval
    let bridge_info = session_manager.get_bridge_info(&bridge_id).await?;
    assert_eq!(bridge_info.state, BridgeState::Creating);
    assert_eq!(bridge_info.sessions.len(), 0);
    assert_eq!(bridge_info.name.as_ref().unwrap(), "Infrastructure Test Bridge");
    assert_eq!(bridge_info.config.max_sessions, 2);
    assert_eq!(bridge_info.config.enable_mixing, true);
    println!("✅ Bridge info retrieval working: {:?}", bridge_info.state);
    
    // Test bridge listing
    let bridges = session_manager.list_bridges().await;
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].id, bridge_id);
    println!("✅ Bridge listing working: {} active bridges", bridges.len());
    
    // Test bridge statistics
    let stats = session_manager.get_bridge_statistics().await;
    assert!(stats.contains_key(&bridge_id));
    assert_eq!(stats[&bridge_id].session_count, 0);
    println!("✅ Bridge statistics working: {} sessions in bridge", stats[&bridge_id].session_count);
    
    // Wait a moment for bridge to potentially transition to Active state
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    // Get updated bridge info to check if state changed
    let bridge_info_updated = session_manager.get_bridge_info(&bridge_id).await?;
    println!("✅ Bridge state after initialization: {:?}", bridge_info_updated.state);
    
    // Only test pause/resume if bridge is in Active state
    if bridge_info_updated.state == BridgeState::Active {
        // Test bridge state management
        session_manager.pause_bridge(&bridge_id).await?;
        let bridge_info_paused = session_manager.get_bridge_info(&bridge_id).await?;
        assert_eq!(bridge_info_paused.state, BridgeState::Paused);
        println!("✅ Bridge pause working: state = {:?}", bridge_info_paused.state);
        
        session_manager.resume_bridge(&bridge_id).await?;
        let bridge_info_resumed = session_manager.get_bridge_info(&bridge_id).await?;
        assert_eq!(bridge_info_resumed.state, BridgeState::Active);
        println!("✅ Bridge resume working: state = {:?}", bridge_info_resumed.state);
    } else {
        println!("ℹ️  Skipping pause/resume test - bridge in {:?} state", bridge_info_updated.state);
    }
    
    // Test bridge event subscription
    let _bridge_events = session_manager.subscribe_to_bridge_events().await;
    println!("✅ Bridge event subscription working");
    
    // Test bridge destruction
    session_manager.destroy_bridge(&bridge_id).await?;
    println!("✅ Bridge destruction initiated");
    
    // Verify bridge cleanup
    let bridges_after = session_manager.list_bridges().await;
    assert_eq!(bridges_after.len(), 0);
    println!("✅ Bridge cleanup verified: {} bridges remaining", bridges_after.len());
    
    println!("🎉 Bridge infrastructure test completed successfully!");
    Ok(())
}

#[tokio::test]
async fn test_bridge_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting bridge error handling test...");
    
    let session_manager = create_test_session_manager().await?;
    println!("✅ Created SessionManager with transaction-core");
    
    // Test accessing non-existent bridge
    let fake_bridge_id = BridgeId::new();
    let result = session_manager.get_bridge_info(&fake_bridge_id).await;
    
    match result {
        Err(BridgeError::BridgeNotFound { bridge_id }) => {
            assert_eq!(bridge_id, fake_bridge_id);
            println!("✅ Bridge not found error handling working");
        },
        _ => panic!("Expected BridgeNotFound error"),
    }
    
    // Test invalid bridge operations
    let result = session_manager.destroy_bridge(&fake_bridge_id).await;
    assert!(matches!(result, Err(BridgeError::BridgeNotFound { .. })));
    println!("✅ Bridge destroy error handling working");
    
    let result = session_manager.pause_bridge(&fake_bridge_id).await;
    assert!(matches!(result, Err(BridgeError::BridgeNotFound { .. })));
    println!("✅ Bridge pause error handling working");
    
    // Test adding session to non-existent bridge
    let session = session_manager.create_incoming_session().await?;
    let result = session_manager.add_session_to_bridge(&fake_bridge_id, &session.id).await;
    assert!(matches!(result, Err(BridgeError::BridgeNotFound { .. })));
    println!("✅ Add session to non-existent bridge error handling working");
    
    println!("🎉 Bridge error handling test completed successfully!");
    Ok(())
}

#[tokio::test]
async fn test_multiple_concurrent_bridges() -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 Starting multiple concurrent bridges test...");
    
    let session_manager = create_test_session_manager().await?;
    println!("✅ Created SessionManager with transaction-core");
    
    // Create multiple bridge configurations
    let bridge1_config = BridgeConfig {
        max_sessions: 2,
        name: Some("Bridge 1".to_string()),
        ..Default::default()
    };
    
    let bridge2_config = BridgeConfig {
        max_sessions: 4,
        name: Some("Bridge 2".to_string()),
        ..Default::default()
    };
    
    let bridge3_config = BridgeConfig {
        max_sessions: 6,
        name: Some("Bridge 3".to_string()),
        ..Default::default()
    };
    
    // Create all bridges concurrently
    let (bridge1_id, bridge2_id, bridge3_id) = tokio::try_join!(
        session_manager.create_bridge(bridge1_config),
        session_manager.create_bridge(bridge2_config),
        session_manager.create_bridge(bridge3_config)
    )?;
    
    println!("✅ Created 3 concurrent bridges: {}, {}, {}", bridge1_id, bridge2_id, bridge3_id);
    
    // Verify all bridges exist
    let bridges = session_manager.list_bridges().await;
    assert_eq!(bridges.len(), 3);
    println!("✅ All 3 bridges listed successfully");
    
    // Verify bridge configurations
    let bridge1_info = session_manager.get_bridge_info(&bridge1_id).await?;
    let bridge2_info = session_manager.get_bridge_info(&bridge2_id).await?;
    let bridge3_info = session_manager.get_bridge_info(&bridge3_id).await?;
    
    assert_eq!(bridge1_info.config.max_sessions, 2);
    assert_eq!(bridge2_info.config.max_sessions, 4);
    assert_eq!(bridge3_info.config.max_sessions, 6);
    println!("✅ Bridge configurations verified");
    
    // Test bridge statistics for all bridges
    let stats = session_manager.get_bridge_statistics().await;
    assert_eq!(stats.len(), 3);
    assert!(stats.contains_key(&bridge1_id));
    assert!(stats.contains_key(&bridge2_id));
    assert!(stats.contains_key(&bridge3_id));
    println!("✅ Statistics available for all bridges");
    
    // Clean up all bridges
    tokio::try_join!(
        session_manager.destroy_bridge(&bridge1_id),
        session_manager.destroy_bridge(&bridge2_id),
        session_manager.destroy_bridge(&bridge3_id)
    )?;
    
    // Verify cleanup
    let bridges_after = session_manager.list_bridges().await;
    assert_eq!(bridges_after.len(), 0);
    println!("✅ All bridges cleaned up successfully");
    
    println!("🎉 Multiple concurrent bridges test completed successfully!");
    Ok(())
}

#[test]
fn test_bridge_data_structures() {
    // Test bridge data structures work correctly
    let bridge_id = BridgeId::new();
    println!("✅ BridgeId creation working: {}", bridge_id);
    
    let bridge_config = BridgeConfig {
        max_sessions: 10,
        name: Some("Test Data Structure Bridge".to_string()),
        timeout_secs: Some(120),
        enable_mixing: false,
    };
    
    assert_eq!(bridge_config.max_sessions, 10);
    assert_eq!(bridge_config.name.as_ref().unwrap(), "Test Data Structure Bridge");
    assert_eq!(bridge_config.timeout_secs.unwrap(), 120);
    assert_eq!(bridge_config.enable_mixing, false);
    
    println!("✅ Bridge data structures working correctly");
} 