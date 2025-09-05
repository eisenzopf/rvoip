use std::sync::Arc;
use rvoip_session_core_v2::{
    SessionStore, SessionState, SessionId, Role, CallState, EventType,
    HistoryConfig, TransitionRecord,
    types::{DialogId, MediaSessionId},
};
use futures::future;

/// Test basic session creation and state management
#[tokio::test]
async fn test_session_creation() {
    let store = Arc::new(SessionStore::new());
    
    // Test UAC session creation
    let uac_id = SessionId::new();
    let uac_session = store.create_session(uac_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create UAC session");
    
    assert_eq!(uac_session.session_id, uac_id);
    assert_eq!(uac_session.role, Role::UAC);
    assert_eq!(uac_session.call_state, CallState::Idle);
    assert!(!uac_session.dialog_established);
    assert!(!uac_session.media_session_ready);
    assert!(!uac_session.sdp_negotiated);
    
    // Test UAS session creation
    let uas_id = SessionId::new();
    let uas_session = store.create_session(uas_id.clone(), Role::UAS, false)
        .await
        .expect("Failed to create UAS session");
    
    assert_eq!(uas_session.session_id, uas_id);
    assert_eq!(uas_session.role, Role::UAS);
    assert_eq!(uas_session.call_state, CallState::Idle);
}

/// Test session updates and retrieval
#[tokio::test]
async fn test_session_updates() {
    let store = Arc::new(SessionStore::new());
    
    // Create session
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create session");
    
    // Update session state
    session.call_state = CallState::Initiating;
    session.dialog_established = true;
    session.local_uri = Some("sip:alice@example.com".to_string());
    session.remote_uri = Some("sip:bob@example.com".to_string());
    
    store.update_session(session.clone()).await.unwrap();
    
    // Retrieve and verify
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert_eq!(retrieved.call_state, CallState::Initiating);
    assert!(retrieved.dialog_established);
    assert_eq!(retrieved.local_uri, Some("sip:alice@example.com".to_string()));
    assert_eq!(retrieved.remote_uri, Some("sip:bob@example.com".to_string()));
}

/// Test session with history enabled
#[tokio::test]
async fn test_session_with_history() {
    let store = Arc::new(SessionStore::new());
    
    // Create session with history
    let session_id = SessionId::new();
    let session = store.create_session(session_id.clone(), Role::UAC, true)
        .await
        .expect("Failed to create session with history");
    
    assert!(session.history.is_some(), "History should be enabled");
    
    // Verify history is initialized
    if let Some(ref history) = session.history {
        assert_eq!(history.total_transitions, 0);
        assert_eq!(history.total_errors, 0);
        assert!(history.session_age() >= std::time::Duration::from_millis(0));
    }
    
    // Update session and add history manually
    let mut session = store.get_session(&session_id).await.unwrap();
    if let Some(ref mut history) = session.history {
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
    }
    session.call_state = CallState::Initiating;
    store.update_session(session).await.unwrap();
    
    // Verify history was preserved
    let session = store.get_session(&session_id).await.unwrap();
    if let Some(ref history) = session.history {
        assert_eq!(history.total_transitions, 1);
    }
}

/// Test all call states
#[tokio::test]
async fn test_all_call_states() {
    use rvoip_session_core_v2::state_table::types::FailureReason;
    let store = Arc::new(SessionStore::new());
    
    let states = vec![
        CallState::Idle,
        CallState::Initiating,
        CallState::Ringing,
        CallState::EarlyMedia,
        CallState::Active,
        CallState::OnHold,
        CallState::Resuming,
        CallState::Bridged,
        CallState::Transferring,
        CallState::Terminating,
        CallState::Terminated,
        CallState::Failed(FailureReason::Timeout),
        CallState::Failed(FailureReason::Rejected),
        CallState::Failed(FailureReason::NetworkError),
        CallState::Failed(FailureReason::MediaError),
        CallState::Failed(FailureReason::ProtocolError),
        CallState::Failed(FailureReason::Other),
    ];
    
    for (i, state) in states.iter().enumerate() {
        let session_id = SessionId::new();
        let mut session = store.create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect(&format!("Failed to create session {}", i));
        
        session.call_state = state.clone();
        store.update_session(session).await.unwrap();
        
        let retrieved = store.get_session(&session_id).await.unwrap();
        assert_eq!(retrieved.call_state, *state, "State mismatch for {:?}", state);
    }
}

/// Test readiness conditions
#[tokio::test]
async fn test_readiness_conditions() {
    let store = Arc::new(SessionStore::new());
    
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAS, false)
        .await
        .expect("Failed to create session");
    
    // Initially all conditions should be false
    assert!(!session.dialog_established);
    assert!(!session.media_session_ready);
    assert!(!session.sdp_negotiated);
    assert!(!session.call_established_triggered);
    
    // Set conditions one by one
    session.dialog_established = true;
    store.update_session(session.clone()).await.unwrap();
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(retrieved.dialog_established);
    
    let mut session = retrieved;
    session.media_session_ready = true;
    store.update_session(session.clone()).await.unwrap();
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(retrieved.media_session_ready);
    
    let mut session = retrieved;
    session.sdp_negotiated = true;
    store.update_session(session.clone()).await.unwrap();
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(retrieved.sdp_negotiated);
    
    // All conditions met
    assert!(retrieved.dialog_established);
    assert!(retrieved.media_session_ready);
    assert!(retrieved.sdp_negotiated);
}

