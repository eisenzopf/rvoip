/// Integration test for the state machine
use rvoip_session_core_v2::{
    state_table::{Role, CallState, EventType, SessionId},
    session_store::{SessionStore, SessionState},
};

#[tokio::test]
async fn test_session_state_transitions() {
    // Create a session store
    let store = SessionStore::new();
    
    // Create a UAC session
    let session_id = SessionId::new();
    let session = store.create_session(session_id.clone(), Role::UAC, false).await.unwrap();
    
    // Initial state should be Idle
    assert_eq!(session.call_state, CallState::Idle);
    assert!(!session.all_conditions_met());
    
    // Simulate state transitions
    let mut session = store.get_session(&session_id).await.unwrap();
    
    // Transition to Initiating (simulating MakeCall)
    session.transition_to(CallState::Initiating);
    store.update_session(session.clone()).await.unwrap();
    assert_eq!(session.call_state, CallState::Initiating);
    
    // Transition to Ringing (simulating 180 Ringing)
    session.transition_to(CallState::Ringing);
    store.update_session(session.clone()).await.unwrap();
    assert_eq!(session.call_state, CallState::Ringing);
    
    // Transition to Active (simulating 200 OK)
    session.transition_to(CallState::Active);
    session.dialog_established = true;
    store.update_session(session.clone()).await.unwrap();
    assert_eq!(session.call_state, CallState::Active);
    
    // Set media ready and SDP negotiated
    session.media_session_ready = true;
    session.sdp_negotiated = true;
    store.update_session(session.clone()).await.unwrap();
    
    // Now all conditions should be met
    assert!(session.all_conditions_met());
    
    // Transition to Terminating (simulating BYE)
    session.transition_to(CallState::Terminating);
    store.update_session(session.clone()).await.unwrap();
    assert_eq!(session.call_state, CallState::Terminating);
    
    // Transition to Terminated
    session.transition_to(CallState::Terminated);
    store.update_session(session.clone()).await.unwrap();
    assert_eq!(session.call_state, CallState::Terminated);
}

#[tokio::test]
async fn test_condition_tracking() {
    let store = SessionStore::new();
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAS, false).await.unwrap();
    
    // Initially no conditions met
    assert!(!session.dialog_established);
    assert!(!session.media_session_ready);
    assert!(!session.sdp_negotiated);
    assert!(!session.all_conditions_met());
    
    // Set conditions one by one
    session.dialog_established = true;
    assert!(!session.all_conditions_met());
    
    session.media_session_ready = true;
    assert!(!session.all_conditions_met());
    
    session.sdp_negotiated = true;
    assert!(session.all_conditions_met());
    
    // Update in store
    store.update_session(session).await.unwrap();
    
    // Verify persistence
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(retrieved.all_conditions_met());
}

#[tokio::test]
async fn test_session_store_indexes() {
    let store = SessionStore::new();
    
    // Create multiple sessions
    let session1_id = SessionId::new();
    let session2_id = SessionId::new();
    
    let mut session1 = store.create_session(session1_id.clone(), Role::UAC, false).await.unwrap();
    let mut session2 = store.create_session(session2_id.clone(), Role::UAS, false).await.unwrap();
    
    // Set dialog IDs
    session1.dialog_id = Some("dialog-1".to_string());
    session2.dialog_id = Some("dialog-2".to_string());
    
    store.update_session(session1).await.unwrap();
    store.update_session(session2).await.unwrap();
    
    // Find by dialog ID
    let found = store.find_by_dialog(&"dialog-1".to_string()).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_id, session1_id);
    
    let found = store.find_by_dialog(&"dialog-2".to_string()).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().session_id, session2_id);
    
    // Get all sessions
    let all_sessions = store.get_all_sessions().await;
    assert_eq!(all_sessions.len(), 2);
    
    // Get statistics
    let stats = store.get_stats().await;
    assert_eq!(stats.total, 2);
    assert_eq!(stats.idle, 2); // Both sessions are in Idle state
}

#[tokio::test]
async fn test_session_with_history() {
    let store = SessionStore::new();
    let session_id = SessionId::new();
    
    // Create session with history tracking
    let mut session = SessionState::with_history(session_id.clone(), Role::UAC);
    
    // Make some transitions
    session.transition_to(CallState::Initiating);
    session.transition_to(CallState::Ringing);
    session.transition_to(CallState::Active);
    
    // Check history
    if let Some(history) = &session.history {
        let records = history.get_history();
        assert_eq!(records.len(), 3);
        
        // Verify transitions
        assert_eq!(records[0].from_state, CallState::Idle);
        assert_eq!(records[0].to_state, CallState::Initiating);
        
        assert_eq!(records[1].from_state, CallState::Initiating);
        assert_eq!(records[1].to_state, CallState::Ringing);
        
        assert_eq!(records[2].from_state, CallState::Ringing);
        assert_eq!(records[2].to_state, CallState::Active);
    } else {
        panic!("History should be present");
    }
}