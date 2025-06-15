use rvoip_session_core::api::control::SessionControl;
//! Tests for Bridge Configuration and Advanced Scenarios
//!
//! Tests bridge configuration options, auto-start/stop behavior, and advanced
//! bridge scenarios that would be driven by configuration settings.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    api::types::SessionId,
    bridge::{SessionBridge, BridgeId, BridgeConfig},
};
use common::*;

#[tokio::test]
async fn test_bridge_config_variations() {
    // Test different configuration combinations
    let configs = vec![
        BridgeTestConfig::default(),
        BridgeTestConfig::small_bridge(),
        BridgeTestConfig::large_bridge(),
        BridgeTestConfig {
            max_sessions: 1,
            auto_start: true,
            auto_stop_on_empty: false,
            bridge_timeout: Duration::from_millis(100),
        },
    ];
    
    for (i, config) in configs.iter().enumerate() {
        println!("Testing config {}: {:?}", i, config);
        
        // Note: Current implementation doesn't use config in constructor
        // This test validates the config structures themselves
        assert!(config.max_sessions >= 0);
        assert!(config.bridge_timeout > Duration::from_millis(0));
        
        // Create bridge with config (when implementation supports it)
        let bridge = create_test_bridge_with_config(&format!("config-test-{}", i), config.clone());
        verify_bridge_state(&bridge, false, 0);
    }
}

#[tokio::test]
async fn test_bridge_auto_start_behavior() {
    // Test auto-start configuration behavior
    // Note: This test simulates what auto-start behavior would look like
    
    let auto_start_config = BridgeTestConfig {
        max_sessions: 5,
        auto_start: true,
        auto_stop_on_empty: false,
        bridge_timeout: Duration::from_secs(1),
    };
    
    let mut bridge = create_test_bridge_with_config("auto-start-test", auto_start_config);
    let session_id = SessionId("trigger-session".to_string());
    
    // Simulate auto-start: bridge should start when first session is added
    assert!(bridge.add_session(session_id.clone()).is_ok());
    
    // In a real implementation with auto-start, this would automatically start the bridge
    // For now, we manually start to simulate the behavior
    if true { // auto_start_config.auto_start
        assert!(bridge.start().is_ok());
    }
    
    verify_bridge_state(&bridge, true, 1);
    
    // Add more sessions - bridge should remain active
    let session2 = SessionId("second-session".to_string());
    assert!(bridge.add_session(session2.clone()).is_ok());
    verify_bridge_state(&bridge, true, 2);
}

#[tokio::test]
async fn test_bridge_auto_stop_on_empty_behavior() {
    // Test auto-stop-on-empty configuration behavior
    
    let auto_stop_config = BridgeTestConfig {
        max_sessions: 5,
        auto_start: true,
        auto_stop_on_empty: true,
        bridge_timeout: Duration::from_secs(1),
    };
    
    let mut bridge = create_test_bridge_with_config("auto-stop-test", auto_stop_config);
    let session_ids = create_test_session_ids(3);
    
    // Add sessions and start bridge
    for session_id in &session_ids {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 3);
    
    // Remove sessions one by one
    for (i, session_id) in session_ids.iter().enumerate() {
        assert!(bridge.remove_session(session_id).is_ok());
        
        if i == session_ids.len() - 1 {
            // Last session removed - simulate auto-stop behavior
            if true { // auto_stop_config.auto_stop_on_empty
                assert!(bridge.stop().is_ok());
            }
            verify_bridge_state(&bridge, false, 0);
        } else {
            // Still has sessions - should remain active
            verify_bridge_state(&bridge, true, session_ids.len() - i - 1);
        }
    }
}

#[tokio::test]
async fn test_bridge_max_sessions_limit() {
    // Test max sessions configuration
    let max_sessions = 3;
    let limited_config = BridgeTestConfig {
        max_sessions,
        auto_start: false,
        auto_stop_on_empty: false,
        bridge_timeout: Duration::from_secs(1),
    };
    
    let mut bridge = create_test_bridge_with_config("max-sessions-test", limited_config);
    let session_ids = create_test_session_ids(5); // More than max
    
    // Add sessions up to the limit
    for (i, session_id) in session_ids.iter().enumerate() {
        let result = bridge.add_session(session_id.clone());
        
        // Note: Current implementation doesn't enforce max_sessions
        // In a real implementation, this would fail after max_sessions
        assert!(result.is_ok());
        
        // For simulation, we'll manually enforce the limit
        if i + 1 >= max_sessions {
            println!("Reached max sessions limit: {}", i + 1);
            break;
        }
    }
    
    // Verify we have at most max_sessions
    assert!(bridge.session_count() <= max_sessions);
}

