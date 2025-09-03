use rvoip_session_core_v2::{SessionStore, SessionState, SessionId, Role, CallState};
use std::sync::Arc;

#[tokio::test]
async fn test_session_creation() {
    let store = Arc::new(SessionStore::new());
    let session_id = SessionId::new();
    
    // Create a session
    store.create_session(session_id.clone(), Role::UAC, false).await.unwrap();
    
    // Retrieve the session
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert_eq!(retrieved.session_id, session_id);
    assert_eq!(retrieved.role, Role::UAC);
    assert!(matches!(retrieved.call_state, CallState::Idle));
}

#[tokio::test]
async fn test_session_state_transition() {
    let store = Arc::new(SessionStore::new());
    let session_id = SessionId::new();
    
    // Create a session
    store.create_session(session_id.clone(), Role::UAC, false).await.unwrap();
    
    // Get and update state
    let mut session = store.get_session(&session_id).await.unwrap();
    session.call_state = CallState::Initiating;
    store.update_session(session).await.unwrap();
    
    // Verify update
    let retrieved = store.get_session(&session_id).await.unwrap();
    assert!(matches!(retrieved.call_state, CallState::Initiating));
}

#[tokio::test]
async fn test_cleanup_sessions() {
    use rvoip_session_core_v2::CleanupConfig;
    use std::time::Duration;
    
    let store = Arc::new(SessionStore::new());
    
    // Create multiple sessions
    for i in 0..5 {
        let session_id = SessionId::new();
        store.create_session(session_id.clone(), Role::UAC, false).await.unwrap();
        
        if i < 2 {
            let mut session = store.get_session(&session_id).await.unwrap();
            session.call_state = CallState::Terminated;
            store.update_session(session).await.unwrap();
        }
    }
    
    // Run cleanup
    let config = CleanupConfig {
        terminated_ttl: Duration::from_millis(1), // Very short for testing
        ..Default::default()
    };
    
    // Wait a bit
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let stats = store.cleanup_sessions(config).await;
    
    // Should have removed terminated sessions
    assert!(stats.terminated_removed >= 2);
    assert!(stats.total_removed >= 2);
}