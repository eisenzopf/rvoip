//! Tests for conference features
//! 
//! These tests demonstrate multi-party conference functionality

use rvoip_session_core_v2::api::unified::{UnifiedCoordinator, Config};

/// Create a test configuration with unique ports
fn test_config(base_port: u16) -> Config {
    Config {
        sip_port: base_port,
        media_port_start: base_port + 1000,
        media_port_end: base_port + 2000,
        local_ip: "127.0.0.1".parse().unwrap(),
        bind_addr: format!("127.0.0.1:{}", base_port).parse().unwrap(),
        state_table_path: None,
        local_uri: format!("sip:test@127.0.0.1:{}", base_port),
    }
}

#[tokio::test]
async fn test_create_conference_from_active_call() {
    let coordinator = UnifiedCoordinator::new(test_config(15300)).await.unwrap();
    
    // Make initial call
    let host_session = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant1@localhost:15301"
    ).await.unwrap();
    
    // Create conference from the call
    let result = coordinator.create_conference(&host_session, "Daily Standup").await;
    assert!(result.is_ok());
    
    // Verify host is in conference state
    let is_in_conf = coordinator.is_in_conference(&host_session).await.unwrap();
    assert!(is_in_conf);
}

#[tokio::test]
async fn test_add_multiple_participants() {
    let coordinator = UnifiedCoordinator::new(test_config(15302)).await.unwrap();
    
    // Create host session and conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant1@localhost:15303"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Team Meeting").await.unwrap();
    
    // Add multiple participants
    let participant2 = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant2@localhost:15304"
    ).await.unwrap();
    
    let participant3 = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant3@localhost:15305"
    ).await.unwrap();
    
    // Add them to conference
    assert!(coordinator.add_to_conference(&host, &participant2).await.is_ok());
    assert!(coordinator.add_to_conference(&host, &participant3).await.is_ok());
}

#[tokio::test]
async fn test_conference_with_late_joiners() {
    let coordinator = UnifiedCoordinator::new(test_config(15306)).await.unwrap();
    
    // Start conference with initial participants
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:early_bird@localhost:15307"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Planning Session").await.unwrap();
    
    // Simulate time passing...
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    
    // Late joiner arrives
    let late_joiner = coordinator.make_call(
        "sip:host@localhost",
        "sip:late_joiner@localhost:15308"
    ).await.unwrap();
    
    // Add late joiner
    let result = coordinator.add_to_conference(&host, &late_joiner).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_leave_and_rejoin_conference() {
    let coordinator = UnifiedCoordinator::new(test_config(15309)).await.unwrap();
    
    // Create conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant@localhost:15310"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Recurring Meeting").await.unwrap();
    
    // Add participant
    let participant = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant2@localhost:15311"
    ).await.unwrap();
    
    coordinator.add_to_conference(&host, &participant).await.unwrap();
    
    // Participant leaves (hangs up)
    coordinator.hangup(&participant).await.unwrap();
    
    // Participant rejoins with new call
    let participant_new = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant2@localhost:15311"
    ).await.unwrap();
    
    // Add back to conference
    let result = coordinator.add_to_conference(&host, &participant_new).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_conference_with_hold() {
    let coordinator = UnifiedCoordinator::new(test_config(15312)).await.unwrap();
    
    // Create conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant@localhost:15313"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Board Meeting").await.unwrap();
    
    // Host puts conference on hold
    let hold_result = coordinator.hold(&host).await;
    assert!(hold_result.is_ok());
    
    // Resume conference
    let resume_result = coordinator.resume(&host).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
async fn test_conference_recording() {
    let coordinator = UnifiedCoordinator::new(test_config(15314)).await.unwrap();
    
    // Create conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant@localhost:15315"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Important Meeting").await.unwrap();
    
    // Add participants
    let p1 = coordinator.make_call(
        "sip:host@localhost",
        "sip:p1@localhost:15316"
    ).await.unwrap();
    
    let p2 = coordinator.make_call(
        "sip:host@localhost",
        "sip:p2@localhost:15317"
    ).await.unwrap();
    
    coordinator.add_to_conference(&host, &p1).await.unwrap();
    coordinator.add_to_conference(&host, &p2).await.unwrap();
    
    // Start recording the conference
    let record_result = coordinator.start_recording(&host).await;
    assert!(record_result.is_ok());
    
    // Stop recording
    let stop_result = coordinator.stop_recording(&host).await;
    assert!(stop_result.is_ok());
}

#[tokio::test]
async fn test_conference_dtmf() {
    let coordinator = UnifiedCoordinator::new(test_config(15318)).await.unwrap();
    
    // Create conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:participant@localhost:15319"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Phone Conference").await.unwrap();
    
    // Send DTMF in conference (e.g., for conference controls)
    let dtmf_result = coordinator.send_dtmf(&host, '#').await;
    assert!(dtmf_result.is_ok());
    
    // Mute command
    let mute_dtmf = coordinator.send_dtmf(&host, '*').await;
    assert!(mute_dtmf.is_ok());
}

#[tokio::test]
async fn test_large_conference() {
    let coordinator = UnifiedCoordinator::new(test_config(15320)).await.unwrap();
    
    // Create conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:p1@localhost:15321"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "All Hands").await.unwrap();
    
    // Add many participants
    let mut participants = Vec::new();
    for i in 0..5 {
        let p = coordinator.make_call(
            "sip:host@localhost",
            &format!("sip:p{}@localhost:{}", i+2, 15322 + i)
        ).await.unwrap();
        
        coordinator.add_to_conference(&host, &p).await.unwrap();
        participants.push(p);
    }
    
    // Verify all are in conference
    assert!(coordinator.is_in_conference(&host).await.unwrap());
    for p in &participants {
        assert!(coordinator.is_in_conference(p).await.unwrap_or(false));
    }
    
    // Clean up
    coordinator.hangup(&host).await.unwrap();
}

#[tokio::test]
async fn test_conference_error_cases() {
    let coordinator = UnifiedCoordinator::new(test_config(15327)).await.unwrap();
    
    // Try to create conference without active call
    let fake_session = rvoip_session_core_v2::state_table::types::SessionId::new();
    let result = coordinator.create_conference(&fake_session, "Invalid Conference").await;
    assert!(result.is_err());
    
    // Try to add non-existent session to conference
    let host = coordinator.make_call(
        "sip:host@localhost",
        "sip:p@localhost:15328"
    ).await.unwrap();
    
    coordinator.create_conference(&host, "Test").await.unwrap();
    
    let result = coordinator.add_to_conference(&host, &fake_session).await;
    assert!(result.is_err());
}