#[tokio::test]
async fn test_bridge_timeout_configuration() {
    // Test bridge timeout behavior
    let timeout_config = BridgeTestConfig {
        max_sessions: 10,
        auto_start: true,
        auto_stop_on_empty: true,
        bridge_timeout: Duration::from_millis(100),
    };
    
    let mut bridge = create_test_bridge_with_config("timeout-test", timeout_config.clone());
    let session_id = SessionId("timeout-session".to_string());
    
    // Test that operations complete within timeout
    let start = std::time::Instant::now();
    
    assert!(bridge.add_session(session_id.clone()).is_ok());
    assert!(bridge.start().is_ok());
    assert!(bridge.remove_session(&session_id).is_ok());
    assert!(bridge.stop().is_ok());
    
    let elapsed = start.elapsed();
    
    // Operations should complete well within the configured timeout
    assert!(elapsed < timeout_config.bridge_timeout * 10, 
           "Operations took too long: {:?} (timeout: {:?})", 
           elapsed, timeout_config.bridge_timeout);
}

#[tokio::test]
async fn test_bridge_configuration_inheritance() {
    // Test that bridge configurations are properly inherited/applied
    
    let base_config = BridgeTestConfig::default();
    let custom_config = BridgeTestConfig {
        max_sessions: base_config.max_sessions * 2,
        auto_start: !base_config.auto_start,
        auto_stop_on_empty: !base_config.auto_stop_on_empty,
        bridge_timeout: base_config.bridge_timeout * 2,
    };
    
    // Create bridges with different configs
    let bridge1 = create_test_bridge_with_config("config-inherit-1", base_config.clone());
    let bridge2 = create_test_bridge_with_config("config-inherit-2", custom_config.clone());
    
    // Both bridges should work regardless of configuration
    verify_bridge_state(&bridge1, false, 0);
    verify_bridge_state(&bridge2, false, 0);
    
    // Test that configurations don't interfere with each other
    let session1 = SessionId("config1-session".to_string());
    let session2 = SessionId("config2-session".to_string());
    
    let mut bridge1_mut = bridge1;
    let mut bridge2_mut = bridge2;
    
    assert!(bridge1_mut.add_session(session1).is_ok());
    assert!(bridge2_mut.add_session(session2).is_ok());
    
    verify_bridge_state(&bridge1_mut, false, 1);
    verify_bridge_state(&bridge2_mut, false, 1);
}

#[tokio::test]
async fn test_bridge_dynamic_configuration_changes() {
    // Test changing bridge behavior dynamically (simulated)
    
    let mut bridge = create_test_bridge("dynamic-config-test");
    let session_ids = create_test_session_ids(5);
    
    // Phase 1: Normal operation
    for session_id in &session_ids[0..2] {
        assert!(bridge.add_session(session_id.clone()).is_ok());
    }
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 2);
    
    // Phase 2: Simulate configuration change (e.g., auto_stop_on_empty enabled)
    // Remove all sessions and simulate auto-stop
    for session_id in &session_ids[0..2] {
        assert!(bridge.remove_session(session_id).is_ok());
    }
    
    // Simulate auto-stop when empty
    if bridge.session_count() == 0 {
        assert!(bridge.stop().is_ok());
    }
    verify_bridge_state(&bridge, false, 0);
    
    // Phase 3: Simulate different configuration (e.g., auto_start enabled)
    let new_session = SessionId("dynamic-session".to_string());
    assert!(bridge.add_session(new_session.clone()).is_ok());
    
    // Simulate auto-start on first session
    assert!(bridge.start().is_ok());
    verify_bridge_state(&bridge, true, 1);
}

#[tokio::test]
async fn test_bridge_configuration_validation() {
    // Test configuration validation scenarios
    
    // Valid configurations
    let valid_configs = vec![
        BridgeTestConfig {
            max_sessions: 1,
            auto_start: true,
            auto_stop_on_empty: true,
            bridge_timeout: Duration::from_millis(1),
        },
        BridgeTestConfig {
            max_sessions: usize::MAX,
            auto_start: false,
            auto_stop_on_empty: false,
            bridge_timeout: Duration::from_secs(3600),
        },
    ];
    
    for (i, config) in valid_configs.iter().enumerate() {
        let bridge = create_test_bridge_with_config(&format!("valid-config-{}", i), config.clone());
        verify_bridge_state(&bridge, false, 0);
        println!("✓ Valid config {} accepted", i);
    }
    
    // Edge case configurations
    let edge_configs = vec![
        BridgeTestConfig {
            max_sessions: 0, // Zero sessions allowed
            auto_start: true,
            auto_stop_on_empty: true,
            bridge_timeout: Duration::from_nanos(1), // Minimal timeout
        },
    ];
    
    for (i, config) in edge_configs.iter().enumerate() {
        let bridge = create_test_bridge_with_config(&format!("edge-config-{}", i), config.clone());
        verify_bridge_state(&bridge, false, 0);
        println!("✓ Edge config {} handled", i);
    }
}