/// Test SDP and negotiated config storage
#[tokio::test]
async fn test_sdp_storage() {
    use rvoip_session_core_v2::NegotiatedConfig;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    
    let store = Arc::new(SessionStore::new());
    
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create session");
    
    // Add SDP data
    session.local_sdp = Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.1\r\n".to_string());
    session.remote_sdp = Some("v=0\r\no=bob 789 012 IN IP4 192.168.1.2\r\n".to_string());
    
    // Add negotiated config
    session.negotiated_config = Some(NegotiatedConfig {
        local_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)), 5004),
        remote_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 2)), 5006),
        codec: "PCMU".to_string(),
        sample_rate: 8000,
        channels: 1,
    });
    
    store.update_session(session).await.unwrap();
    
    // Verify storage
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(retrieved.local_sdp.is_some());
    assert!(retrieved.remote_sdp.is_some());
    assert!(retrieved.negotiated_config.is_some());
    
    if let Some(config) = retrieved.negotiated_config {
        assert_eq!(config.local_addr.port(), 5004);
        assert_eq!(config.codec, "PCMU");
        assert_eq!(config.remote_addr.ip().to_string(), "192.168.1.2");
        assert_eq!(config.sample_rate, 8000);
        assert_eq!(config.channels, 1);
    }
}

/// Test related ID storage
#[tokio::test]
async fn test_related_ids() {
    let store = Arc::new(SessionStore::new());
    
    let session_id = SessionId::new();
    let mut session = store.create_session(session_id.clone(), Role::UAS, false)
        .await
        .expect("Failed to create session");
    
    // Add related IDs
    let dialog_id = DialogId::new();
    let media_id = MediaSessionId::new();
    session.dialog_id = Some(dialog_id.clone());
    session.media_session_id = Some(media_id.clone());
    session.call_id = Some("call-abcdef".to_string());
    
    store.update_session(session).await.unwrap();
    
    // Verify storage
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert_eq!(retrieved.dialog_id, Some(dialog_id));
    assert_eq!(retrieved.media_session_id, Some(media_id));
    assert_eq!(retrieved.call_id, Some("call-abcdef".to_string()));
}

/// Test session finding capabilities
#[tokio::test]
async fn test_find_sessions() {
    let store = Arc::new(SessionStore::new());
    
    // Create multiple sessions with different characteristics
    for i in 0..5 {
        let session_id = SessionId::new();
        let mut session = store.create_session(
            session_id.clone(),
            if i < 3 { Role::UAC } else { Role::UAS },
            false,
        ).await.expect("Failed to create session");
        
        session.call_state = match i {
            0 => CallState::Active,
            1 => CallState::OnHold,
            2 => CallState::Active,
            3 => CallState::Ringing,
            _ => CallState::Idle,
        };
        
        if i < 2 {
            session.dialog_established = true;
        }
        
        store.update_session(session).await.unwrap();
    }
    
    // Find all UAC sessions
    let uac_sessions = store.find_sessions(|s| s.role == Role::UAC).await;
    assert_eq!(uac_sessions.len(), 3, "Should find 3 UAC sessions");
    
    // Find all UAS sessions
    let uas_sessions = store.find_sessions(|s| s.role == Role::UAS).await;
    assert_eq!(uas_sessions.len(), 2, "Should find 2 UAS sessions");
    
    // Find active sessions
    let active_sessions = store.find_sessions(|s| s.call_state == CallState::Active).await;
    assert_eq!(active_sessions.len(), 2, "Should find 2 active sessions");
    
    // Find sessions with dialog established
    let dialog_sessions = store.find_sessions(|s| s.dialog_established).await;
    assert_eq!(dialog_sessions.len(), 2, "Should find 2 sessions with dialog");
}

/// Test concurrent session operations
#[tokio::test]
async fn test_concurrent_operations() {
    let store = Arc::new(SessionStore::new());
    
    // Create multiple sessions concurrently
    let mut handles = vec![];
    for i in 0..10 {
        let store_clone = Arc::clone(&store);
        let handle = tokio::spawn(async move {
            let session_id = SessionId::new();
            let session = store_clone.create_session(
                session_id.clone(),
                if i % 2 == 0 { Role::UAC } else { Role::UAS },
                false,
            ).await.expect("Failed to create session");
            
            // Update the session
            let mut session = session;
            session.call_state = if i < 5 { CallState::Active } else { CallState::Idle };
            store_clone.update_session(session).await.unwrap();
            
            session_id
        });
        handles.push(handle);
    }
    
    // Wait for all operations to complete
    let session_ids: Vec<SessionId> = future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap())
        .collect();
    
    // Verify all sessions exist
    for id in session_ids {
        assert!(store.get_session(&id).await.is_ok(), "Session should exist");
    }
    
    // Verify we have the right total
    let all_sessions = store.find_sessions(|_| true).await;
    assert_eq!(all_sessions.len(), 10, "Should have 10 sessions total");
}