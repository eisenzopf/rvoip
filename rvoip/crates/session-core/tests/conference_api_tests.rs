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
    SessionError,
};

async fn create_test_manager() -> ConferenceManager {
    ConferenceManager::new()
}

fn create_test_config() -> ConferenceConfig {
    ConferenceConfig {
        max_participants: 10,
        audio_mixing_enabled: true,
        audio_sample_rate: 8000,
        audio_channels: 1,
        rtp_port_range: Some((10000, 20000)),
        timeout: None,
        name: "Test Conference".to_string(),
    }
}

#[tokio::test]
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

#[tokio::test]
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

#[tokio::test]
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

#[tokio::test]
async fn test_join_nonexistent_conference() {
    let manager = create_test_manager().await;
    let conference_id = ConferenceId::new();
    let session_id = SessionId::new();
    
    let result = manager.join_conference(&conference_id, &session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
}

#[tokio::test]
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

#[tokio::test]
async fn test_capacity_limit() {
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