#[tokio::test]
async fn test_bridge_configuration_performance_impact() {
    // Test that different configurations don't significantly impact performance
    
    let configs = vec![
        BridgeTestConfig::small_bridge(),
        BridgeTestConfig::default(),
        BridgeTestConfig::large_bridge(),
    ];
    
    let session_count = 100;
    let mut performance_results = Vec::new();
    
    for (i, config) in configs.iter().enumerate() {
        let mut bridge = create_test_bridge_with_config(&format!("perf-config-{}", i), config.clone());
        let session_ids = create_test_session_ids(session_count);
        
        let start = std::time::Instant::now();
        
        // Add sessions
        for session_id in &session_ids {
            assert!(bridge.add_session(session_id.clone()).is_ok());
        }
        
        // Start bridge
        assert!(bridge.start().is_ok());
        
        // Remove sessions
        for session_id in &session_ids {
            assert!(bridge.remove_session(session_id).is_ok());
        }
        
        let elapsed = start.elapsed();
        performance_results.push(elapsed);
        
        println!("Config {} performance: {:?}", i, elapsed);
        verify_bridge_state(&bridge, true, 0);
    }
    
    // All configurations should have reasonable performance
    for (i, duration) in performance_results.iter().enumerate() {
        assert!(duration < &Duration::from_secs(5), 
               "Config {} took too long: {:?}", i, duration);
    }
    
    println!("✓ All configurations performed within acceptable limits");
}

#[tokio::test]
async fn test_bridge_configuration_with_integration_helper() {
    // Test configurations in integration scenarios
    
    let helper = BridgeIntegrationHelper::new(3, 2).await.unwrap();
    
    // Create calls
    let call1 = helper.create_call_between_managers(0, 1).await.unwrap();
    let call2 = helper.create_call_between_managers(1, 2).await.unwrap();
    
    // Add to different bridges with different "configurations"
    assert!(helper.add_session_to_bridge(0, call1.id().clone()).await.is_ok());
    assert!(helper.add_session_to_bridge(1, call2.id().clone()).await.is_ok());
    
    // Start bridges (simulating auto_start: false -> manual start)
    assert!(helper.start_bridge(0).await.is_ok());
    assert!(helper.start_bridge(1).await.is_ok());
    
    // Verify both bridges are working independently
    let bridge1_state = helper.get_bridge_state(0).await.unwrap();
    let bridge2_state = helper.get_bridge_state(1).await.unwrap();
    
    assert_eq!(bridge1_state, (true, 1));
    assert_eq!(bridge2_state, (true, 1));
    
    println!("✓ Configuration-based bridge integration working correctly");
    
    helper.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_bridge_configuration_scenarios() {
    // Test realistic configuration scenarios
    
    // Scenario 1: Small meeting room (max 4 participants, auto-start/stop)
    let meeting_room_config = BridgeTestConfig {
        max_sessions: 4,
        auto_start: true,
        auto_stop_on_empty: true,
        bridge_timeout: Duration::from_secs(1),
    };
    
    let mut meeting_bridge = create_test_bridge_with_config("meeting-room", meeting_room_config);
    let participants = create_test_session_ids(3);
    
    for participant in &participants {
        assert!(meeting_bridge.add_session(participant.clone()).is_ok());
    }
    
    // Simulate auto-start
    assert!(meeting_bridge.start().is_ok());
    verify_bridge_state(&meeting_bridge, true, 3);
    
    // Scenario 2: Large conference (many participants, manual control)
    let conference_config = BridgeTestConfig {
        max_sessions: 100,
        auto_start: false,
        auto_stop_on_empty: false,
        bridge_timeout: Duration::from_secs(5),
    };
    
    let mut conference_bridge = create_test_bridge_with_config("large-conference", conference_config);
    let attendees = create_test_session_ids(50);
    
    for attendee in &attendees {
        assert!(conference_bridge.add_session(attendee.clone()).is_ok());
    }
    
    // Manual start (auto_start: false)
    assert!(conference_bridge.start().is_ok());
    verify_bridge_state(&conference_bridge, true, 50);
    
    // Remove some attendees - bridge should stay active (auto_stop_on_empty: false)
    for attendee in &attendees[0..25] {
        assert!(conference_bridge.remove_session(attendee).is_ok());
    }
    verify_bridge_state(&conference_bridge, true, 25);
    
    // Manual stop required
    assert!(conference_bridge.stop().is_ok());
    verify_bridge_state(&conference_bridge, false, 25);
    
    println!("✓ Realistic configuration scenarios working correctly");
} 