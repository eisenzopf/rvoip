//! Conference Integration Tests
//!
//! End-to-end integration tests for the complete conference system.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        manager::ConferenceManager,
        types::*,
        api::ConferenceApi,
        events::{ConferenceEvent, ConferenceEventHandler},
    },
    SessionError,
};

/// Event collector for testing
#[derive(Debug)]
struct EventCollector {
    events: Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>,
}

impl EventCollector {
    fn new() -> (Self, Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>) {
        let events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        (Self { events: events.clone() }, events)
    }
}

#[async_trait::async_trait]
impl ConferenceEventHandler for EventCollector {
    async fn handle_event(&self, event: ConferenceEvent) {
        let mut events = self.events.lock().await;
        events.push(event);
    }
}

async fn create_test_system() -> (
    Arc<ConferenceManager>,
    Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>,
) {
    let conference_manager = Arc::new(ConferenceManager::new());

    let (event_handler, events) = EventCollector::new();
    conference_manager.add_event_handler("test", Arc::new(event_handler)).await;

    (conference_manager, events)
}

#[tokio::test]
async fn test_end_to_end_conference_flow() {
    let (manager, events) = create_test_system().await;
    
    // Create conference
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Add participants
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    let session3 = SessionId::new();
    
    // Join participants
    let participant1 = manager.join_conference(&conference_id, &session1).await.unwrap();
    let participant2 = manager.join_conference(&conference_id, &session2).await.unwrap();
    let participant3 = manager.join_conference(&conference_id, &session3).await.unwrap();
    
    // Verify all participants joined
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 3);
    
    // Update participant statuses
    manager.update_participant_status(&conference_id, &session1, ParticipantStatus::Active).await.unwrap();
    manager.update_participant_status(&conference_id, &session2, ParticipantStatus::Active).await.unwrap();
    manager.update_participant_status(&conference_id, &session3, ParticipantStatus::Muted).await.unwrap();
    
    // Check conference stats
    let stats = manager.get_conference_stats(&conference_id).await.unwrap();
    assert_eq!(stats.total_participants, 3);
    assert_eq!(stats.active_participants, 2); // Only session1 and session2 are active
    
    // Generate SDP for each participant
    let sdp1 = manager.generate_conference_sdp(&conference_id, &session1).await.unwrap();
    let sdp2 = manager.generate_conference_sdp(&conference_id, &session2).await.unwrap();
    let sdp3 = manager.generate_conference_sdp(&conference_id, &session3).await.unwrap();
    
    // Verify SDPs are unique but contain conference info
    assert_ne!(sdp1, sdp2);
    assert_ne!(sdp2, sdp3);
    assert!(sdp1.contains(&conference_id.to_string()));
    assert!(sdp2.contains(&conference_id.to_string()));
    assert!(sdp3.contains(&conference_id.to_string()));
    
    // Remove one participant
    manager.leave_conference(&conference_id, &session2).await.unwrap();
    
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 2);
    
    // Terminate conference
    manager.terminate_conference(&conference_id).await.unwrap();
    
    // Verify conference is gone
    assert!(!manager.conference_exists(&conference_id).await);
    
    // Verify events were generated
    tokio::time::sleep(Duration::from_millis(10)).await;
    let events = events.lock().await;
    assert!(events.len() >= 6); // Creation, 3 joins, 1 leave, 1 termination + status changes
}

#[tokio::test]
async fn test_concurrent_multi_conference() {
    let (manager, _events) = create_test_system().await;
    
    // Create multiple conferences concurrently
    let config = ConferenceConfig::default();
    let mut conference_tasks = Vec::new();
    
    for i in 0..5 {
        let manager_clone = manager.clone();
        let config_clone = config.clone();
        
        let task = tokio::spawn(async move {
            let conference_id = manager_clone.create_conference(config_clone).await.unwrap();
            
            // Add participants to each conference
            let mut participants = Vec::new();
            for j in 0..3 {
                let session_id = SessionId::new();
                manager_clone.join_conference(&conference_id, &session_id).await.unwrap();
                participants.push(session_id);
            }
            
            (conference_id, participants)
        });
        
        conference_tasks.push(task);
    }
    
    // Wait for all conferences to be created
    let mut all_conferences = Vec::new();
    for task in conference_tasks {
        let (conference_id, participants) = task.await.unwrap();
        all_conferences.push((conference_id, participants));
    }
    
    // Verify all conferences exist and have correct participant counts
    assert_eq!(all_conferences.len(), 5);
    assert_eq!(manager.conference_count(), 5);
    
    for (conference_id, expected_participants) in &all_conferences {
        let participants = manager.list_participants(conference_id).await.unwrap();
        assert_eq!(participants.len(), expected_participants.len());
    }
    
    // Verify conference isolation - participants in one conference shouldn't affect others
    let (first_conf, first_participants) = &all_conferences[0];
    manager.leave_conference(first_conf, &first_participants[0]).await.unwrap();
    
    // Other conferences should be unaffected
    for (conf_id, expected_participants) in &all_conferences[1..] {
        let participants = manager.list_participants(conf_id).await.unwrap();
        assert_eq!(participants.len(), expected_participants.len());
    }
}

#[tokio::test]
async fn test_error_handling_and_recovery() {
    let (manager, _events) = create_test_system().await;
    
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    // Join participant
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Test operations on non-existent entities
    let fake_conference = ConferenceId::new();
    let fake_session = SessionId::new();
    
    let result = manager.join_conference(&fake_conference, &session_id).await;
    assert!(result.is_err());
    
    let result = manager.leave_conference(&conference_id, &fake_session).await;
    assert!(result.is_err());
    
    let result = manager.update_participant_status(&fake_conference, &session_id, ParticipantStatus::Active).await;
    assert!(result.is_err());
    
    // Test capacity limits
    let mut limited_config = ConferenceConfig::default();
    limited_config.max_participants = 1;
    
    let limited_conf = manager.create_conference(limited_config).await.unwrap();
    manager.join_conference(&limited_conf, &SessionId::new()).await.unwrap();
    
    let result = manager.join_conference(&limited_conf, &SessionId::new()).await;
    assert!(result.is_err());
} 