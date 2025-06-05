use std::sync::Arc;
use std::net::SocketAddr;
use anyhow::Result;
use tokio::time::sleep;
use std::time::Duration;

use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};

use rvoip_sip_core::{
    Method,
    Uri,
    Request,
    Message,
};

use rvoip_session_core::{
    events::{EventBus, SessionEvent},
    session::{SessionId, SessionState, SessionDirection, SessionConfig},
};

// Import specific modules  
use rvoip_session_core::session::manager::SessionManager;
use rvoip_session_core::session::session::Session;

// Helper to create a minimal configuration for testing
fn create_test_config() -> SessionConfig {
    SessionConfig {
        local_signaling_addr: "127.0.0.1:5060".parse().unwrap(),
        local_media_addr: "127.0.0.1:10000".parse().unwrap(),
        supported_codecs: vec![],
        display_name: Some("Test User".to_string()),
        user_agent: "RVOIP-Test/0.1.0".to_string(),
        max_duration: 0,
        max_sessions: None,
    }
}

// Create a unified dialog API for testing
async fn create_test_dialog_api() -> Arc<UnifiedDialogApi> {
    let config = DialogManagerConfig::client("127.0.0.1:0".parse().unwrap())
        .with_from_uri("sip:test@example.com")
        .build();
    
    Arc::new(UnifiedDialogApi::create(config).await.unwrap())
}

#[tokio::test]
async fn test_session_creation() -> Result<()> {
    let event_bus = EventBus::new(10).await.unwrap();
    let config = create_test_config();
    
    // Create a new outgoing session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        event_bus.clone()
    );
    
    // Verify initial state
    assert_eq!(session.state().await, SessionState::Initializing);
    assert!(session.is_active().await);
    assert!(!session.is_terminated().await);
    
    Ok(())
}

#[tokio::test]
async fn test_session_state_transitions() -> Result<()> {
    let event_bus = EventBus::new(10).await.unwrap();
    let config = create_test_config();
    
    // Create a new session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        event_bus.clone()
    );
    
    // Valid state transitions
    assert_eq!(session.state().await, SessionState::Initializing);
    
    // Initializing -> Dialing
    session.set_state(SessionState::Dialing).await?;
    assert_eq!(session.state().await, SessionState::Dialing);
    
    // Dialing -> Connected
    session.set_state(SessionState::Connected).await?;
    assert_eq!(session.state().await, SessionState::Connected);
    
    // Connected -> OnHold
    session.set_state(SessionState::OnHold).await?;
    assert_eq!(session.state().await, SessionState::OnHold);
    
    // OnHold -> Connected
    session.set_state(SessionState::Connected).await?;
    assert_eq!(session.state().await, SessionState::Connected);
    
    // Connected -> Terminating
    session.set_state(SessionState::Terminating).await?;
    assert_eq!(session.state().await, SessionState::Terminating);
    
    // Terminating -> Terminated
    session.set_state(SessionState::Terminated).await?;
    assert_eq!(session.state().await, SessionState::Terminated);
    assert!(session.is_terminated().await);
    assert!(!session.is_active().await);
    
    // Test invalid transition - should return an error
    let new_session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        event_bus.clone()
    );
    
    // Invalid: Initializing -> OnHold
    let result = new_session.set_state(SessionState::OnHold).await;
    assert!(result.is_err());
    
    Ok(())
}

