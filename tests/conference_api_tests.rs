//! Conference API Tests
//!
//! Tests for the ConferenceApi trait and its default implementations.

use std::sync::Arc;
use tokio::test;
use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        api::{ConferenceApi, ConferenceApiExt},
        manager::ConferenceManager,
        types::*,
    },
    errors::SessionError,
};

async fn create_test_manager() -> ConferenceManager {
    ConferenceManager::new()
}

fn create_test_config() -> ConferenceConfig {
    ConferenceConfig {
        max_participants: 10,
        audio_mixing_enabled: true,
        recording_enabled: false,
        auto_terminate_timeout: std::time::Duration::from_secs(300),
    }
}

#[test]
async fn test_create_conference() {
    let manager = create_test_manager().await;
    let config = create_test_config();

    let conference_id = manager.create_conference(config.clone()).await.unwrap();
    
    // Verify conference was created
    assert!(manager.conference_exists(&conference_id).await);
    
    // Verify configuration
    let stored_config = manager.get_conference_config(&conference_id).await.unwrap();
    assert_eq!(stored_config.max_participants, config.max_participants);
    assert_eq!(stored_config.audio_mixing_enabled, config.audio_mixing_enabled);
}

#[test]
async fn test_create_named_conference() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = ConferenceId::new();

    // Create named conference
    manager.create_named_conference(conference_id.clone(), config.clone()).await.unwrap();
    
    // Verify conference exists
    assert!(manager.conference_exists(&conference_id).await);
    
    // Try to create same conference again - should fail
    let result = manager.create_named_conference(conference_id.clone(), config).await;
    assert!(matches!(result, Err(SessionError::InvalidState(_))));
}

#[test]
async fn test_join_leave_conference() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    let session_id = SessionId::new();
    
    // Join conference
    let participant_info = manager.join_conference(&conference_id, &session_id).await.unwrap();
    assert_eq!(participant_info.session_id, session_id);
    assert_eq!(participant_info.status, ParticipantStatus::Joining);
    
    // Verify participant is listed
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].session_id, session_id);
    
    // Leave conference
    manager.leave_conference(&conference_id, &session_id).await.unwrap();
    
    // Verify participant is removed
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 0);
}

#[test]
async fn test_join_nonexistent_conference() {
    let manager = create_test_manager().await;
    let conference_id = ConferenceId::new();
    let session_id = SessionId::new();
    
    let result = manager.join_conference(&conference_id, &session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
}

#[test]
async fn test_leave_nonexistent_participant() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    let result = manager.leave_conference(&conference_id, &session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
}

#[test]
async fn test_list_conferences() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    
    // Initially no conferences
    let conferences = manager.list_conferences().await.unwrap();
    assert_eq!(conferences.len(), 0);
    
    // Create conferences
    let conf1 = manager.create_conference(config.clone()).await.unwrap();
    let conf2 = manager.create_conference(config.clone()).await.unwrap();
    
    // Verify both listed
    let conferences = manager.list_conferences().await.unwrap();
    assert_eq!(conferences.len(), 2);
    assert!(conferences.contains(&conf1));
    assert!(conferences.contains(&conf2));
}

#[test]
async fn test_get_conference_stats() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Initial stats
    let stats = manager.get_conference_stats(&conference_id).await.unwrap();
    assert_eq!(stats.total_participants, 0);
    assert_eq!(stats.active_participants, 0);
    assert_eq!(stats.state, ConferenceState::Creating);
    
    // Add participant
    let session_id = SessionId::new();
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Updated stats
    let stats = manager.get_conference_stats(&conference_id).await.unwrap();
    assert_eq!(stats.total_participants, 1);
}

#[test]
async fn test_terminate_conference() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Add participant
    let session_id = SessionId::new();
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Terminate conference
    manager.terminate_conference(&conference_id).await.unwrap();
    
    // Verify conference is removed
    assert!(!manager.conference_exists(&conference_id).await);
    
    // Verify can't join terminated conference
    let session_id2 = SessionId::new();
    let result = manager.join_conference(&conference_id, &session_id2).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
}

