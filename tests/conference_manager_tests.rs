//! Conference Manager Tests
//!
//! Tests for ConferenceManager high-level functionality and event handling.

use std::sync::Arc;
use std::time::Duration;
use tokio::test;
use rvoip_session_core::{
    api::types::SessionId,
    conference::{
        manager::ConferenceManager,
        types::*,
        events::{ConferenceEvent, ConferenceEventHandler},
        api::ConferenceApi,
    },
    errors::Result,
};

/// Test event handler that collects events
#[derive(Debug)]
struct TestEventHandler {
    events: Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>,
}

impl TestEventHandler {
    fn new() -> (Self, Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>) {
        let events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        (
            Self {
                events: events.clone(),
            },
            events,
        )
    }
}

#[async_trait::async_trait]
impl ConferenceEventHandler for TestEventHandler {
    async fn handle_event(&self, event: ConferenceEvent) {
        let mut events = self.events.lock().await;
        events.push(event);
    }
}

async fn create_manager_with_handler() -> (ConferenceManager, Arc<tokio::sync::Mutex<Vec<ConferenceEvent>>>) {
    let manager = ConferenceManager::new();
    let (handler, events) = TestEventHandler::new();
    manager.add_event_handler("test", Arc::new(handler)).await;
    (manager, events)
}

#[tokio::test]
async fn test_manager_creation() {
    let manager = ConferenceManager::new();
    
    // Initially no conferences
    assert_eq!(manager.conference_count(), 0);
    assert_eq!(manager.event_handler_count().await, 0);
}

#[tokio::test]
async fn test_event_handler_management() {
    let manager = ConferenceManager::new();
    let (handler1, _) = TestEventHandler::new();
    let (handler2, _) = TestEventHandler::new();
    
    // Add handlers
    manager.add_event_handler("handler1", Arc::new(handler1)).await;
    manager.add_event_handler("handler2", Arc::new(handler2)).await;
    
    assert_eq!(manager.event_handler_count().await, 2);
    
    // Remove handler
    let removed = manager.remove_event_handler("handler1").await;
    assert!(removed);
    assert_eq!(manager.event_handler_count().await, 1);
    
    // Try to remove non-existent handler
    let removed = manager.remove_event_handler("nonexistent").await;
    assert!(!removed);
    assert_eq!(manager.event_handler_count().await, 1);
}

#[tokio::test]
async fn test_conference_creation_events() {
    let (manager, events) = create_manager_with_handler().await;
    let config = ConferenceConfig::default();
    
    // Create conference
    let conference_id = manager.create_conference(config.clone()).await.unwrap();
    
    // Verify event was published
    tokio::time::sleep(Duration::from_millis(10)).await; // Brief delay for event processing
    let events = events.lock().await;
    assert_eq!(events.len(), 1);
    
    match &events[0] {
        ConferenceEvent::ConferenceCreated { conference_id: id, config: cfg, .. } => {
            assert_eq!(*id, conference_id);
            assert_eq!(cfg.max_participants, config.max_participants);
        }
        _ => panic!("Expected ConferenceCreated event"),
    }
}

#[tokio::test]
async fn test_participant_join_leave_events() {
    let (manager, events) = create_manager_with_handler().await;
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    // Join conference
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    
    // Leave conference
    manager.leave_conference(&conference_id, &session_id).await.unwrap();
    
    // Verify events
    tokio::time::sleep(Duration::from_millis(10)).await;
    let events = events.lock().await;
    assert_eq!(events.len(), 3); // ConferenceCreated, ParticipantJoined, ParticipantLeft
    
    match &events[1] {
        ConferenceEvent::ParticipantJoined { conference_id: id, session_id: sid, .. } => {
            assert_eq!(*id, conference_id);
            assert_eq!(*sid, session_id);
        }
        _ => panic!("Expected ParticipantJoined event"),
    }
    
    match &events[2] {
        ConferenceEvent::ParticipantLeft { conference_id: id, session_id: sid, .. } => {
            assert_eq!(*id, conference_id);
            assert_eq!(*sid, session_id);
        }
        _ => panic!("Expected ParticipantLeft event"),
    }
}

#[tokio::test]
async fn test_participant_status_change_events() {
    let (manager, events) = create_manager_with_handler().await;
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    let session_id = SessionId::new();
    
    // Join and update status
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    manager.update_participant_status(&conference_id, &session_id, ParticipantStatus::Active).await.unwrap();
    
    // Verify status change event
    tokio::time::sleep(Duration::from_millis(10)).await;
    let events = events.lock().await;
    
    // Find the status change event
    let status_event = events.iter().find(|e| matches!(e, ConferenceEvent::ParticipantStatusChanged { .. }));
    assert!(status_event.is_some());
    
    match status_event.unwrap() {
        ConferenceEvent::ParticipantStatusChanged { 
            conference_id: id, 
            session_id: sid, 
            old_status,
            new_status,
            .. 
        } => {
            assert_eq!(*id, conference_id);
            assert_eq!(*sid, session_id);
            assert_eq!(*old_status, ParticipantStatus::Joining);
            assert_eq!(*new_status, ParticipantStatus::Active);
        }
        _ => panic!("Expected ParticipantStatusChanged event"),
    }
}

