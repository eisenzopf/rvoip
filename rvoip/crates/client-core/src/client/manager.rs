use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, Mutex};
use dashmap::DashMap;
use uuid::Uuid;

// Import session-core APIs - UPDATED to use new API structure
use rvoip_session_core::api::{
    SessionCoordinator,
    SessionManagerBuilder,
    SessionControl,
    SipClient,
    types::SessionId,
    handlers::CallHandler,
};

// Import client-core types
use crate::{
    ClientConfig, ClientResult, ClientError,
    call::{CallId, CallInfo},
    registration::{RegistrationConfig, RegistrationInfo},
    events::{ClientEventHandler, ClientEvent},
};

// Import types from our types module
use super::types::*;
use super::events::ClientCallHandler;

/// High-level SIP client manager that coordinates all client operations
/// 
/// Delegates to session-core for all SIP/media functionality while providing
/// a client-focused API for application integration.
pub struct ClientManager {
    /// Session coordinator from session-core
    pub(crate) coordinator: Arc<SessionCoordinator>,
    
    /// Local SIP address (bound)
    pub(crate) local_sip_addr: std::net::SocketAddr,
    
    /// Whether the client is running
    pub(crate) is_running: Arc<RwLock<bool>>,
    
    /// Statistics
    pub(crate) stats: Arc<Mutex<ClientStats>>,
    
    /// Active registrations
    pub(crate) registrations: Arc<RwLock<HashMap<Uuid, RegistrationInfo>>>,
    
    /// Call/Session mapping (CallId -> SessionId)
    pub(crate) session_mapping: Arc<DashMap<CallId, SessionId>>,
    
    /// Call info storage
    pub(crate) call_info: Arc<DashMap<CallId, CallInfo>>,
    
    /// Call handler
    pub(crate) call_handler: Arc<ClientCallHandler>,
    
    /// Event broadcast channel
    pub(crate) event_tx: tokio::sync::broadcast::Sender<ClientEvent>,
}

