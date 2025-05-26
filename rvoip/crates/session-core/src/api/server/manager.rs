//! Server Manager
//!
//! This module provides high-level server operations for handling incoming calls,
//! managing sessions, and coordinating with the transport layer.

use std::sync::Arc;
use std::collections::HashMap;
use anyhow::{Result, Context};
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};

use rvoip_sip_core::{Request, Response, StatusCode, Method};
use crate::api::server::config::ServerConfig;
use crate::session::{SessionManager, Session};
use crate::transport::SessionTransportEvent;
use crate::{SessionId, Error};

/// High-level server manager for handling SIP server operations
pub struct ServerManager {
    /// Core session manager
    session_manager: Arc<SessionManager>,
    
    /// Server configuration
    config: ServerConfig,
    
    /// Pending incoming calls (Call-ID -> SessionId)
    pending_calls: Arc<RwLock<HashMap<String, SessionId>>>,
    
    /// Active sessions (SessionId -> Session)
    active_sessions: Arc<RwLock<HashMap<SessionId, Arc<Session>>>>,
}

impl ServerManager {
    /// Create a new server manager
    pub fn new(session_manager: Arc<SessionManager>, config: ServerConfig) -> Self {
        Self {
            session_manager,
            config,
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Handle incoming transport events
    pub async fn handle_transport_event(&self, event: SessionTransportEvent) -> Result<()> {
        match event {
            SessionTransportEvent::IncomingRequest { request, source, transport } => {
                self.handle_incoming_request(request, source, transport).await?;
            },
            SessionTransportEvent::IncomingResponse { response, source, transport } => {
                self.handle_incoming_response(response, source, transport).await?;
            },
            SessionTransportEvent::TransportError { error, source } => {
                warn!("Transport error from {:?}: {}", source, error);
            },
            SessionTransportEvent::ConnectionEstablished { local_addr, remote_addr, transport } => {
                info!("Connection established: {} -> {:?} ({})", local_addr, remote_addr, transport);
            },
            SessionTransportEvent::ConnectionClosed { local_addr, remote_addr, transport } => {
                info!("Connection closed: {} -> {:?} ({})", local_addr, remote_addr, transport);
            },
        }
        Ok(())
    }
    
    /// Handle incoming SIP request
    async fn handle_incoming_request(&self, request: Request, source: std::net::SocketAddr, transport: String) -> Result<()> {
        debug!("Handling {} request from {} via {}", request.method(), source, transport);
        
        match request.method() {
            Method::Invite => {
                self.handle_invite_request(request, source).await?;
            },
            Method::Bye => {
                self.handle_bye_request(request, source).await?;
            },
            Method::Ack => {
                self.handle_ack_request(request, source).await?;
            },
            _ => {
                info!("Received {} request from {} - not handled yet", request.method(), source);
            }
        }
        
        Ok(())
    }
    
    /// Handle incoming SIP response
    async fn handle_incoming_response(&self, response: Response, source: std::net::SocketAddr, transport: String) -> Result<()> {
        debug!("Handling {} response from {} via {}", response.status_code(), source, transport);
        // Response handling would be implemented here
        Ok(())
    }
    
    /// Handle incoming INVITE request
    async fn handle_invite_request(&self, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Handling INVITE from {}", source);
        
        // Extract Call-ID using the correct API
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("INVITE missing Call-ID header"))?
            .value();
        
        // Create incoming session
        let session = self.session_manager.create_incoming_session().await
            .context("Failed to create incoming session")?;
        
        let session_id = session.id.clone(); // Clone the session ID
        
        // Set session to Ringing state for incoming calls
        session.set_state(crate::session::session_types::SessionState::Ringing).await
            .context("Failed to set session to ringing state")?;
        
        // Store the pending call
        {
            let mut pending = self.pending_calls.write().await;
            pending.insert(call_id.clone(), session_id.clone());
        }
        
        // Store the active session
        {
            let mut active = self.active_sessions.write().await;
            active.insert(session_id.clone(), session);
        }
        
        info!("Created session {} for incoming INVITE with Call-ID {} (state: Ringing)", session_id, call_id);
        Ok(())
    }
    
    /// Handle incoming BYE request
    async fn handle_bye_request(&self, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Handling BYE from {}", source);
        
        // Extract Call-ID using the correct API
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("BYE missing Call-ID header"))?
            .value();
        
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).cloned()
        };
        
        if let Some(session_id) = session_id {
            self.end_call(&session_id).await?;
            info!("Ended call for session {} (Call-ID: {})", session_id, call_id);
        } else {
            warn!("Received BYE for unknown Call-ID: {}", call_id);
        }
        
        Ok(())
    }
    
    /// Handle incoming ACK request
    async fn handle_ack_request(&self, request: Request, source: std::net::SocketAddr) -> Result<()> {
        debug!("Handling ACK from {}", source);
        // ACK handling would be implemented here
        Ok(())
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Accepting call for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        // Check current state
        let current_state = session.state().await;
        info!("Session {} current state: {}", session_id, current_state);
        
        // Validate that we can accept from the current state
        if current_state != crate::session::session_types::SessionState::Ringing {
            return Err(anyhow::anyhow!(
                "Cannot accept call in state {}. Session must be in Ringing state to accept.",
                current_state
            ));
        }
        
        // Set session to connected state
        session.set_state(crate::session::session_types::SessionState::Connected).await
            .context("Failed to set session state to connected")?;
        
        info!("Call accepted for session {} (state: Connected)", session_id);
        Ok(())
    }
    
    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &SessionId, status_code: StatusCode) -> Result<()> {
        info!("Rejecting call for session {} with status {}", session_id, status_code);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        // Set session to terminated state
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        info!("Call rejected for session {}", session_id);
        Ok(())
    }
    
    /// End an active call
    pub async fn end_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Ending call for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
        };
        
        let session = match session {
            Some(session) => session,
            None => {
                warn!("Session {} not found in active sessions (may have been already removed)", session_id);
                return Ok(()); // Consider this a success since the session is already gone
            }
        };
        
        // Check current state
        let current_state = session.state().await;
        info!("Session {} current state before ending: {}", session_id, current_state);
        
        // Stop media and set terminated state
        if let Err(e) = session.stop_media().await {
            warn!("Failed to stop media for session {}: {}", session_id, e);
            // Continue with termination even if media stop fails
        }
        
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        info!("Call ended for session {} (state: Terminated)", session_id);
        Ok(())
    }
    
    /// Get all active sessions
    pub async fn get_active_sessions(&self) -> Vec<SessionId> {
        let active = self.active_sessions.read().await;
        active.keys().cloned().collect()
    }
    
    /// Get session by ID
    pub async fn get_session(&self, session_id: &SessionId) -> Option<Arc<Session>> {
        let active = self.active_sessions.read().await;
        active.get(session_id).cloned()
    }
    
    /// Get server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
} 