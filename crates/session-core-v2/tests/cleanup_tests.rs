use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use rvoip_session_core_v2::{
    SessionStore, SessionId, Role, CallState, CleanupConfig,
};

#[tokio::test]
async fn test_cleanup_terminated_sessions() {
    let store = Arc::new(SessionStore::new());
    
    // Create sessions in different states
    let mut terminated_ids = Vec::new();
    let mut active_ids = Vec::new();
    
    for i in 0..5 {
        let session_id = SessionId::new();
        
        // Create session with history enabled
        let mut session = store.create_session(session_id.clone(), Role::UAC, true)
            .await
            .expect("Failed to create session");
        
        if i < 3 {
            // Mark as terminated
            session.call_state = CallState::Terminated;
            terminated_ids.push(session_id.clone());
        } else {
            // Mark as active
            session.call_state = CallState::Active;
            active_ids.push(session_id.clone());
        }
        
        // Update the session in the store
        store.update_session(session).await.unwrap();
    }
    
    // Wait to ensure terminated sessions age beyond the TTL
    sleep(Duration::from_millis(100)).await;
    
    // Run cleanup with very short TTL for terminated sessions
    let config = CleanupConfig {
        enabled: true,
        interval: Duration::from_secs(1),
        terminated_ttl: Duration::from_millis(50),
        failed_ttl: Duration::from_millis(50),
        max_idle_time: Duration::from_secs(3600),
        max_session_age: Duration::from_secs(3600),
        max_memory_bytes: None,
        max_sessions: None,
    };
    
    let stats = store.cleanup_sessions(config).await;
    
    // Verify cleanup statistics
    assert_eq!(stats.terminated_removed, 3, "Should have removed 3 terminated sessions");
    assert_eq!(stats.active_preserved, 2, "Should have preserved 2 active sessions");
    assert_eq!(stats.total_removed, 3, "Total removed should be 3");
    assert_eq!(stats.sessions_checked, 5, "Should have checked all 5 sessions");
    
    // Verify terminated sessions are actually gone
    for id in terminated_ids {
        let result = store.get_session(&id).await;
        assert!(result.is_err(), "Terminated session {} should have been removed", id);
    }
    
    // Verify active sessions still exist
    for id in active_ids {
        let result = store.get_session(&id).await;
        assert!(result.is_ok(), "Active session {} should still exist", id);
        let session = result.unwrap();
        assert_eq!(session.call_state, CallState::Active, "Session should still be active");
    }
}

#[tokio::test] 
async fn test_cleanup_idle_sessions() {
    let store = Arc::new(SessionStore::new());
    
    // Create an idle session with history enabled (required for idle tracking)
    let idle_id = SessionId::new();
    let idle_session = store.create_session(idle_id.clone(), Role::UAS, true)
        .await
        .expect("Failed to create idle session");
    store.update_session(idle_session).await.unwrap();
    
    // Sleep to let idle session age
    sleep(Duration::from_millis(100)).await;
    
    // Create an active session and update it to refresh last_activity
    let active_id = SessionId::new();
    let mut active_session = store.create_session(active_id.clone(), Role::UAC, true)
        .await
        .expect("Failed to create active session");
    
    // Update active session to refresh its last_activity timestamp
    active_session.call_state = CallState::Active;
    store.update_session(active_session).await.unwrap();
    
    // Run cleanup with very short idle timeout
    let config = CleanupConfig {
        enabled: true,
        interval: Duration::from_secs(1),
        terminated_ttl: Duration::from_secs(3600),
        failed_ttl: Duration::from_secs(3600),
        max_idle_time: Duration::from_millis(50),
        max_session_age: Duration::from_secs(3600),
        max_memory_bytes: None,
        max_sessions: None,
    };
    
    let stats = store.cleanup_sessions(config).await;
    
    // Should have cleaned up idle session but not the recently updated one
    assert_eq!(stats.idle_removed, 1, "Should have removed 1 idle session");
    assert!(stats.total_removed >= 1, "Total removed should be at least 1");
    
    // Verify idle session is gone
    assert!(store.get_session(&idle_id).await.is_err(), "Idle session should be removed");
    
    // Verify active session still exists
    assert!(store.get_session(&active_id).await.is_ok(), "Active session should still exist");
}

