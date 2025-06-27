use rvoip_session_core::api::control::SessionControl;
// Conference Room Tests
//
// Tests for ConferenceRoom participant management and state transitions.

use std::time::Duration;
use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        room::ConferenceRoom,
        participant::ConferenceParticipant,
        types::*,
    },
    SessionError,
};

fn create_test_config() -> ConferenceConfig {
    ConferenceConfig {
        max_participants: 5,
        audio_mixing_enabled: true,
        audio_sample_rate: 8000,
        audio_channels: 1,
        rtp_port_range: Some((10000, 20000)),
        timeout: None,
        name: "Test Conference Room".to_string(),
    }
}

fn create_test_participant(session_id: SessionId) -> ConferenceParticipant {
    ConferenceParticipant::new(
        session_id.clone(),
        format!("sip:test_{}@example.com", session_id.as_str())
    )
}

#[tokio::test]
async fn test_room_creation() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    
    let room = ConferenceRoom::new(conference_id.clone(), config.clone());
    
    assert_eq!(room.id, conference_id);
    assert_eq!(room.config.max_participants, config.max_participants);
    assert_eq!(room.state, ConferenceState::Creating);
    assert_eq!(room.participants.len(), 0);
}

#[tokio::test]
async fn test_add_remove_participants() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    let session_id = SessionId::new();
    let participant = create_test_participant(session_id.clone());
    
    // Add participant
    room.add_participant(participant).unwrap();
    assert_eq!(room.participants.len(), 1);
    
    // Verify participant exists
    let retrieved = room.get_participant(&session_id);
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().session_id, session_id);
    
    // Remove participant
    let removed = room.remove_participant(&session_id);
    assert!(removed.is_some());
    assert_eq!(room.participants.len(), 0);
}

#[tokio::test]
async fn test_capacity_limits() {
    let conference_id = ConferenceId::new();
    let mut config = create_test_config();
    config.max_participants = 2; // Small limit for testing
    
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Add participants up to limit
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    room.add_participant(create_test_participant(session1)).unwrap();
    room.add_participant(create_test_participant(session2)).unwrap();
    
    // Try to add one more - should fail
    let session3 = SessionId::new();
    let result = room.add_participant(create_test_participant(session3));
    assert!(matches!(result, Err(SessionError::ResourceLimitExceeded(_))));
}

#[tokio::test]
async fn test_state_transitions() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Valid transitions
    assert_eq!(room.state, ConferenceState::Creating);
    
    room.set_state(ConferenceState::Active).unwrap();
    assert_eq!(room.state, ConferenceState::Active);
    
    room.set_state(ConferenceState::Locked).unwrap();
    assert_eq!(room.state, ConferenceState::Locked);
    
    room.set_state(ConferenceState::Terminating).unwrap();
    assert_eq!(room.state, ConferenceState::Terminating);
    
    room.set_state(ConferenceState::Terminated).unwrap();
    assert_eq!(room.state, ConferenceState::Terminated);
} 