#[test]
async fn test_update_participant_status() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    // Join conference
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Update status
    manager.update_participant_status(&conference_id, &session_id, ParticipantStatus::Active).await.unwrap();
    
    // Verify status updated
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants[0].status, ParticipantStatus::Active);
    
    // Update to muted
    manager.update_participant_status(&conference_id, &session_id, ParticipantStatus::Muted).await.unwrap();
    
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants[0].status, ParticipantStatus::Muted);
}

#[test]
async fn test_generate_conference_sdp() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    // Join conference
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Generate SDP
    let sdp = manager.generate_conference_sdp(&conference_id, &session_id).await.unwrap();
    
    // Verify SDP contains expected elements
    assert!(sdp.contains("v=0"));
    assert!(sdp.contains("o=conference_"));
    assert!(sdp.contains("s=Conference Room"));
    assert!(sdp.contains("m=audio"));
    assert!(sdp.contains("a=rtpmap:0 PCMU/8000"));
    assert!(sdp.contains("a=rtpmap:8 PCMA/8000"));
    assert!(sdp.contains("a=conf:audio-mixing")); // Audio mixing enabled
}

#[test]
async fn test_update_conference_config() {
    let manager = create_test_manager().await;
    let mut config = create_test_config();
    let conference_id = manager.create_conference(config.clone()).await.unwrap();
    
    // Update configuration
    config.max_participants = 20;
    config.audio_mixing_enabled = false;
    manager.update_conference_config(&conference_id, config.clone()).await.unwrap();
    
    // Verify configuration updated
    let stored_config = manager.get_conference_config(&conference_id).await.unwrap();
    assert_eq!(stored_config.max_participants, 20);
    assert_eq!(stored_config.audio_mixing_enabled, false);
}

#[test]
async fn test_conference_api_ext_mute_all() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Add multiple participants
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    manager.join_conference(&conference_id, &session1).await.unwrap();
    manager.join_conference(&conference_id, &session2).await.unwrap();
    
    // Set them active first
    manager.update_participant_status(&conference_id, &session1, ParticipantStatus::Active).await.unwrap();
    manager.update_participant_status(&conference_id, &session2, ParticipantStatus::Active).await.unwrap();
    
    // Mute all participants
    manager.mute_all_participants(&conference_id).await.unwrap();
    
    // Verify all are muted
    let participants = manager.list_participants(&conference_id).await.unwrap();
    for participant in participants {
        assert_eq!(participant.status, ParticipantStatus::Muted);
    }
}

#[test]
async fn test_conference_api_ext_kick_participant() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    let session_id = SessionId::new();
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Kick participant
    manager.kick_participant(&conference_id, &session_id, "Removed by admin").await.unwrap();
    
    // Verify participant is removed
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 0);
}

#[test]
async fn test_conference_capacity_limit() {
    let manager = create_test_manager().await;
    let mut config = create_test_config();
    config.max_participants = 2; // Small limit for testing
    
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Add participants up to limit
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    manager.join_conference(&conference_id, &session1).await.unwrap();
    manager.join_conference(&conference_id, &session2).await.unwrap();
    
    // Try to add one more - should fail
    let session3 = SessionId::new();
    let result = manager.join_conference(&conference_id, &session3).await;
    assert!(matches!(result, Err(SessionError::ResourceLimitExceeded(_))));
}

#[test]
async fn test_multiple_conferences_isolation() {
    let manager = create_test_manager().await;
    let config = create_test_config();
    
    // Create two conferences
    let conf1 = manager.create_conference(config.clone()).await.unwrap();
    let conf2 = manager.create_conference(config).await.unwrap();
    
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    
    // Add different participants to each conference
    manager.join_conference(&conf1, &session1).await.unwrap();
    manager.join_conference(&conf2, &session2).await.unwrap();
    
    // Verify isolation
    let participants1 = manager.list_participants(&conf1).await.unwrap();
    let participants2 = manager.list_participants(&conf2).await.unwrap();
    
    assert_eq!(participants1.len(), 1);
    assert_eq!(participants2.len(), 1);
    assert_eq!(participants1[0].session_id, session1);
    assert_eq!(participants2[0].session_id, session2);
} 