#[tokio::test]
async fn test_cleanup_with_session_limit() {
    let store = Arc::new(SessionStore::new());
    
    // Create 10 sessions with delays to ensure different creation times
    let mut session_ids = Vec::new();
    for i in 0..10 {
        let id = SessionId::new();
        let session = store.create_session(id.clone(), Role::UAC, false)
            .await
            .expect(&format!("Failed to create session {}", i));
        store.update_session(session).await.unwrap();
        session_ids.push(id);
        
        // Small delay to ensure different timestamps
        sleep(Duration::from_millis(10)).await;
    }
    
    // Run cleanup with max 5 sessions
    let config = CleanupConfig {
        enabled: true,
        interval: Duration::from_secs(1),
        terminated_ttl: Duration::from_secs(3600),
        failed_ttl: Duration::from_secs(3600),
        max_idle_time: Duration::from_secs(3600),
        max_session_age: Duration::from_secs(3600),
        max_memory_bytes: None,
        max_sessions: Some(5),
    };
    
    let stats = store.cleanup_sessions(config).await;
    
    // Should have removed 5 oldest sessions to stay within limit
    assert_eq!(stats.memory_pressure_removed, 5, "Should have removed 5 sessions due to limit");
    assert_eq!(stats.total_removed, 5, "Total removed should be 5");
    
    // Count remaining sessions
    let remaining_count = store.find_sessions(|_| true).await.len();
    assert_eq!(remaining_count, 5, "Should have exactly 5 sessions remaining");
    
    // Verify that the oldest sessions were removed (first 5)
    for i in 0..5 {
        assert!(store.get_session(&session_ids[i]).await.is_err(), 
                "Old session {} should be removed", i);
    }
    
    // Verify that the newest sessions remain (last 5)
    for i in 5..10 {
        assert!(store.get_session(&session_ids[i]).await.is_ok(), 
                "New session {} should still exist", i);
    }
}

#[tokio::test]
async fn test_cleanup_disabled() {
    let store = Arc::new(SessionStore::new());
    
    // Create some terminated sessions that would normally be cleaned up
    for _ in 0..3 {
        let id = SessionId::new();
        let mut session = store.create_session(id, Role::UAC, false)
            .await
            .expect("Failed to create session");
        session.call_state = CallState::Terminated;
        store.update_session(session).await.unwrap();
    }
    
    // Wait to ensure sessions would be old enough for cleanup
    sleep(Duration::from_millis(10)).await;
    
    // Run cleanup with disabled flag - even with aggressive TTLs
    let config = CleanupConfig {
        enabled: false,  // Cleanup is disabled
        interval: Duration::from_secs(1),
        terminated_ttl: Duration::from_millis(1),  // Very aggressive TTL
        failed_ttl: Duration::from_millis(1),
        max_idle_time: Duration::from_millis(1),
        max_session_age: Duration::from_millis(1),
        max_memory_bytes: None,
        max_sessions: None,
    };
    
    let stats = store.cleanup_sessions(config).await;
    
    // Should not remove any sessions when disabled
    assert_eq!(stats.total_removed, 0, "Should not remove any sessions when disabled");
    assert_eq!(stats.sessions_checked, 0, "Should not check any sessions when disabled");
    
    // Verify all sessions still exist
    let sessions = store.find_sessions(|_| true).await;
    assert_eq!(sessions.len(), 3, "All 3 sessions should still exist");
}

#[tokio::test]
async fn test_cleanup_failed_sessions() {
    let store = Arc::new(SessionStore::new());
    
    // Create sessions with different failure reasons
    let mut failed_ids = Vec::new();
    for reason in [
        CallState::Failed(rvoip_session_core_v2::state_table::types::FailureReason::Timeout),
        CallState::Failed(rvoip_session_core_v2::state_table::types::FailureReason::Rejected),
        CallState::Failed(rvoip_session_core_v2::state_table::types::FailureReason::NetworkError),
    ] {
        let id = SessionId::new();
        let mut session = store.create_session(id.clone(), Role::UAS, false)
            .await
            .expect("Failed to create session");
        session.call_state = reason;
        store.update_session(session).await.unwrap();
        failed_ids.push(id);
    }
    
    // Create one active session
    let active_id = SessionId::new();
    let mut active_session = store.create_session(active_id.clone(), Role::UAC, false)
        .await
        .expect("Failed to create active session");
    active_session.call_state = CallState::Active;  // Actually mark it as active
    store.update_session(active_session).await.unwrap();
    
    // Wait to let failed sessions age
    sleep(Duration::from_millis(100)).await;
    
    // Run cleanup with short failed TTL
    let config = CleanupConfig {
        enabled: true,
        interval: Duration::from_secs(1),
        terminated_ttl: Duration::from_secs(3600),
        failed_ttl: Duration::from_millis(50),
        max_idle_time: Duration::from_secs(3600),
        max_session_age: Duration::from_secs(3600),
        max_memory_bytes: None,
        max_sessions: None,
    };
    
    let stats = store.cleanup_sessions(config).await;
    
    // Should have removed all failed sessions
    assert_eq!(stats.failed_removed, 3, "Should have removed 3 failed sessions");
    assert_eq!(stats.active_preserved, 1, "Should have preserved 1 active session");
    
    // Verify failed sessions are gone
    for id in failed_ids {
        assert!(store.get_session(&id).await.is_err(), "Failed session should be removed");
    }
    
    // Verify active session still exists
    assert!(store.get_session(&active_id).await.is_ok(), "Active session should still exist");
}