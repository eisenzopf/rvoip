//! Client API for RVOIP Session Core
//!
//! This module provides client-specific functionality for building SIP clients,
//! including factory functions, configuration, and client-oriented helper methods.

pub mod config;

// Re-export the new config types
pub use config::{ClientConfig, ClientCredentials};

use crate::{
    session::{SessionManager, SessionConfig, SessionDirection},
    events::{EventBus, SessionEvent},
    media::{MediaManager, MediaConfig},
    Error, SessionId, Session
};
use std::sync::Arc;
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::Uri;

/// Legacy client configuration for backward compatibility
#[derive(Debug, Clone)]
pub struct LegacyClientConfig {
    /// Display name for outgoing calls
    pub display_name: String,
    
    /// Default SIP URI for this client
    pub uri: String,
    
    /// Default contact address
    pub contact: String,
    
    /// Authentication username
    pub auth_user: Option<String>,
    
    /// Authentication password
    pub auth_password: Option<String>,
    
    /// Registration interval (in seconds)
    pub registration_interval: Option<u32>,
    
    /// User agent string
    pub user_agent: String,
    
    /// Maximum concurrent calls
    pub max_concurrent_calls: usize,
    
    /// Auto-answer incoming calls
    pub auto_answer: bool,
    
    /// Session configuration
    pub session_config: SessionConfig,
}

impl Default for LegacyClientConfig {
    fn default() -> Self {
        Self {
            display_name: "RVOIP Client".to_string(),
            uri: "sip:user@example.com".to_string(),
            contact: "sip:user@127.0.0.1:5060".to_string(),
            auth_user: None,
            auth_password: None,
            registration_interval: Some(3600),
            user_agent: "RVOIP-Client/1.0".to_string(),
            max_concurrent_calls: 10,
            auto_answer: false,
            session_config: SessionConfig::default(),
        }
    }
}

/// Client-specific session manager with enhanced client functionality
pub struct ClientSessionManager {
    /// Core session manager
    session_manager: Arc<SessionManager>,
    
    /// Media manager for coordinating RTP streams
    media_manager: Arc<MediaManager>,
    
    /// Client configuration
    config: ClientConfig,
    
    /// Registration state
    registered: Arc<std::sync::atomic::AtomicBool>,
}

impl ClientSessionManager {
    /// Create a new client session manager
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: ClientConfig
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let event_bus = EventBus::new(100).await?;
        