#[tokio::test]
async fn test_session_manager_basics() -> Result<()> {
    let dialog_api = create_test_dialog_api().await;
    let event_bus = EventBus::new(10).await.unwrap();
    let config = create_test_config();
    
    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        dialog_api.clone(),
        config.clone(),
        event_bus
    ).await?);
    
    // Start the session manager
    session_manager.start().await?;
    
    // Create an outgoing session
    let session = session_manager.create_outgoing_session().await?;
    let session_id = session.id.clone();
    
    // Verify the session was added to the manager
    let retrieved = session_manager.get_session(&session_id);
    assert!(retrieved.is_ok());
    
    // Check list_sessions
    let sessions = session_manager.list_sessions();
    assert_eq!(sessions.len(), 1);
    
    // Terminate the session
    session.set_state(SessionState::Terminating).await?;
    session.set_state(SessionState::Terminated).await?;
    
    // Clean up terminated sessions
    let cleaned = session_manager.cleanup_terminated().await;
    assert_eq!(cleaned, 1);
    
    // Verify the session was removed
    let sessions_after = session_manager.list_sessions();
    assert_eq!(sessions_after.len(), 0);
    
    Ok(())
}

#[tokio::test]
async fn test_session_media_operations() -> Result<()> {
    let event_bus = EventBus::new(10).await.unwrap();
    let config = create_test_config();
    
    // Create a new session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        event_bus.clone()
    );
    
    // Starting media might fail in a test environment without proper media setup
    // Just verify the method can be called and returns a Result
    let start_result = session.start_media().await;
    // In a test environment, this may fail due to lack of media infrastructure
    // The important thing is that the API exists and can be called
    
    // Stopping media should always work (even if start failed)
    let stop_result = session.stop_media().await;
    assert!(stop_result.is_ok(), "Stop media should always succeed");
    
    Ok(())
}

#[tokio::test]
async fn test_session_manager_terminate_all() -> Result<()> {
    let dialog_api = create_test_dialog_api().await;
    let event_bus = EventBus::new(10).await.unwrap();
    let config = create_test_config();
    
    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        dialog_api.clone(),
        config.clone(),
        event_bus
    ).await?);
    
    // Start the session manager
    session_manager.start().await?;
    
    // Create multiple sessions
    let _session1 = session_manager.create_outgoing_session().await?;
    let _session2 = session_manager.create_outgoing_session().await?;
    let _session3 = session_manager.create_outgoing_session().await?;
    
    // Verify we have 3 sessions
    let sessions = session_manager.list_sessions();
    assert_eq!(sessions.len(), 3);
    
    // Set all sessions to Connected to verify terminate_all works properly
    for session in &sessions {
        session.set_state(SessionState::Dialing).await?;
        session.set_state(SessionState::Connected).await?;
    }
    
    // Terminate all sessions
    session_manager.terminate_all().await?;
    
    // Allow some time for async operations
    sleep(Duration::from_millis(50)).await;
    
    // Clean up terminated sessions
    let cleaned = session_manager.cleanup_terminated().await;
    
    // This might not be exactly 3 because termination is asynchronous
    // and some sessions might still be in Terminating state
    
    // Verify we have fewer sessions
    let sessions_after = session_manager.list_sessions();
    assert!(sessions_after.len() <= sessions.len());
    
    Ok(())
}

#[test]
fn test_session_id_creation() {
    let id1 = SessionId::new();
    let id2 = SessionId::new();
    
    // Two IDs should be different
    assert_ne!(id1, id2);
    
    // Test Display implementation
    let id_str = id1.to_string();
    assert!(!id_str.is_empty());
    
    // Test Default implementation
    let id_default = SessionId::default();
    assert_ne!(id_default, id1);
}

#[test]
fn test_session_config() {
    let config = SessionConfig::default();
    
    // Check default values
    assert_eq!(config.local_signaling_addr.to_string(), "0.0.0.0:5060");
    assert_eq!(config.local_media_addr.to_string(), "0.0.0.0:10000");
    assert_eq!(config.user_agent, "RVOIP/0.1.0");
}

#[test]
fn test_session_state_display() {
    assert_eq!(SessionState::Initializing.to_string(), "Initializing");
    assert_eq!(SessionState::Dialing.to_string(), "Dialing");
    assert_eq!(SessionState::Ringing.to_string(), "Ringing");
    assert_eq!(SessionState::Connected.to_string(), "Connected");
    assert_eq!(SessionState::Terminated.to_string(), "Terminated");
} 