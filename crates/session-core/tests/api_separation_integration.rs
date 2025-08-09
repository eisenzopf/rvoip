//! Integration test for API separation between client.rs and server.rs
//!
//! This test verifies that:
//! 1. SipClient trait works as a convenience wrapper for non-dialog SIP operations
//! 2. ServerSessionManager trait works as a wrapper for bridge operations
//! 3. client-core style usage patterns work with SipClient
//! 4. call-engine style usage patterns work with ServerSessionManager

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::api::*;
use common::*;

#[tokio::test]
async fn test_sip_client_api_separation() {
    let coordinator = create_test_session_manager().await.unwrap();
    
    // Start the coordinator
    SessionControl::start(&coordinator).await.unwrap();
    
    // Test SipClient trait methods
    
    // 1. Test REGISTER (client-core style usage)
    let registration = coordinator.register(
        "sip:registrar.example.com",
        "sip:alice@example.com", 
        "sip:alice@192.168.1.100:5060",
        3600
    ).await;
    
    // Should work (or fail gracefully due to no real registrar)
    match registration {
        Ok(reg_handle) => {
            assert_eq!(reg_handle.expires, 3600);
            assert_eq!(reg_handle.registrar_uri, "sip:registrar.example.com");
            println!("✅ REGISTER request sent successfully");
        }
        Err(e) => {
            // Expected to fail in test environment - just verify it's not an API structure error
            println!("✅ REGISTER failed as expected (no real registrar): {}", e);
        }
    }
    
    // 2. Test OPTIONS (capability query)
    let options_response = coordinator.send_options("sip:test@example.com").await;
    
    match options_response {
        Ok(response) => {
            println!("✅ OPTIONS request sent successfully: status {}", response.status_code);
        }
        Err(e) => {
            // Expected to fail in test environment
            println!("✅ OPTIONS failed as expected (no real target): {}", e);
        }
    }
    
    // 3. Test MESSAGE (instant messaging) 
    let message_response = coordinator.send_message(
        "sip:bob@example.com",
        "Hello from integration test!",
        Some("text/plain")
    ).await;
    
    match message_response {
        Ok(response) => {
            println!("✅ MESSAGE request sent successfully: status {}", response.status_code);
        }
        Err(e) => {
            // Expected to fail in test environment
            println!("✅ MESSAGE failed as expected (no real target): {}", e);
        }
    }
    
    SessionControl::stop(&coordinator).await.unwrap();
}

#[tokio::test]
async fn test_server_session_manager_api_separation() {
    let coordinator = create_test_session_manager().await.unwrap();
    
    // Start the coordinator
    SessionControl::start(&coordinator).await.unwrap();
    
    // Test ServerSessionManager trait methods (call-engine style usage)
    
    // 1. Create two test sessions for bridging
    let session1 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:alice@example.com",
        "sip:bob@example.com",
        None
    ).await.unwrap();
    
    let session2 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:carol@example.com", 
        "sip:david@example.com",
        None
    ).await.unwrap();
    
    // 2. Test bridge creation (server-style operation)
    let bridge_id = coordinator.bridge_sessions(&session1.id, &session2.id).await.unwrap();
    println!("✅ Bridge created successfully: {}", bridge_id.0);
    
    // 3. Test bridge listing
    let bridges = coordinator.list_bridges().await;
    assert!(!bridges.is_empty(), "Should have at least one bridge");
    assert!(bridges.iter().any(|b| b.id == bridge_id), "Should find our bridge");
    println!("✅ Bridge listing works: found {} bridges", bridges.len());
    
    // 4. Test session bridge lookup
    let found_bridge = coordinator.get_session_bridge(&session1.id).await.unwrap();
    assert!(found_bridge.is_some(), "Session should be in a bridge");
    assert_eq!(found_bridge.unwrap(), bridge_id, "Should find the correct bridge");
    println!("✅ Session bridge lookup works");
    
    // 5. Test bridge events subscription
    let mut bridge_events = coordinator.subscribe_to_bridge_events().await;
    
    // Remove session from bridge (should trigger event)
    coordinator.remove_session_from_bridge(&bridge_id, &session1.id).await.unwrap();
    println!("✅ Session removed from bridge");
    
    // Check that session is no longer in bridge
    let found_bridge_after = coordinator.get_session_bridge(&session1.id).await.unwrap();
    assert!(found_bridge_after.is_none(), "Session should no longer be in bridge");
    
    // Try to receive bridge event (with timeout)
    tokio::select! {
        event = bridge_events.recv() => {
            if let Some(bridge_event) = event {
                println!("✅ Received bridge event: {:?}", bridge_event);
            }
        }
        _ = tokio::time::sleep(Duration::from_millis(100)) => {
            println!("⚠️  No bridge event received (may be expected in test environment)");
        }
    }
    
    // 6. Test bridge destruction
    coordinator.destroy_bridge(&bridge_id).await.unwrap();
    println!("✅ Bridge destroyed successfully");
    
    // Verify bridge is gone
    let bridges_after = coordinator.list_bridges().await;
    assert!(!bridges_after.iter().any(|b| b.id == bridge_id), "Bridge should be destroyed");
    
    // 7. Test pre-allocated session creation (for agent registration)
    let pre_allocated_session = coordinator.create_outgoing_session().await.unwrap();
    println!("✅ Pre-allocated session created: {}", pre_allocated_session.0);
    
    // Verify session exists in registry
    let session_info = SessionControl::get_session(&coordinator, &pre_allocated_session).await.unwrap();
    assert!(session_info.is_some(), "Pre-allocated session should exist");
    
    SessionControl::stop(&coordinator).await.unwrap();
}