impl ClientManager {
    /// Create a new client manager with the given configuration
    pub async fn new(config: ClientConfig) -> ClientResult<Arc<Self>> {
        // Create call/session mapping
        let call_mapping = Arc::new(DashMap::new());
        let session_mapping = Arc::new(DashMap::new());
        let call_info = Arc::new(DashMap::new());
        let incoming_calls = Arc::new(DashMap::new());
        
        // Create event broadcast channel
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        
        // Create call handler
        let call_handler = Arc::new(ClientCallHandler::new(
            call_mapping.clone(),
            session_mapping.clone(),
            call_info.clone(),
            incoming_calls.clone(),
        ).with_event_tx(event_tx.clone()));
        
        // Create session manager using session-core builder
        let coordinator = SessionManagerBuilder::new()
            .with_local_address(&format!("sip:client@{}", config.local_sip_addr.ip()))
            .with_sip_port(config.local_sip_addr.port())
            .with_media_ports(config.local_media_addr.port(), config.local_media_addr.port() + 100)
            .with_handler(call_handler.clone() as Arc<dyn CallHandler>)
            .enable_sip_client()  // Enable SIP client features for REGISTER support
            .build()
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to create session coordinator: {}", e) 
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
            coordinator,
            local_sip_addr: config.local_sip_addr,
            is_running: Arc::new(RwLock::new(false)),
            stats: Arc::new(Mutex::new(stats)),
            registrations: Arc::new(RwLock::new(HashMap::new())),
            session_mapping,
            call_info,
            call_handler,
            event_tx,
        }))
    }
    
    /// Set the event handler for client events
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        self.call_handler.set_event_handler(handler).await;
    }
    
    /// Start the client manager
    pub async fn start(&self) -> ClientResult<()> {
        // Start the session coordinator using SessionControl trait
        SessionControl::start(&self.coordinator)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to start session coordinator: {}", e) 
            })?;
            
        *self.is_running.write().await = true;
        
        // Update stats with actual bound addresses
        let actual_addr = SessionControl::get_bound_address(&self.coordinator);
        let mut stats = self.stats.lock().await;
        stats.is_running = true;
        stats.local_sip_addr = actual_addr;
        
        tracing::info!("ClientManager started on {}", actual_addr);
        Ok(())
    }
    
    /// Stop the client manager
    pub async fn stop(&self) -> ClientResult<()> {
        SessionControl::stop(&self.coordinator)
            .await
            .map_err(|e| ClientError::InternalError { 
                message: format!("Failed to stop session coordinator: {}", e) 
            })?;
            
        *self.is_running.write().await = false;
        
        let mut stats = self.stats.lock().await;
        stats.is_running = false;
        
        tracing::info!("ClientManager stopped");
        Ok(())
    }
    
    /// Register with a SIP server
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        // Use SipClient trait to register
        let registration_handle = SipClient::register(
            &self.coordinator,
            &config.server_uri,
            &config.from_uri,
            &config.contact_uri,
            config.expires,
        )
        .await
        .map_err(|e| ClientError::InternalError { 
            message: format!("Failed to register: {}", e) 
        })?;
        
        // Create registration info
        let reg_id = Uuid::new_v4();
        let registration_info = RegistrationInfo {
            id: reg_id,
            server_uri: config.server_uri.clone(),
            from_uri: config.from_uri.clone(),
            contact_uri: config.contact_uri.clone(),
            expires: config.expires,
            status: crate::registration::RegistrationStatus::Active,
            registration_time: chrono::Utc::now(),
            refresh_time: None,
            handle: Some(registration_handle),
        };
        
        // Store registration
        self.registrations.write().await.insert(reg_id, registration_info);
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.total_registrations += 1;
        stats.active_registrations += 1;
        
        // Broadcast registration event
        let _ = self.event_tx.send(ClientEvent::RegistrationStatusChanged {
            info: crate::events::RegistrationStatusInfo {
                registration_id: reg_id,
                server_uri: config.server_uri.clone(),
                user_uri: config.from_uri.clone(),
                status: crate::registration::RegistrationStatus::Active,
                reason: Some("Registration successful".to_string()),
                timestamp: chrono::Utc::now(),
            },
            priority: crate::events::EventPriority::Normal,
        });
        
        tracing::info!("Registered {} with server {}", config.from_uri, config.server_uri);
        Ok(reg_id)
    }
    
    /// Unregister from a SIP server
    pub async fn unregister(&self, reg_id: Uuid) -> ClientResult<()> {
        let mut registrations = self.registrations.write().await;
        
        if let Some(registration_info) = registrations.get_mut(&reg_id) {
            // To unregister, send REGISTER with expires=0
            if let Some(handle) = &registration_info.handle {
                SipClient::register(
                    &self.coordinator,
                    &handle.registrar_uri,
                    &registration_info.from_uri,
                    &handle.contact_uri,
                    0, // expires=0 means unregister
                )
                .await
                .map_err(|e| ClientError::InternalError { 
                    message: format!("Failed to unregister: {}", e) 
                })?;
            }
            
            // Update status
            registration_info.status = crate::registration::RegistrationStatus::Cancelled;
            registration_info.handle = None;
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
            
            tracing::info!("Unregistered {}", registration_info.from_uri);
            Ok(())
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
        }
    }
    
    /// Get registration information
    pub async fn get_registration(&self, reg_id: Uuid) -> ClientResult<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.get(&reg_id)
            .cloned()
            .ok_or(ClientError::InvalidConfiguration { 
                field: "registration_id".to_string(),
                reason: "Registration not found".to_string() 
            })
    }
    
    /// Get all active registrations
    pub async fn get_all_registrations(&self) -> Vec<crate::registration::RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.values()
            .filter(|r| r.status == crate::registration::RegistrationStatus::Active)
            .cloned()
            .collect()
    }
    
    /// Refresh a registration
    pub async fn refresh_registration(&self, reg_id: Uuid) -> ClientResult<()> {
        // Get registration data
        let (registrar_uri, from_uri, contact_uri, expires) = {
            let registrations = self.registrations.read().await;
            
            if let Some(registration_info) = registrations.get(&reg_id) {
                if let Some(handle) = &registration_info.handle {
                    (
                        handle.registrar_uri.clone(),
                        registration_info.from_uri.clone(),
                        handle.contact_uri.clone(),
                        registration_info.expires,
                    )
                } else {
                    return Err(ClientError::InvalidConfiguration { 
                        field: "registration".to_string(),
                        reason: "Registration has no handle".to_string() 
                    });
                }
            } else {
                return Err(ClientError::InvalidConfiguration { 
                    field: "registration_id".to_string(),
                    reason: "Registration not found".to_string() 
                });
            }
        };
        
        // Re-register with the same parameters
        let new_handle = SipClient::register(
            &self.coordinator,
            &registrar_uri,
            &from_uri,
            &contact_uri,
            expires,
        )
        .await
        .map_err(|e| ClientError::InternalError { 
            message: format!("Failed to refresh registration: {}", e) 
        })?;
        
        // Update registration with new handle
        let mut registrations = self.registrations.write().await;
        if let Some(reg) = registrations.get_mut(&reg_id) {
            reg.handle = Some(new_handle);
            reg.refresh_time = Some(chrono::Utc::now());
        }
        
        tracing::info!("Refreshed registration for {}", from_uri);
        Ok(())
    }
    
    /// Clear expired registrations
    pub async fn clear_expired_registrations(&self) {
        let mut registrations = self.registrations.write().await;
        let mut to_remove = Vec::new();
        
        for (id, reg) in registrations.iter() {
            if reg.status == crate::registration::RegistrationStatus::Expired {
                to_remove.push(*id);
            }
        }
        
        for id in to_remove {
            registrations.remove(&id);
            
            // Update stats
            let mut stats = self.stats.lock().await;
            if stats.active_registrations > 0 {
                stats.active_registrations -= 1;
            }
        }
    }
    
    // ===== CONVENIENCE METHODS FOR EXAMPLES =====
    
    /// Convenience method: Register with simple parameters (for examples)
    pub async fn register_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr,
        duration: std::time::Duration
    ) -> ClientResult<()> {
        let config = RegistrationConfig {
            server_uri: format!("sip:{}", server_addr),
            from_uri: agent_uri.to_string(),
            contact_uri: format!("sip:{}:{}", self.local_sip_addr.ip(), self.local_sip_addr.port()),
            expires: duration.as_secs() as u32,
            username: None,
            password: None,
            realm: None,
        };
        
        self.register(config).await?;
        Ok(())
    }
    
    /// Convenience method: Unregister with simple parameters (for examples)
    pub async fn unregister_simple(
        &self, 
        agent_uri: &str, 
        server_addr: &std::net::SocketAddr
    ) -> ClientResult<()> {
        // Find the registration matching these parameters
        let registrations = self.registrations.read().await;
        let reg_id = registrations.iter()
            .find(|(_, reg)| {
                reg.from_uri == agent_uri && 
                reg.server_uri == format!("sip:{}", server_addr)
            })
            .map(|(id, _)| *id);
        drop(registrations);
        
        if let Some(id) = reg_id {
            self.unregister(id).await
        } else {
            Err(ClientError::InvalidConfiguration { 
                field: "registration".to_string(),
                reason: "No matching registration found".to_string() 
            })
        }
    }
    
    /// Subscribe to client events
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ClientEvent> {
        self.event_tx.subscribe()
    }
    
    // ===== PRIORITY 3.2: CALL CONTROL OPERATIONS =====
    // Call control operations have been moved to controls.rs
    
    // ===== PRIORITY 4.1: ENHANCED MEDIA INTEGRATION =====
    // Media operations have been moved to media.rs
}


