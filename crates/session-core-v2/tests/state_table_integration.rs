use std::sync::Arc;
use rvoip_session_core_v2::{
    SessionStore, SessionState, SessionId, Role, CallState, EventType,
    state_table::{self, StateKey, Guard, Action},
    HistoryConfig, TransitionRecord,
};

/// Test that the state table has necessary transitions for UAC
#[test]
fn test_uac_transitions_exist() {
    let table = &*state_table::MASTER_TABLE;
    
    // Verify UAC can make a call from Idle
    let has_makecall = table.get_transition(&StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall { target: "test".to_string() },
    });
    assert!(has_makecall.is_some(), "UAC should be able to make call from Idle");
    
    // Verify UAC transitions through call setup
    let has_ringing = table.get_transition(&StateKey {
        role: Role::UAC,
        state: CallState::Initiating,
        event: EventType::Dialog180Ringing,
    });
    assert!(has_ringing.is_some(), "UAC should transition to Ringing on 180");
    
    // Verify UAC can go active
    let has_active = table.get_transition(&StateKey {
        role: Role::UAC,
        state: CallState::Ringing,
        event: EventType::Dialog200OK,
    });
    assert!(has_active.is_some(), "UAC should transition to Active on 200 OK");
}

/// Test that the state table has necessary transitions for UAS
#[test]
fn test_uas_transitions_exist() {
    let table = &*state_table::MASTER_TABLE;
    
    // Verify UAS can receive a call
    let has_incoming = table.get_transition(&StateKey {
        role: Role::UAS,
        state: CallState::Idle,
        event: EventType::IncomingCall { from: "test".to_string(), sdp: None },
    });
    assert!(has_incoming.is_some(), "UAS should be able to receive incoming call");
    
    // Verify UAS can accept call
    let has_accept = table.get_transition(&StateKey {
        role: Role::UAS,
        state: CallState::Ringing,
        event: EventType::AcceptCall,
    });
    assert!(has_accept.is_some(), "UAS should be able to accept call when ringing");
    
    // Verify UAS can reject call
    let has_reject = table.get_transition(&StateKey {
        role: Role::UAS,
        state: CallState::Ringing,
        event: EventType::RejectCall { reason: "Busy".to_string() },
    });
    assert!(has_reject.is_some(), "UAS should be able to reject call when ringing");
}

/// Test session state transitions without the full state machine
#[tokio::test]
async fn test_session_state_transitions() {
    let store = Arc::new(SessionStore::new());
    
    // Create a session
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAC, true)
        .await
        .expect("Failed to create session");
    
    // Verify initial state
    assert_eq!(session.call_state, CallState::Idle, "Should start in Idle");
    assert!(!session.dialog_established, "Dialog should not be established initially");
    assert!(!session.media_session_ready, "Media should not be ready initially");
    assert!(!session.sdp_negotiated, "SDP should not be negotiated initially");
    
    // Manually transition through states
    session.call_state = CallState::Initiating;
    session.entered_state_at = std::time::Instant::now();
    store.update_session(session.clone()).await.unwrap();
    
    // Verify we can retrieve updated state
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Initiating, "Should be in Initiating");
    
    // Set readiness conditions
    let mut session = store.get_session(&session_id).await.unwrap();
    session.dialog_established = true;
    session.media_session_ready = true;
    session.sdp_negotiated = true;
    session.call_state = CallState::Active;
    store.update_session(session.clone()).await.unwrap();
    
    // Verify all conditions are set
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Active, "Should be Active");
    assert!(session.dialog_established, "Dialog should be established");
    assert!(session.media_session_ready, "Media should be ready");
    assert!(session.sdp_negotiated, "SDP should be negotiated");
}