        // Convert new ClientConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()),
            ..Default::default()
        };
        
        let session_manager = SessionManager::new(
            transaction_manager,
            session_config,
            event_bus
        ).await?;
        
        // Create media manager for coordinating RTP streams
        let media_manager = MediaManager::new().await?;
        
        Ok(Self {
            session_manager: Arc::new(session_manager),
            media_manager: Arc::new(media_manager),
            config,
            registered: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    
    /// Create a new client session manager (synchronous)
    pub fn new_sync(
        transaction_manager: Arc<TransactionManager>,
        config: ClientConfig
    ) -> Self {
        let event_bus = EventBus::new_simple(100);
        
        // Convert new ClientConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()),
            ..Default::default()
        };
        
        let session_manager = SessionManager::new_sync(
            transaction_manager,
            session_config,
            event_bus
        );
        
        // Create media manager for coordinating RTP streams
        let media_manager = tokio::runtime::Handle::current().block_on(async {
            MediaManager::new().await.expect("Failed to create media manager")
        });
        
        Self {
            session_manager: Arc::new(session_manager),
            media_manager: Arc::new(media_manager),
            config,
            registered: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
    
    /// Get the underlying session manager
    pub fn session_manager(&self) -> &Arc<SessionManager> {
        &self.session_manager
    }
    
    /// Get the client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }
    
    /// Make an outgoing call
    pub async fn make_call(&self, destination: Uri) -> Result<Arc<Session>, Error> {
        // Check if we're at the maximum concurrent calls
        let active_sessions = self.session_manager.list_sessions();
        if active_sessions.len() >= self.config.max_sessions {
            return Err(Error::ResourceLimitExceeded(
                format!("Maximum concurrent calls ({}) reached", self.config.max_sessions),
                crate::errors::ErrorContext {
                    category: crate::errors::ErrorCategory::Resource,
                    severity: crate::errors::ErrorSeverity::Error,
                    recovery: crate::errors::RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                    retryable: true,
                    timestamp: std::time::SystemTime::now(),
                    details: Some("Too many concurrent calls".to_string()),
                    ..Default::default()
                }
            ));
        }
        
        // Create outgoing session
        let session = self.session_manager.create_outgoing_session().await?;
        
        // FIXED: Automatically set up media coordination
        let media_config = MediaConfig {
            local_addr: "127.0.0.1:10000".parse().unwrap(),
            remote_addr: None,
            media_type: crate::media::SessionMediaType::Audio,
            payload_type: 0,
            clock_rate: 8000,
            audio_codec: crate::media::AudioCodecType::PCMU,
            direction: crate::media::SessionMediaDirection::SendRecv,
        };
        
        // Create media session and associate with SIP session
        let media_session_id = self.media_manager.create_media_session(media_config).await
            .map_err(|e| Error::MediaResourceError(
                format!("Failed to create media session: {}", e),
                crate::errors::ErrorContext::default()
            ))?;
        
        // Set media session ID on the session
        session.set_media_session_id(Some(media_session_id.clone())).await;
        
        // Set media state to negotiating (will be configured after SDP negotiation)
        session.set_media_negotiating().await?;
        
        // Set initial session state
        session.set_state(crate::session::session_types::SessionState::Dialing).await?;
        
        Ok(session)
    }
    
    /// Answer an incoming call
    pub async fn answer_call(&self, session_id: &SessionId) -> Result<(), Error> {
        let session = self.session_manager.get_session(session_id)?;
        
        // Check current state
        let current_state = session.state().await;
        if current_state != crate::session::session_types::SessionState::Ringing {
            return Err(Error::InvalidSessionStateTransition { 
                from: current_state.to_string(), 
                to: crate::session::session_types::SessionState::Connected.to_string(),
                context: crate::errors::ErrorContext::default()
            });
        }
        
        // Set connected state
        session.set_state(crate::session::session_types::SessionState::Connected).await?;
        
        Ok(())
    }
    
    /// End a call
    pub async fn end_call(&self, session_id: &SessionId) -> Result<(), Error> {
        let session = self.session_manager.get_session(session_id)?;
        
        // FIXED: Automatically stop media when ending call
        if let Some(media_session_id) = session.media_session_id().await {
            // Stop media in the media manager
            self.media_manager.stop_media(&media_session_id, "Call ended".to_string()).await
                .map_err(|e| Error::MediaResourceError(
                    format!("Failed to stop media: {}", e),
                    crate::errors::ErrorContext::default()
                ))?;
        }
        
        // Stop session media
        session.stop_media().await?;
        
        // Set terminating state
        let _ = session.set_state(crate::session::session_types::SessionState::Terminating).await;
        
        // Then set terminated state
        session.set_state(crate::session::session_types::SessionState::Terminated).await?;
        
        Ok(())
    }
    
    /// Transfer a call
    pub async fn transfer_call(
        &self,
        session_id: &SessionId,
        target_uri: String,
        transfer_type: crate::session::session_types::TransferType
    ) -> Result<crate::session::session_types::TransferId, Error> {
        self.session_manager.initiate_transfer(
            session_id,
            target_uri,
            transfer_type,
            self.config.from_uri.clone()
        ).await
    }
    
    /// Put a call on hold
    pub async fn hold_call(&self, session_id: &SessionId) -> Result<(), Error> {
        let session = self.session_manager.get_session(session_id)?;
        
        // FIXED: Automatically pause media when putting call on hold
        session.pause_media().await?;
        
        // If we have a media session, pause it in the media manager too
        if let Some(media_session_id) = session.media_session_id().await {
            self.media_manager.pause_media(&media_session_id).await
                .map_err(|e| Error::MediaResourceError(
                    format!("Failed to pause media: {}", e),
                    crate::errors::ErrorContext::default()
                ))?;
        }
        
        Ok(())
    }
    
    /// Resume a held call
    pub async fn resume_call(&self, session_id: &SessionId) -> Result<(), Error> {
        let session = self.session_manager.get_session(session_id)?;
        
        // FIXED: Automatically resume media when resuming call
        session.resume_media().await?;
        
        // If we have a media session, resume it in the media manager too
        if let Some(media_session_id) = session.media_session_id().await {
            self.media_manager.resume_media(&media_session_id).await
                .map_err(|e| Error::MediaResourceError(
                    format!("Failed to resume media: {}", e),
                    crate::errors::ErrorContext::default()
                ))?;
        }
        
        Ok(())
    }
    
    /// Get all active calls
    pub fn get_active_calls(&self) -> Vec<Arc<Session>> {
        self.session_manager.list_sessions()
    }
    
    /// Check if registered
    pub fn is_registered(&self) -> bool {
        self.registered.load(std::sync::atomic::Ordering::SeqCst)
    }
    
    /// Set registration state
    pub fn set_registered(&self, registered: bool) {
        self.registered.store(registered, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Create a session manager configured for client use
pub async fn create_client_session_manager(
    transaction_manager: Arc<TransactionManager>,
    config: ClientConfig
) -> Result<Arc<SessionManager>, Box<dyn std::error::Error>> {
    let client_manager = ClientSessionManager::new(transaction_manager, config).await?;
    Ok(client_manager.session_manager().clone())
}

/// Create a session manager configured for client use (synchronous)
pub fn create_client_session_manager_sync(
    transaction_manager: Arc<TransactionManager>,
    config: ClientConfig
) -> Arc<SessionManager> {
    let client_manager = ClientSessionManager::new_sync(transaction_manager, config);
    client_manager.session_manager().clone()
}

/// Create a full-featured client session manager
pub async fn create_full_client_manager(
    transaction_manager: Arc<TransactionManager>,
    config: ClientConfig
) -> Result<Arc<ClientSessionManager>, Box<dyn std::error::Error>> {
    let client_manager = ClientSessionManager::new(transaction_manager, config).await?;
    Ok(Arc::new(client_manager))
}

/// Create a full-featured client session manager (synchronous)
pub fn create_full_client_manager_sync(
    transaction_manager: Arc<TransactionManager>,
    config: ClientConfig
) -> Arc<ClientSessionManager> {
    let client_manager = ClientSessionManager::new_sync(transaction_manager, config);
    Arc::new(client_manager)
} 