#[tokio::test]
async fn test_conference_termination_events() {
    let (manager, events) = create_manager_with_handler().await;
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Add participant and terminate
    let session_id = SessionId::new();
    manager.join_conference(&conference_id, &session_id).await.unwrap();
    manager.terminate_conference(&conference_id).await.unwrap();
    
    // Verify termination event
    tokio::time::sleep(Duration::from_millis(10)).await;
    let events = events.lock().await;
    
    let termination_event = events.iter().find(|e| matches!(e, ConferenceEvent::ConferenceTerminated { .. }));
    assert!(termination_event.is_some());
    
    match termination_event.unwrap() {
        ConferenceEvent::ConferenceTerminated { conference_id: id, .. } => {
            assert_eq!(*id, conference_id);
        }
        _ => panic!("Expected ConferenceTerminated event"),
    }
}

#[tokio::test]
async fn test_conference_count_tracking() {
    let manager = ConferenceManager::new();
    let config = ConferenceConfig::default();
    
    assert_eq!(manager.conference_count(), 0);
    
    // Create conferences
    let conf1 = manager.create_conference(config.clone()).await.unwrap();
    assert_eq!(manager.conference_count(), 1);
    
    let conf2 = manager.create_conference(config.clone()).await.unwrap();
    assert_eq!(manager.conference_count(), 2);
    
    // Terminate one
    manager.terminate_conference(&conf1).await.unwrap();
    assert_eq!(manager.conference_count(), 1);
    
    // Terminate the other
    manager.terminate_conference(&conf2).await.unwrap();
    assert_eq!(manager.conference_count(), 0);
}

#[tokio::test]
async fn test_concurrent_operations() {
    let manager = Arc::new(ConferenceManager::new());
    let config = ConferenceConfig::default();
    
    // Create conference
    let conference_id = manager.create_conference(config).await.unwrap();
    
    // Spawn multiple tasks to join the conference concurrently
    let mut handles = Vec::new();
    for i in 0..10 {
        let manager_clone = manager.clone();
        let conference_id_clone = conference_id.clone();
        
        let handle = tokio::spawn(async move {
            let session_id = SessionId::new();
            manager_clone.join_conference(&conference_id_clone, &session_id).await
        });
        handles.push(handle);
    }
    
    // Wait for all joins to complete
    let mut successful_joins = 0;
    for handle in handles {
        if handle.await.unwrap().is_ok() {
            successful_joins += 1;
        }
    }
    
    // All should succeed (default config allows many participants)
    assert_eq!(successful_joins, 10);
    
    // Verify participant count
    let participants = manager.list_participants(&conference_id).await.unwrap();
    assert_eq!(participants.len(), 10);
}

#[tokio::test]
async fn test_multiple_event_handlers() {
    let manager = ConferenceManager::new();
    let (handler1, events1) = TestEventHandler::new();
    let (handler2, events2) = TestEventHandler::new();
    
    manager.add_event_handler("handler1", Arc::new(handler1)).await;
    manager.add_event_handler("handler2", Arc::new(handler2)).await;
    
    // Create conference - should notify both handlers
    let config = ConferenceConfig::default();
    let conference_id = manager.create_conference(config).await.unwrap();
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    // Both handlers should have received the event
    let events1 = events1.lock().await;
    let events2 = events2.lock().await;
    assert_eq!(events1.len(), 1);
    assert_eq!(events2.len(), 1);
}

#[tokio::test]
async fn test_configuration_updates() {
    let manager = ConferenceManager::new();
    let mut config = ConferenceConfig::default();
    config.max_participants = 5;
    
    let conference_id = manager.create_conference(config.clone()).await.unwrap();
    
    // Update configuration
    config.max_participants = 10;
    config.audio_mixing_enabled = false;
    manager.update_conference_config(&conference_id, config.clone()).await.unwrap();
    
    // Verify configuration was updated
    let stored_config = manager.get_conference_config(&conference_id).await.unwrap();
    assert_eq!(stored_config.max_participants, 10);
    assert_eq!(stored_config.audio_mixing_enabled, false);
}

#[tokio::test]
async fn test_error_handling() {
    let manager = ConferenceManager::new();
    let nonexistent_conference = ConferenceId::new();
    let session_id = SessionId::new();
    
    // Test operations on non-existent conference
    let result = manager.join_conference(&nonexistent_conference, &session_id).await;
    assert!(result.is_err());
    
    let result = manager.leave_conference(&nonexistent_conference, &session_id).await;
    assert!(result.is_err());
    
    let result = manager.list_participants(&nonexistent_conference).await;
    assert!(result.is_err());
    
    let result = manager.get_conference_stats(&nonexistent_conference).await;
    assert!(result.is_err());
    
    let result = manager.terminate_conference(&nonexistent_conference).await;
    assert!(result.is_err());
} 