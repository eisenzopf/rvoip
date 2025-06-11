//! Conference Room Tests
//!
//! Tests for ConferenceRoom participant management and state transitions.

use std::time::Duration;
use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        room::ConferenceRoom,
        participant::ConferenceParticipant,
        types::*,
    },
    errors::SessionError,
};

fn create_test_config() -> ConferenceConfig {
    ConferenceConfig {
        max_participants: 5,
        audio_mixing_enabled: true,
        recording_enabled: false,
        auto_terminate_timeout: Duration::from_secs(300),
    }
}

fn create_test_participant(session_id: SessionId) -> ConferenceParticipant {
    ConferenceParticipant::new(
        session_id,
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

#[tokio::test]
async fn test_invalid_state_transitions() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Try invalid transition: Creating -> Terminating (skipping Active)
    let result = room.set_state(ConferenceState::Terminating);
    assert!(matches!(result, Err(SessionError::InvalidState(_))));
    
    // Try invalid transition: Creating -> Terminated
    let result = room.set_state(ConferenceState::Terminated);
    assert!(matches!(result, Err(SessionError::InvalidState(_))));
}

#[tokio::test]
async fn test_participant_status_updates() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    let session_id = SessionId::new();
    let participant = create_test_participant(session_id.clone());
    room.add_participant(participant).unwrap();
    
    // Update status
    room.update_participant_status(&session_id, ParticipantStatus::Active).unwrap();
    
    let participant = room.get_participant(&session_id).unwrap();
    assert_eq!(participant.status, ParticipantStatus::Active);
    
    // Update to muted
    room.update_participant_status(&session_id, ParticipantStatus::Muted).unwrap();
    
    let participant = room.get_participant(&session_id).unwrap();
    assert_eq!(participant.status, ParticipantStatus::Muted);
}

#[tokio::test]
async fn test_audio_management() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    let session_id = SessionId::new();
    let participant = create_test_participant(session_id.clone());
    room.add_participant(participant).unwrap();
    
    // Initially audio is inactive
    let participant = room.get_participant(&session_id).unwrap();
    assert!(!participant.audio_active);
    
    // Activate audio
    room.set_participant_audio(&session_id, true).unwrap();
    
    let participant = room.get_participant(&session_id).unwrap();
    assert!(participant.audio_active);
    
    // Deactivate audio
    room.set_participant_audio(&session_id, false).unwrap();
    
    let participant = room.get_participant(&session_id).unwrap();
    assert!(!participant.audio_active);
}

#[tokio::test]
async fn test_rtp_port_assignment() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    let session_id = SessionId::new();
    let participant = create_test_participant(session_id.clone());
    room.add_participant(participant).unwrap();
    
    // Initially no RTP port
    let participant = room.get_participant(&session_id).unwrap();
    assert!(participant.rtp_port.is_none());
    
    // Assign RTP port
    room.set_participant_rtp_port(&session_id, 12345).unwrap();
    
    let participant = room.get_participant(&session_id).unwrap();
    assert_eq!(participant.rtp_port, Some(12345));
}

#[tokio::test]
async fn test_room_statistics() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Initial stats
    let stats = room.get_stats();
    assert_eq!(stats.total_participants, 0);
    assert_eq!(stats.active_participants, 0);
    assert_eq!(stats.audio_participants, 0);
    
    // Add participants
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    room.add_participant(create_test_participant(session1.clone())).unwrap();
    room.add_participant(create_test_participant(session2.clone())).unwrap();
    
    // Set one active with audio
    room.update_participant_status(&session1, ParticipantStatus::Active).unwrap();
    room.set_participant_audio(&session1, true).unwrap();
    
    let stats = room.get_stats();
    assert_eq!(stats.total_participants, 2);
    assert_eq!(stats.active_participants, 1);
    assert_eq!(stats.audio_participants, 1);
}

#[tokio::test]
async fn test_media_readiness() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Not ready initially
    assert!(!room.is_media_ready());
    
    // Not ready with just one participant
    let session1 = SessionId::new();
    room.add_participant(create_test_participant(session1.clone())).unwrap();
    room.set_state(ConferenceState::Active).unwrap();
    assert!(!room.is_media_ready());
    
    // Add second participant but no RTP ports
    let session2 = SessionId::new();
    room.add_participant(create_test_participant(session2.clone())).unwrap();
    assert!(!room.is_media_ready());
    
    // Assign RTP ports
    room.set_participant_rtp_port(&session1, 12345).unwrap();
    room.set_participant_rtp_port(&session2, 12346).unwrap();
    
    // Now should be ready
    assert!(room.is_media_ready());
}

#[tokio::test]
async fn test_should_terminate() {
    let conference_id = ConferenceId::new();
    let config = create_test_config();
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Empty room should terminate
    assert!(room.should_terminate());
    
    // Room with one active participant should terminate
    let session1 = SessionId::new();
    room.add_participant(create_test_participant(session1.clone())).unwrap();
    room.update_participant_status(&session1, ParticipantStatus::Active).unwrap();
    assert!(room.should_terminate());
    
    // Room with two active participants should not terminate
    let session2 = SessionId::new();
    room.add_participant(create_test_participant(session2.clone())).unwrap();
    room.update_participant_status(&session2, ParticipantStatus::Active).unwrap();
    assert!(!room.should_terminate());
}

#[tokio::test]
async fn test_capacity_utilization() {
    let conference_id = ConferenceId::new();
    let mut config = create_test_config();
    config.max_participants = 4;
    let mut room = ConferenceRoom::new(conference_id, config);
    
    // Empty room
    assert_eq!(room.capacity_utilization(), 0.0);
    assert!(!room.is_full());
    
    // Half full
    room.add_participant(create_test_participant(SessionId::new())).unwrap();
    room.add_participant(create_test_participant(SessionId::new())).unwrap();
    assert_eq!(room.capacity_utilization(), 0.5);
    assert!(!room.is_full());
    
    // Full
    room.add_participant(create_test_participant(SessionId::new())).unwrap();
    room.add_participant(create_test_participant(SessionId::new())).unwrap();
    assert_eq!(room.capacity_utilization(), 1.0);
    assert!(room.is_full());
} 