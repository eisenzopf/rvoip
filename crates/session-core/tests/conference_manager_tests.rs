use rvoip_session_core::api::control::SessionControl;
// Conference Manager Tests
//
// Unit tests for the ConferenceManager implementation.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::Mutex;

use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        manager::ConferenceManager,
        api::ConferenceApi,
        types::*,
        events::{ConferenceEvent, ConferenceEventHandler},
    },
    SessionError,
};

/// Test event handler that counts events
#[derive(Debug)]
struct CountingEventHandler {
    count: AtomicUsize,
    events: Arc<Mutex<Vec<ConferenceEvent>>>,
}

impl CountingEventHandler {
    fn new() -> (Arc<Self>, Arc<Mutex<Vec<ConferenceEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(Self {
            count: AtomicUsize::new(0),
            events: events.clone(),
        });
        (handler, events)
    }

    fn get_count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ConferenceEventHandler for CountingEventHandler {
    async fn handle_event(&self, event: ConferenceEvent) {
        self.count.fetch_add(1, Ordering::SeqCst);
        let mut events = self.events.lock().await;
        events.push(event);
    }
}

#[tokio::test]
async fn test_event_handler_management() {
    let manager = ConferenceManager::new();
    
    // Initially no handlers
    assert_eq!(manager.event_handler_count().await, 0);
    
    // Add first handler
    let (handler1, _) = CountingEventHandler::new();
    manager.add_event_handler("handler1", handler1.clone()).await;
    assert_eq!(manager.event_handler_count().await, 1);
    
    // Add second handler
    let (handler2, _) = CountingEventHandler::new();
    manager.add_event_handler("handler2", handler2.clone()).await;
    assert_eq!(manager.event_handler_count().await, 2);
    
    // Create a conference to trigger events
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Give handlers time to process
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    // Both handlers should have received the event
    assert_eq!(handler1.get_count(), 1);
    assert_eq!(handler2.get_count(), 1);
    
    // Remove one handler
    assert!(manager.remove_event_handler("handler1").await);
    assert_eq!(manager.event_handler_count().await, 1);
    
    // Remove non-existent handler
    assert!(!manager.remove_event_handler("nonexistent").await);
    
    // Terminate conference and check only handler2 gets the event
    manager.terminate_conference(&conference_id).await.unwrap();
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    assert_eq!(handler1.get_count(), 1); // Still 1, not incremented
    assert_eq!(handler2.get_count(), 2); // Incremented to 2
}

#[tokio::test]
async fn test_conference_counting() {
    let manager = ConferenceManager::new();
    
    // Initially no conferences
    assert_eq!(manager.conference_count(), 0);
    
    // Create conferences
    let config = ConferenceConfig::default();
    let conf1 = manager.create_conference(config.clone()).await.unwrap();
    assert_eq!(manager.conference_count(), 1);
    
    let conf2 = manager.create_conference(config.clone()).await.unwrap();
    assert_eq!(manager.conference_count(), 2);
    
    let conf3 = manager.create_conference(config).await.unwrap();
    assert_eq!(manager.conference_count(), 3);
    
    // Terminate conferences
    manager.terminate_conference(&conf1).await.unwrap();
    assert_eq!(manager.conference_count(), 2);
    
    manager.terminate_conference(&conf2).await.unwrap();
    assert_eq!(manager.conference_count(), 1);
    
    manager.terminate_conference(&conf3).await.unwrap();
    assert_eq!(manager.conference_count(), 0);
}

#[tokio::test]
async fn test_concurrent_conference_operations() {
    let manager = Arc::new(ConferenceManager::new());
    let config = ConferenceConfig::default();
    
    // Create multiple conferences concurrently
    let mut create_tasks = Vec::new();
    for _ in 0..10 {
        let manager_clone = manager.clone();
        let config_clone = config.clone();
        
        let task = tokio::spawn(async move {
            manager_clone.create_conference(config_clone).await.unwrap()
        });
        create_tasks.push(task);
    }
    
    let mut conference_ids = Vec::new();
    for task in create_tasks {
        conference_ids.push(task.await.unwrap());
    }
    
    assert_eq!(manager.conference_count(), 10);
    
    // Join participants concurrently
    let mut join_tasks = Vec::new();
    for (i, conference_id) in conference_ids.iter().enumerate() {
        for j in 0..5 {
            let manager_clone = manager.clone();
            let conference_id = conference_id.clone();
            let session_id = SessionId::new();
            
            let task = tokio::spawn(async move {
                manager_clone.join_conference(&conference_id, &session_id).await.unwrap()
            });
            join_tasks.push(task);
        }
    }
    
    for task in join_tasks {
        task.await.unwrap();
    }
    
    // Verify all participants joined
    for conference_id in &conference_ids {
        let participants = manager.list_participants(conference_id).await.unwrap();
        assert_eq!(participants.len(), 5);
    }
    
    // Terminate conferences concurrently
    let mut terminate_tasks = Vec::new();
    for conference_id in conference_ids {
        let manager_clone = manager.clone();
        
        let task = tokio::spawn(async move {
            manager_clone.terminate_conference(&conference_id).await.unwrap()
        });
        terminate_tasks.push(task);
    }
    
    for task in terminate_tasks {
        task.await.unwrap();
    }
    
    assert_eq!(manager.conference_count(), 0);
}

#[tokio::test]
async fn test_event_ordering() {
    let manager = ConferenceManager::new();
    let (handler, events) = CountingEventHandler::new();
    
    manager.add_event_handler("test", handler).await;
    
    // Create conference
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Join participants
    let session1 = SessionId::new();
    let session2 = SessionId::new();
    
    manager.join_conference(&conference_id, &session1).await.unwrap();
    manager.join_conference(&conference_id, &session2).await.unwrap();
    
    // Update status
    manager.update_participant_status(&conference_id, &session1, ParticipantStatus::Active).await.unwrap();
    
    // Leave conference
    manager.leave_conference(&conference_id, &session1).await.unwrap();
    
    // Terminate
    manager.terminate_conference(&conference_id).await.unwrap();
    
    // Give time for all events to be processed
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Check event order
    let events = events.lock().await;
    assert!(events.len() >= 6);
    
    // Verify event types in order
    assert!(matches!(&events[0], ConferenceEvent::ConferenceCreated { .. }));
    assert!(matches!(&events[1], ConferenceEvent::ParticipantJoined { .. }));
    assert!(matches!(&events[2], ConferenceEvent::ParticipantJoined { .. }));
    assert!(matches!(&events[3], ConferenceEvent::ParticipantStatusChanged { .. }));
    assert!(matches!(&events[4], ConferenceEvent::ParticipantLeft { .. }));
    assert!(matches!(&events[5], ConferenceEvent::ConferenceTerminated { .. }));
}

#[tokio::test]
async fn test_update_conference_config() {
    let manager = ConferenceManager::new();
    
    // Create conference with initial config
    let mut config = ConferenceConfig::default();
    config.max_participants = 10;
    config.audio_mixing_enabled = true;
    
    let conference_id = manager.create_conference(config.clone()).await.unwrap();
    
    // Verify initial config
    let stored_config = manager.get_conference_config(&conference_id).await.unwrap();
    assert_eq!(stored_config.max_participants, 10);
    assert!(stored_config.audio_mixing_enabled);
    
    // Update config
    let mut new_config = config.clone();
    new_config.max_participants = 20;
    new_config.audio_mixing_enabled = false;
    
    manager.update_conference_config(&conference_id, new_config.clone()).await.unwrap();
    
    // Verify updated config
    let updated_config = manager.get_conference_config(&conference_id).await.unwrap();
    assert_eq!(updated_config.max_participants, 20);
    assert!(!updated_config.audio_mixing_enabled);
}

#[tokio::test]
async fn test_error_handling() {
    let manager = ConferenceManager::new();
    let fake_conference_id = ConferenceId::new();
    let fake_session_id = SessionId::new();
    
    // Operations on non-existent conference
    let result = manager.get_conference_config(&fake_conference_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
    
    let result = manager.join_conference(&fake_conference_id, &fake_session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
    
    let result = manager.leave_conference(&fake_conference_id, &fake_session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
    
    let result = manager.terminate_conference(&fake_conference_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
    
    // Create conference to test participant errors
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Try to remove non-existent participant
    let result = manager.leave_conference(&conference_id, &fake_session_id).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
    
    // Try to update status of non-existent participant
    let result = manager.update_participant_status(&conference_id, &fake_session_id, ParticipantStatus::Active).await;
    assert!(matches!(result, Err(SessionError::SessionNotFound(_))));
}