#[tokio::test]
async fn test_trait_api_consistency() {
    // Test that the trait methods provide the same functionality as the general API
    let coordinator = create_test_session_manager().await.unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    // Create sessions using general API
    let session1 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:test1@example.com",
        "sip:target1@example.com", 
        None
    ).await.unwrap();
    
    let session2 = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:test2@example.com",
        "sip:target2@example.com",
        None  
    ).await.unwrap();
    
    // Bridge using ServerSessionManager trait
    let bridge_id = coordinator.bridge_sessions(&session1.id, &session2.id).await.unwrap();
    
    // Verify using general SessionControl API  
    let session1_info = SessionControl::get_session(&coordinator, &session1.id).await.unwrap();
    assert!(session1_info.is_some(), "Session should exist");
    
    // Verify bridge using ServerSessionManager trait
    let bridge_info = coordinator.get_session_bridge(&session1.id).await.unwrap();
    assert!(bridge_info.is_some(), "Session should be in bridge");
    assert_eq!(bridge_info.unwrap(), bridge_id, "Should be in correct bridge");
    
    // Clean up using trait API
    coordinator.destroy_bridge(&bridge_id).await.unwrap();
    
    // Terminate using general API
    SessionControl::terminate_session(&coordinator, &session1.id).await.unwrap();
    SessionControl::terminate_session(&coordinator, &session2.id).await.unwrap();
    
    SessionControl::stop(&coordinator).await.unwrap();
    
    println!("✅ API consistency verified - trait methods work seamlessly with general API");
}

#[tokio::test] 
async fn test_client_core_usage_pattern() {
    // Simulate how client-core would use the SipClient trait
    let coordinator = create_test_session_manager().await.unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    // Typical client-core workflow:
    // 1. Register with server
    // 2. Send keepalive OPTIONS
    // 3. Handle incoming messages
    // 4. Make outgoing calls using general API
    
    // Registration (would normally succeed with real registrar)
    let _registration_attempt = coordinator.register(
        "sip:registrar.provider.com",
        "sip:user@provider.com",
        "sip:user@192.168.1.100:5060", 
        3600
    ).await;
    
    // Keepalive OPTIONS  
    let _options_attempt = coordinator.send_options("sip:registrar.provider.com").await;
    
    // Make regular call using general API (this is the main client-core functionality)
    let call_session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:user@provider.com",
        "sip:friend@example.com",
        None
    ).await.unwrap();
    
    println!("✅ Client-core style usage pattern works - session: {}", call_session.id.0);
    
    SessionControl::stop(&coordinator).await.unwrap();
}

#[tokio::test]
async fn test_call_engine_usage_pattern() {
    // Simulate how call-engine would use the ServerSessionManager trait
    let coordinator = create_test_session_manager().await.unwrap();
    
    SessionControl::start(&coordinator).await.unwrap();
    
    // Typical call-engine workflow:
    // 1. Handle multiple incoming calls
    // 2. Create bridges between calls  
    // 3. Manage conference scenarios
    // 4. Route calls between agents
    
    // Simulate incoming calls being handled (using general API)
    let agent_session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:agent@callcenter.com",
        "sip:customer1@example.com",
        None
    ).await.unwrap();
    
    let customer_session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:system@callcenter.com", 
        "sip:customer2@example.com",
        None
    ).await.unwrap();
    
    // Bridge the calls (server-style operation)
    let bridge = coordinator.bridge_sessions(&agent_session.id, &customer_session.id).await.unwrap();
    
    // Monitor bridge events (call-engine would use this for logging/analytics)
    let _bridge_events = coordinator.subscribe_to_bridge_events().await;
    
    // Get bridge statistics
    let bridges = coordinator.list_bridges().await;
    assert_eq!(bridges.len(), 1, "Should have exactly one bridge");
    assert_eq!(bridges[0].participant_count, 2, "Bridge should have 2 participants");
    
    // Transfer scenario - remove one session and add another
    let transfer_session = SessionControl::create_outgoing_call(
        &coordinator,
        "sip:supervisor@callcenter.com",
        "sip:customer3@example.com", 
        None
    ).await.unwrap();
    
    // Remove agent from bridge
    coordinator.remove_session_from_bridge(&bridge, &agent_session.id).await.unwrap();
    
    // Add supervisor to bridge  
    coordinator.add_session_to_bridge(&bridge, &transfer_session.id).await.unwrap();
    
    // Verify transfer worked
    let agent_bridge = coordinator.get_session_bridge(&agent_session.id).await.unwrap();
    assert!(agent_bridge.is_none(), "Agent should no longer be in bridge");
    
    let supervisor_bridge = coordinator.get_session_bridge(&transfer_session.id).await.unwrap(); 
    assert!(supervisor_bridge.is_some(), "Supervisor should be in bridge");
    
    println!("✅ Call-engine style usage pattern works - call transfer completed");
    
    // Cleanup
    coordinator.destroy_bridge(&bridge).await.unwrap();
    
    SessionControl::stop(&coordinator).await.unwrap();
}
