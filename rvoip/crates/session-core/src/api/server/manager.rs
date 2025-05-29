//! Server Manager
//!
//! This module provides high-level server operations for handling incoming calls,
//! managing sessions, and coordinating with the transport layer via transaction-core.
//!
//! **ARCHITECTURAL PRINCIPLE**: session-core is a COORDINATOR, not a SIP protocol handler.
//! - session-core REACTS to transaction events from transaction-core
//! - session-core COORDINATES between SIP signaling and media processing
//! - session-core NEVER sends SIP responses directly (that's transaction-core's job)

use std::sync::Arc;
use std::collections::HashMap;
use anyhow::{Result, Context};
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};
use uuid::Uuid;
use std::net::SocketAddr;
use async_trait::async_trait;

use rvoip_sip_core::{Request, Response, StatusCode, Method, Message};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
use crate::api::server::config::ServerConfig;
use crate::api::server::{IncomingCallEvent, CallerInfo, CallDecision, IncomingCallNotification};
use crate::session::{SessionManager, Session};
use crate::transport::{SessionTransportEvent, TransportIntegration};
use crate::{SessionId, Error};

/// High-level server manager for policy decisions and call coordination
/// 
/// **ARCHITECTURAL PRINCIPLE**: ServerManager makes policy decisions and delegates implementation.
/// - Decides whether to accept/reject incoming calls based on server policy
/// - Delegates all SIP implementation to SessionManager
/// - Receives notifications about call events for logging/monitoring
#[derive(Clone)]
pub struct ServerManager {
    /// Core session manager for SIP implementation
    session_manager: Arc<SessionManager>,
    
    /// Transaction manager reference (for coordination only)
    transaction_manager: Arc<TransactionManager>,
    
    /// Server configuration and policies
    config: ServerConfig,
}

#[async_trait]
impl IncomingCallNotification for ServerManager {
    /// Policy decision for incoming calls
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision {
        info!("🎯 ServerManager making policy decision for incoming call from {}", event.caller_info.from);
        
        // Example server policy logic - customize based on needs
        if self.should_accept_call(&event).await {
            info!("✅ ServerManager policy: ACCEPT call from {}", event.caller_info.from);
            CallDecision::Accept
        } else {
            info!("❌ ServerManager policy: REJECT call from {}", event.caller_info.from);
            CallDecision::Reject { 
                status_code: StatusCode::BusyHere, 
                reason: Some("Server busy".to_string()) 
            }
        }
    }
    
    /// Notification that a call was terminated by remote party
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String) {
        info!("📞 ServerManager notified: Call {} (session {}) terminated by remote party", call_id, session_id);
        // Add any cleanup logic, logging, or monitoring here
    }
    
    /// Notification that a call was ended by the server
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String) {
        info!("📞 ServerManager notified: Call {} (session {}) ended by server", call_id, session_id);
        // Add any cleanup logic, logging, or monitoring here
    }
}

impl ServerManager {
    /// Create a new server manager
    pub fn new(
        session_manager: Arc<SessionManager>, 
        transaction_manager: Arc<TransactionManager>,
        config: ServerConfig
    ) -> Self {
        Self {
            session_manager,
            transaction_manager,
            config,
        }
    }
    
    /// **NEW**: Setup the notification system - connects SessionManager to ServerManager
    pub async fn setup_notification_system(&self) -> Result<()> {
        // **FIXED**: Now we can set the notifier properly with interior mutability
        self.session_manager.set_incoming_call_notifier(Arc::new(self.clone())).await;
        
        info!("✅ ServerManager notification system setup complete");
        Ok(())
    }
    
    /// Handle transaction events - simplified to only delegate
    pub async fn handle_transaction_event(&self, event: TransactionEvent) -> Result<()> {
        debug!("ServerManager delegating transaction event to SessionManager: {:?}", event);
        
        // **ARCHITECTURAL FIX**: Delegate everything to SessionManager
        self.session_manager.handle_transaction_event(event).await
            .map_err(|e| anyhow::anyhow!("SessionManager failed to handle transaction event: {}", e))?;
        
        Ok(())
    }
    
    /// Handle incoming transport events (legacy compatibility)
    pub async fn handle_transport_event(&self, event: SessionTransportEvent) -> Result<()> {
        match event {
            SessionTransportEvent::TransportError { error, source } => {
                warn!("Transport error from {:?}: {}", source, error);
            },
            SessionTransportEvent::ConnectionEstablished { local_addr, remote_addr, transport } => {
                info!("Connection established: {} -> {:?} ({})", local_addr, remote_addr, transport);
            },
            SessionTransportEvent::ConnectionClosed { local_addr, remote_addr, transport } => {
                info!("Connection closed: {} -> {:?} ({})", local_addr, remote_addr, transport);
            },
            _ => {
                debug!("Transport event handled by transaction-core");
            }
        }
        Ok(())
    }
    
