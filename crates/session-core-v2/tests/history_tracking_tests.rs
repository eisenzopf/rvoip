use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use rvoip_session_core_v2::{
    SessionStore, SessionState, SessionId, Role, CallState, EventType,
    HistoryConfig, SessionHistory, TransitionRecord,
};

#[tokio::test]
async fn test_history_tracking_basic() {
    let store = Arc::new(SessionStore::new());
    
    // Create session with history enabled
    let session_id = SessionId::new();
    let session = store.create_session(session_id.clone(), Role::UAC, true)
        .await
        .expect("Failed to create session with history");
    
    // Verify history is enabled
    assert!(session.history.is_some(), "History should be enabled");
    
    // Store the session
    store.update_session(session).await.unwrap();
    
    // Get session and check history is tracking
    let session = store.get_session(&session_id).await.unwrap();
    if let Some(ref history) = session.history {
        assert_eq!(history.total_transitions, 0, "Should have no transitions initially");
        assert!(history.session_age() > Duration::from_millis(0), "Session should have age");
    } else {
        panic!("History should be present");
    }
    
    // Update session state multiple times
    for next_state in [CallState::Initiating, CallState::Ringing, CallState::Active] {
        let mut session = store.get_session(&session_id).await.unwrap();
        session.call_state = next_state;
        store.update_session(session).await.unwrap();
    }
    
    // Verify session went through the state changes
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Active, "Should be in Active state");
}