/// Test that history is properly maintained in SessionState
#[tokio::test]
async fn test_session_history() {
    let store = Arc::new(SessionStore::new());
    
    // Create session with history
    let session_id = SessionId::new();
    let session = store.create_session(session_id.clone(), Role::UAS, true)
        .await
        .expect("Failed to create session");
    
    assert!(session.history.is_some(), "History should be enabled");
    
    // Update session and manually add history
    let mut session = store.get_session(&session_id).await.unwrap();
    
    // Record a transition manually
    if let Some(ref mut history) = session.history {
        let record = TransitionRecord {
            timestamp: std::time::Instant::now(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            sequence: 0,
            from_state: CallState::Idle,
            event: EventType::IncomingCall { 
                from: "sip:alice@example.com".to_string(),
                sdp: None,
            },
            to_state: Some(CallState::Ringing),
            guards_evaluated: vec![],
            actions_executed: vec![],
            events_published: vec![],
            duration_ms: 5,
            errors: vec![],
        };
        history.record_transition(record);
    }
    
    session.call_state = CallState::Ringing;
    store.update_session(session).await.unwrap();
    
    // Verify history was preserved
    let session = store.get_session(&session_id).await.unwrap();
    if let Some(ref history) = session.history {
        assert_eq!(history.total_transitions, 1, "Should have one transition");
        
        let recent = history.get_recent(10);
        assert_eq!(recent.len(), 1, "Should have one recent transition");
        assert_eq!(recent[0].from_state, CallState::Idle, "Should transition from Idle");
        assert_eq!(recent[0].to_state, Some(CallState::Ringing), "Should transition to Ringing");
    }
}

/// Test hold and resume state transitions
#[tokio::test]
async fn test_hold_resume_states() {
    let store = Arc::new(SessionStore::new());
    
    // Create active session
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create session");
    
    // Set to active state
    session.call_state = CallState::Active;
    session.dialog_established = true;
    session.media_session_ready = true;
    store.update_session(session).await.unwrap();
    
    // Transition to OnHold
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::OnHold;
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::OnHold, "Should be on hold");
    
    // Transition to Resuming
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::Resuming;
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Resuming, "Should be resuming");
    
    // Back to Active
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::Active;
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Active, "Should be active again");
}

/// Test session termination flow
#[tokio::test]
async fn test_termination_flow() {
    let store = Arc::new(SessionStore::new());
    
    // Create active session
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAS, true)
        .await
        .expect("Failed to create session");
    session.call_state = CallState::Active;
    store.update_session(session).await.unwrap();
    
    // Move to Terminating
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::Terminating;
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Terminating, "Should be terminating");
    
    // Move to Terminated
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::Terminated;
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert_eq!(session.call_state, CallState::Terminated, "Should be terminated");
}

/// Test failure states
#[tokio::test]
async fn test_failure_states() {
    use rvoip_session_core_v2::state_table::types::FailureReason;
    let store = Arc::new(SessionStore::new());
    
    // Test timeout failure
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create session");
    session.call_state = CallState::Failed(FailureReason::Timeout);
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id).await.unwrap();
    assert!(matches!(session.call_state, CallState::Failed(FailureReason::Timeout)), 
            "Should be failed with timeout");
    
    // Test rejection failure
    let session_id_2 = SessionId::new();
    let mut session = store.create_session(session_id_2.clone(), Role::UAS, false)
        .await
        .expect("Failed to create session");
    session.call_state = CallState::Failed(FailureReason::Rejected);
    store.update_session(session).await.unwrap();
    
    let session = store.get_session(&session_id_2).await.unwrap();
    assert!(matches!(session.call_state, CallState::Failed(FailureReason::Rejected)), 
            "Should be failed with rejection");
}

/// Test that guards and actions are properly defined in transitions
#[test]
fn test_transition_guards_and_actions() {
    let table = &*state_table::MASTER_TABLE;
    
    // Get a known transition
    let transition = table.get_transition(&StateKey {
        role: Role::UAC,
        state: CallState::Idle,
        event: EventType::MakeCall { target: "test".to_string() },
    }).expect("Should have MakeCall transition");
    
    // Should transition to Initiating
    assert_eq!(transition.next_state, Some(CallState::Initiating), 
               "MakeCall should transition to Initiating");
    
    // Should have appropriate actions
    let has_invite_action = transition.actions.iter().any(|a| 
        matches!(a, Action::SendINVITE)
    );
    assert!(has_invite_action, "MakeCall should trigger SendINVITE action");
}

/// Test session inspection capabilities
#[tokio::test]
async fn test_session_inspection() {
    let store = Arc::new(SessionStore::new());
    
    // Create multiple sessions
    let mut session_ids = Vec::new();
    for i in 0..3 {
        let id = SessionId::new();
        let mut session = store.create_session(id.clone(), Role::UAC, false)
            .await
            .expect("Failed to create session");
        
        // Set different states
        session.call_state = match i {
            0 => CallState::Active,
            1 => CallState::OnHold,
            _ => CallState::Idle,
        };
        
        store.update_session(session).await.unwrap();
        session_ids.push(id);
    }
    
    // Find active sessions
    let active_sessions = store.find_sessions(|s| 
        matches!(s.call_state, CallState::Active)
    ).await;
    assert_eq!(active_sessions.len(), 1, "Should find 1 active session");
    
    // Find on-hold sessions
    let hold_sessions = store.find_sessions(|s|
        matches!(s.call_state, CallState::OnHold)
    ).await;
    assert_eq!(hold_sessions.len(), 1, "Should find 1 on-hold session");
    
    // Find all UAC sessions
    let uac_sessions = store.find_sessions(|s|
        matches!(s.role, Role::UAC)
    ).await;
    assert_eq!(uac_sessions.len(), 3, "Should find all 3 UAC sessions");
}