    /// **NEW**: Server policy method - determine if call should be accepted
    async fn should_accept_call(&self, event: &IncomingCallEvent) -> bool {
        // Example policy logic - customize based on server requirements
        
        // Check server capacity
        let active_sessions = self.get_active_sessions().await;
        if active_sessions.len() >= self.config.max_sessions {
            warn!("Rejecting call from {} - server at capacity ({} sessions)", 
                  event.caller_info.from, active_sessions.len());
            return false;
        }
        
        // Check if caller is allowed (example: could check against whitelist/blacklist)
        if self.is_caller_blocked(&event.caller_info.from).await {
            warn!("Rejecting call from {} - caller is blocked", event.caller_info.from);
            return false;
        }
        
        // Check business hours (example policy)
        if !self.is_within_business_hours().await {
            info!("Rejecting call from {} - outside business hours", event.caller_info.from);
            return false;
        }
        
        // Default: accept the call
        info!("Accepting call from {} - passed all policy checks", event.caller_info.from);
        true
    }
    
    /// **NEW**: Example policy method - check if caller is blocked
    async fn is_caller_blocked(&self, _caller: &str) -> bool {
        // Example: implement blacklist checking
        // For now, always return false (no one is blocked)
        false
    }
    
    /// **NEW**: Example policy method - check business hours
    async fn is_within_business_hours(&self) -> bool {
        // Example: implement business hours checking
        // For now, always return true (24/7 service)
        true
    }
    
    /// **DELEGATION**: Accept an incoming call (policy decision + delegation)
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        info!("🎯 ServerManager policy decision: ACCEPT call for session {}", session_id);
        
        // Delegate implementation to SessionManager
        self.session_manager.accept_call(session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to accept call: {}", e))?;
        
        info!("✅ Call acceptance completed for session {}", session_id);
        Ok(())
    }
    
    /// **DELEGATION**: Reject an incoming call (policy decision + delegation)
    pub async fn reject_call(&self, session_id: &SessionId, status_code: StatusCode) -> Result<()> {
        info!("🎯 ServerManager policy decision: REJECT call for session {} with status {}", session_id, status_code);
        
        // Delegate implementation to SessionManager
        self.session_manager.reject_call(session_id, status_code).await
            .map_err(|e| anyhow::anyhow!("Failed to reject call: {}", e))?;
        
        info!("✅ Call rejection completed for session {}", session_id);
        Ok(())
    }
    
    /// **DELEGATION**: End an active call (policy decision + delegation)
    pub async fn end_call(&self, session_id: &SessionId) -> Result<()> {
        info!("🎯 ServerManager policy decision: END call for session {}", session_id);
        
        // Delegate implementation to SessionManager
        self.session_manager.terminate_call(session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to end call: {}", e))?;
        
        info!("✅ Call termination completed for session {}", session_id);
        Ok(())
    }
    
    /// **DELEGATION**: Get all active sessions (delegates to SessionManager)
    pub async fn get_active_sessions(&self) -> Vec<SessionId> {
        // Delegate to SessionManager - it tracks sessions internally now
        self.session_manager.list_sessions()
            .iter()
            .map(|session| session.id.clone())
            .collect()
    }
    
    /// **DELEGATION**: Get session by ID (delegates to SessionManager)  
    pub async fn get_session(&self, session_id: &SessionId) -> Option<Arc<Session>> {
        // Delegate to SessionManager
        self.session_manager.get_session(session_id).ok()
    }
    
    /// Get server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
    
    /// **DELEGATION**: Hold/pause a call (policy decision + delegation)
    pub async fn hold_call(&self, session_id: &SessionId) -> Result<()> {
        info!("🎯 ServerManager policy decision: HOLD call for session {}", session_id);
        
        // For now, delegate to session's media management
        let session = self.get_session(session_id).await
            .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?;
        
        session.pause_media().await
            .map_err(|e| anyhow::anyhow!("Failed to hold call: {}", e))?;
        
        info!("✅ Call hold completed for session {}", session_id);
        Ok(())
    }
    
    /// **DELEGATION**: Resume a held call (policy decision + delegation)
    pub async fn resume_call(&self, session_id: &SessionId) -> Result<()> {
        info!("🎯 ServerManager policy decision: RESUME call for session {}", session_id);
        
        // For now, delegate to session's media management
        let session = self.get_session(session_id).await
            .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?;
        
        session.resume_media().await
            .map_err(|e| anyhow::anyhow!("Failed to resume call: {}", e))?;
        
        info!("✅ Call resume completed for session {}", session_id);
        Ok(())
    }
    
    /// Start the server manager (setup notification system)
    pub async fn start(&self) -> Result<()> {
        info!("Starting ServerManager with notification system");
        
        // Setup the notification system to connect SessionManager callbacks
        self.setup_notification_system().await?;
        
        info!("✅ ServerManager started with policy decision system");
        Ok(())
    }
} 