#[tokio::test]
async fn test_history_ring_buffer() {
    let history_config = HistoryConfig {
        enabled: true,
        max_transitions: 3,
        track_actions: false,
        track_guards: false,
    };
    
    let mut history = SessionHistory::new(history_config);
    
    // Add more transitions than max
    for i in 0..5 {
        let record = TransitionRecord {
            timestamp: std::time::Instant::now(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0, // Will be set by record_transition
            from_state: CallState::Idle,
            event: EventType::MakeCall { target: format!("target{}", i) },
            to_state: Some(CallState::Initiating),
            guards_evaluated: vec![],
            actions_executed: vec![],
            events_published: vec![],
            duration_ms: 10,
            errors: vec![],
        };
        history.record_transition(record);
    }
    
    // Should only keep last 3 transitions
    let recent = history.get_recent(10);
    assert_eq!(recent.len(), 3, "Should only keep 3 most recent transitions");
    assert_eq!(history.total_transitions, 5, "Should count all 5 transitions");
    
    // Verify we have the most recent ones (sequences 2, 3, 4)
    assert_eq!(recent[0].sequence, 4, "Most recent should be sequence 4");
    assert_eq!(recent[1].sequence, 3, "Second should be sequence 3");
    assert_eq!(recent[2].sequence, 2, "Third should be sequence 2");
}

#[tokio::test]
async fn test_history_error_tracking() {
    let history_config = HistoryConfig::default();
    let mut history = SessionHistory::new(history_config);
    
    // Add transition with error
    let mut record = TransitionRecord {
        timestamp: std::time::Instant::now(),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        sequence: 0,
        from_state: CallState::Active,
        event: EventType::HangupCall,
        to_state: Some(CallState::Terminating),
        guards_evaluated: vec![],
        actions_executed: vec![],
        events_published: vec![],
        duration_ms: 10,
        errors: vec!["Test error".to_string()],
    };
    history.record_transition(record.clone());
    
    // Add successful transition
    record.errors.clear();
    record.from_state = CallState::Terminating;
    record.to_state = Some(CallState::Terminated);
    history.record_transition(record);
    
    // Check error tracking
    assert_eq!(history.total_errors, 1, "Should have 1 error");
    assert_eq!(history.total_transitions, 2, "Should have 2 total transitions");
    assert_eq!(history.error_rate(), 0.5, "Error rate should be 50%");
    
    let errors = history.get_errors();
    assert_eq!(errors.len(), 1, "Should have 1 transition with errors");
    assert!(errors[0].errors.contains(&"Test error".to_string()), "Error message should be preserved");
}

#[tokio::test]
async fn test_history_export() {
    let history_config = HistoryConfig::default();
    let mut history = SessionHistory::new(history_config);
    
    // Add some transitions with different data
    for i in 0..3 {
        let record = TransitionRecord {
            timestamp: std::time::Instant::now(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0,
            from_state: match i {
                0 => CallState::Idle,
                1 => CallState::Initiating,
                _ => CallState::Ringing,
            },
            event: match i {
                0 => EventType::MakeCall { target: format!("user{}", i) },
                1 => EventType::Dialog180Ringing,
                _ => EventType::Dialog200OK,
            },
            to_state: Some(match i {
                0 => CallState::Initiating,
                1 => CallState::Ringing,
                _ => CallState::Active,
            }),
            guards_evaluated: vec![],
            actions_executed: vec![],
            events_published: vec![],
            duration_ms: 10 + i as u64,
            errors: vec![],
        };
        history.record_transition(record);
        sleep(Duration::from_millis(10)).await;
    }
    
    // Test JSON export
    let json = history.export_json();
    assert!(json.contains("\"from_state\""), "JSON should contain from_state field");
    assert!(json.contains("\"Idle\""), "JSON should contain Idle state");
    assert!(json.contains("\"sequence\""), "JSON should contain sequence field");
    
    // Test CSV export  
    let csv = history.export_csv();
    assert!(csv.starts_with("sequence,timestamp_ms,from_state"), "CSV should have correct headers");
    assert!(csv.contains("Idle"), "CSV should contain Idle state");
    assert!(csv.contains("Initiating"), "CSV should contain Initiating state");
}

#[tokio::test]
async fn test_history_disabled() {
    let history_config = HistoryConfig {
        enabled: false,  // History tracking disabled
        max_transitions: 10,
        track_actions: true,
        track_guards: true,
    };
    
    let mut history = SessionHistory::new(history_config);
    
    // Try to add transition with history disabled
    let record = TransitionRecord {
        timestamp: std::time::Instant::now(),
        timestamp_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        sequence: 0,
        from_state: CallState::Idle,
        event: EventType::MakeCall { target: "test".to_string() },
        to_state: Some(CallState::Initiating),
        guards_evaluated: vec![],
        actions_executed: vec![],
        events_published: vec![],
        duration_ms: 10,
        errors: vec![],
    };
    history.record_transition(record);
    
    // Should not record anything when disabled
    assert_eq!(history.total_transitions, 0, "Should not record when disabled");
    assert_eq!(history.get_recent(10).len(), 0, "Should have no records when disabled");
}

#[tokio::test]
async fn test_history_state_filtering() {
    let history_config = HistoryConfig::default();
    let mut history = SessionHistory::new(history_config);
    
    // Add transitions through different states
    let states = vec![
        (CallState::Idle, CallState::Initiating),
        (CallState::Initiating, CallState::Ringing),
        (CallState::Ringing, CallState::Active),
        (CallState::Active, CallState::OnHold),
        (CallState::OnHold, CallState::Active),
    ];
    
    for (from, to) in states {
        let record = TransitionRecord {
            timestamp: std::time::Instant::now(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0,
            from_state: from,
            event: EventType::MakeCall { target: "test".to_string() },
            to_state: Some(to),
            guards_evaluated: vec![],
            actions_executed: vec![],
            events_published: vec![],
            duration_ms: 10,
            errors: vec![],
        };
        history.record_transition(record);
    }
    
    // Find all transitions involving Active state
    let active_transitions = history.get_by_state(CallState::Active);
    assert_eq!(active_transitions.len(), 3, "Should find 3 transitions involving Active state");
    
    // Find transitions involving Idle state
    let idle_transitions = history.get_by_state(CallState::Idle);
    assert_eq!(idle_transitions.len(), 1, "Should find 1 transition involving Idle state");
}