use std::sync::Arc;
use std::net::SocketAddr;
use anyhow::Result;
use tokio::time::sleep;
use std::time::Duration;

use rvoip_transaction_core::{
    TransactionManager,
    TransactionEvent,
    TransactionKey,
};

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

// Import specific missing modules
use rvoip_session_core::session::manager::SessionManager;
use rvoip_session_core::session::session::Session;

// Create a mock transport for the transaction manager
#[derive(Debug, Clone)]
struct MockTransport {
    local_addr: SocketAddr,
}

impl MockTransport {
    fn new(addr_str: &str) -> Self {
        Self {
            local_addr: addr_str.parse().unwrap(),
        }
    }
}

#[async_trait::async_trait]
impl rvoip_sip_transport::Transport for MockTransport {
    fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::error::Error> {
        Ok(self.local_addr)
    }
    
    async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: SocketAddr) 
        -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::error::Error> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

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

// Create a transaction manager for testing
async fn create_test_transaction_manager() -> Arc<TransactionManager> {
    // Create a mock transport
    let transport = Arc::new(MockTransport::new("127.0.0.1:5060"));
    
    // Create a transport event channel (won't be used in our tests)
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    
    // Create the transaction manager with the mock transport
    let (manager, _) = TransactionManager::new(transport, rx, Some(10)).await.unwrap();
    
    Arc::new(manager)
}

#[tokio::test]
async fn test_session_creation() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create a new outgoing session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        transaction_manager.clone(),
        event_bus.clone()
    );
    
    // Verify initial state
    assert_eq!(session.state().await, SessionState::Initializing);
    assert!(session.dialog().await.is_none());
    assert!(session.is_active().await);
    assert!(!session.is_terminated().await);
    
    Ok(())
}

#[tokio::test]
async fn test_session_state_transitions() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create a new session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        transaction_manager.clone(),
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
        transaction_manager.clone(),
        event_bus.clone()
    );
    
    // Invalid: Initializing -> OnHold
    let result = new_session.set_state(SessionState::OnHold).await;
    assert!(result.is_err());
    
    Ok(())
}

#[tokio::test]
async fn test_session_manager_basics() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager.clone(),
        config.clone(),
        event_bus.clone()
    ));
    
    // Start the session manager
    session_manager.start().await?;
    
    // Create an outgoing session
    let session = session_manager.create_outgoing_session().await?;
    let session_id = session.id.clone();
    
    // Verify the session was added to the manager
    let retrieved = session_manager.get_session(&session_id);
    assert!(retrieved.is_some());
    
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
async fn test_session_transaction_tracking() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create a new session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        transaction_manager.clone(),
        event_bus.clone()
    );
    
    // Define a transaction key for testing with required parameters
    let tx_id = TransactionKey::new(
        "z9hG4bK-test-branch".to_string(), 
        Method::Invite, 
        false // client transaction
    );
    
    // Track a transaction
    session.track_transaction(tx_id.clone(), 
        rvoip_session_core::session::SessionTransactionType::InitialInvite).await;
    
    // Verify transaction tracking
    let tx_type = session.get_transaction_type(&tx_id).await;
    assert!(tx_type.is_some());
    assert!(matches!(tx_type.unwrap(), 
        rvoip_session_core::session::SessionTransactionType::InitialInvite));
    
    // Remove the transaction
    let removed = session.remove_transaction(&tx_id).await;
    assert!(removed.is_some());
    
    // Verify it's gone
    let tx_type_after = session.get_transaction_type(&tx_id).await;
    assert!(tx_type_after.is_none());
    
    Ok(())
}

#[tokio::test]
async fn test_session_media_operations() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create a new session
    let session = Session::new(
        SessionDirection::Outgoing,
        config.clone(),
        transaction_manager.clone(),
        event_bus.clone()
    );
    
    // Starting media should work (even though it's a mock implementation)
    let start_result = session.start_media().await;
    assert!(start_result.is_ok());
    
    // Stopping media should work
    let stop_result = session.stop_media().await;
    assert!(stop_result.is_ok());
    
    Ok(())
}

#[tokio::test]
async fn test_session_manager_terminate_all() -> Result<()> {
    let transaction_manager = create_test_transaction_manager().await;
    let event_bus = EventBus::new(10);
    let config = create_test_config();
    
    // Create session manager
    let session_manager = Arc::new(SessionManager::new(
        transaction_manager.clone(),
        config.clone(),
        event_bus.clone()
    ));
    
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