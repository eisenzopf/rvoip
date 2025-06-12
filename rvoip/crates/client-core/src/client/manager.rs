use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, Mutex};
use dashmap::DashMap;
use uuid::Uuid;

// Import session-core APIs
use rvoip_session_core::{
    SessionManager,
    api::{
        builder::SessionManagerBuilder,
        types::SessionId,
        handlers::CallHandler,
    },
};

// Import client-core types
use crate::{
    ClientConfig, ClientResult, ClientError,
    call::{CallId, CallInfo},
    registration::{RegistrationConfig, RegistrationInfo},
    events::ClientEventHandler,
};

// Import types from our types module
use super::types::*;
use super::events::ClientCallHandler;

/// High-level SIP client manager that coordinates all client operations
/// 
/// Delegates to session-core for all SIP/media functionality while providing
/// a client-focused API for application integration.
pub struct ClientManager {
    pub config: ClientConfig,
    pub session_manager: Arc<SessionManager>,
    pub call_handler: Arc<ClientCallHandler>,
    
    // Call tracking
    pub call_mapping: Arc<DashMap<SessionId, CallId>>,
    pub session_mapping: Arc<DashMap<CallId, SessionId>>,
    pub call_info: Arc<DashMap<CallId, CallInfo>>,
    
    // Registration tracking (placeholder - session-core doesn't support REGISTER)
    pub registrations: Arc<RwLock<HashMap<Uuid, RegistrationInfo>>>,
    
    // State
    pub is_running: Arc<RwLock<bool>>,
    pub stats: Arc<Mutex<ClientStats>>,
}

impl ClientManager {
    /// Create a new client manager with the given configuration
    pub async fn new(config: ClientConfig) -> ClientResult<Arc<Self>> {
        // Create call/session mapping
        let call_mapping = Arc::new(DashMap::new());
        let session_mapping = Arc::new(DashMap::new());
        let call_info = Arc::new(DashMap::new());
        
        // Create call handler
        let call_handler = Arc::new(ClientCallHandler::new(
            call_mapping.clone(),
            session_mapping.clone(),
            call_info.clone(),
        ));
        
        // Create session manager using session-core builder
        let session_manager = SessionManagerBuilder::new()
            .with_sip_bind_address("127.0.0.1") // Use config addr without port
            .with_sip_port(config.local_sip_addr.port())
            .with_from_uri(&format!("sip:client@{}", config.local_sip_addr.ip()))
            .with_media_ports(config.local_media_addr.port(), config.local_media_addr.port() + 100)
            .with_handler(call_handler.clone() as Arc<dyn CallHandler>)
            .build()
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to create session manager: {}", e) 
            })?;
            
        let stats = ClientStats {
            is_running: false,
            local_sip_addr: config.local_sip_addr,
            local_media_addr: config.local_media_addr,
            total_calls: 0,
            connected_calls: 0,
            total_registrations: 0,
            active_registrations: 0,
        };

        Ok(Arc::new(Self {
            config,
            session_manager,
            call_handler,
            call_mapping,
            session_mapping,
            call_info,
            registrations: Arc::new(RwLock::new(HashMap::new())),
            is_running: Arc::new(RwLock::new(false)),
            stats: Arc::new(Mutex::new(stats)),
        }))
    }
    
    /// Set the event handler for client events
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        self.call_handler.set_event_handler(handler).await;
    }
    
    /// Start the client manager
    pub async fn start(&self) -> ClientResult<()> {
        // Start the session manager
        self.session_manager.start()
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to start session manager: {}", e) 
            })?;
            
        *self.is_running.write().await = true;
        
        // Update stats with actual bound addresses
        let actual_addr = self.session_manager.get_bound_address();
        let mut stats = self.stats.lock().await;
        stats.is_running = true;
        stats.local_sip_addr = actual_addr;
        
        tracing::info!("ClientManager started on {}", actual_addr);
        Ok(())
    }
    
    /// Stop the client manager
    pub async fn stop(&self) -> ClientResult<()> {
        self.session_manager.stop()
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to stop session manager: {}", e) 
            })?;
            
        *self.is_running.write().await = false;
        
        let mut stats = self.stats.lock().await;
        stats.is_running = false;
        
        tracing::info!("ClientManager stopped");
        Ok(())
    }
    
    /// Register with a SIP server
    /// 
    /// Note: Currently not implemented as session-core doesn't expose REGISTER functionality
    pub async fn register(&self, _config: RegistrationConfig) -> ClientResult<Uuid> {
        Err(ClientError::NotImplemented { 
            feature: "SIP REGISTER".to_string(),
            reason: "session-core does not expose REGISTER functionality".to_string(),
        })
    }
    

    
    // ===== PRIORITY 3.2: CALL CONTROL OPERATIONS =====
    // Call control operations have been moved to controls.rs
    
    // ===== PRIORITY 4.1: ENHANCED MEDIA INTEGRATION =====
    // Media operations have been moved to media.rs
}


