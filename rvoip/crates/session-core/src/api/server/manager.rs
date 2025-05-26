//! Server Manager
//!
//! This module provides high-level server operations for handling incoming calls,
//! managing sessions, and coordinating with the transport layer via transaction-core.

use std::sync::Arc;
use std::collections::HashMap;
use anyhow::{Result, Context};
use tokio::sync::RwLock;
use tracing::{info, debug, warn, error};
use uuid::Uuid;

use rvoip_sip_core::{Request, Response, StatusCode, Method, Message};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
use crate::api::server::config::ServerConfig;
use crate::session::{SessionManager, Session};
use crate::transport::{SessionTransportEvent, TransportIntegration};
use crate::{SessionId, Error};

/// High-level server manager for handling SIP server operations
pub struct ServerManager {
    /// Core session manager
    session_manager: Arc<SessionManager>,
    
    /// Transaction manager for SIP protocol handling
    transaction_manager: Arc<TransactionManager>,
    
    /// Server configuration
    config: ServerConfig,
    
    /// Pending incoming calls (Call-ID -> (SessionId, TransactionKey))
    pending_calls: Arc<RwLock<HashMap<String, (SessionId, TransactionKey)>>>,
    
    /// Active sessions (SessionId -> Session)
    active_sessions: Arc<RwLock<HashMap<SessionId, Arc<Session>>>>,
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
            pending_calls: Arc::new(RwLock::new(HashMap::new())),
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Handle transaction events from transaction-core
    pub async fn handle_transaction_event(&self, event: TransactionEvent) -> Result<()> {
        match event {
            TransactionEvent::InviteRequest { transaction_id, request, source } => {
                self.handle_invite_transaction(transaction_id, request, source).await?;
            },
            TransactionEvent::NonInviteRequest { transaction_id, request, source } => {
                self.handle_non_invite_transaction(transaction_id, request, source).await?;
            },
            TransactionEvent::NewRequest { transaction_id, request, source } => {
                // Handle other new requests
                match request.method() {
                    Method::Bye => {
                        self.handle_bye_transaction(transaction_id, request).await?;
                    },
                    Method::Ack => {
                        self.handle_ack_transaction(request).await?;
                    },
                    _ => {
                        info!("Received {} request - handled by transaction-core", request.method());
                    }
                }
            },
            _ => {
                // Other transaction events (responses, timeouts, etc.) are handled by transaction-core
                debug!("Received transaction event: {:?}", event);
            }
        }
        Ok(())
    }
    
    /// Handle incoming transport events (legacy compatibility)
    pub async fn handle_transport_event(&self, event: SessionTransportEvent) -> Result<()> {
        // This is now mainly for logging - transaction-core handles the actual SIP protocol
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
                // Requests and responses are now handled through transaction events
                debug!("Transport event handled by transaction-core");
            }
        }
        Ok(())
    }
    
    /// Handle INVITE transaction (transaction-core already sent 180 Ringing)
    async fn handle_invite_transaction(&self, transaction_id: TransactionKey, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Creating session for INVITE transaction {}", transaction_id);
        
        // Extract Call-ID
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("INVITE missing Call-ID header"))?
            .value();
        
        // Create incoming session
        let session = self.session_manager.create_incoming_session().await
            .context("Failed to create incoming session")?;
        
        let session_id = session.id.clone();
        
        // Set session to Ringing state
        session.set_state(crate::session::session_types::SessionState::Ringing).await
            .context("Failed to set session to ringing state")?;
        
        // Store the session mapping with transaction ID
        {
            let mut pending = self.pending_calls.write().await;
            pending.insert(call_id.clone(), (session_id.clone(), transaction_id.clone()));
        }
        
        {
            let mut active = self.active_sessions.write().await;
            active.insert(session_id.clone(), session);
        }
        
        info!("Created session {} for INVITE transaction {} with Call-ID {} (transaction-core handles SIP responses)", 
              session_id, transaction_id, call_id);
        Ok(())
    }
    
    /// Handle non-INVITE transaction
    async fn handle_non_invite_transaction(&self, transaction_id: TransactionKey, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Handling non-INVITE transaction {} for method {}", transaction_id, request.method());
        
        match request.method() {
            Method::Options | Method::Info | Method::Message => {
                // Send 200 OK response for these methods
                let response = Response::new(StatusCode::Ok);
                if let Err(e) = self.transaction_manager.send_response(&transaction_id, response).await {
                    warn!("Failed to send 200 OK response for {}: {}", request.method(), e);
                }
            },
            _ => {
                info!("Non-INVITE method {} handled by transaction-core", request.method());
            }
        }
        
        Ok(())
    }
    
    /// Handle BYE transaction
    async fn handle_bye_transaction(&self, transaction_id: TransactionKey, request: Request) -> Result<()> {
        info!("Handling BYE transaction {}", transaction_id);
        
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("BYE missing Call-ID header"))?
            .value();
        
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).cloned().map(|(session_id, _)| session_id)
        };
        
        // Send 200 OK response to BYE via transaction-core
        let response = Response::new(StatusCode::Ok);
        if let Err(e) = self.transaction_manager.send_response(&transaction_id, response).await {
            warn!("Failed to send 200 OK response to BYE: {}", e);
        }
        
        if let Some(session_id) = session_id {
            self.end_call(&session_id).await?;
            info!("Ended call for session {} (Call-ID: {})", session_id, call_id);
        } else {
            warn!("Received BYE for unknown Call-ID: {}", call_id);
        }
        
        Ok(())
    }
    
    /// Handle ACK transaction
    async fn handle_ack_transaction(&self, request: Request) -> Result<()> {
        info!("Handling ACK for session confirmation");
        
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("ACK missing Call-ID header"))?
            .value();
        
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).cloned().map(|(session_id, _)| session_id)
        };
        
        if let Some(session_id) = session_id {
            let session = {
                let active = self.active_sessions.read().await;
                active.get(&session_id).cloned()
            };
            
            if let Some(session) = session {
                let current_state = session.state().await;
                info!("ACK received for session {} (current state: {})", session_id, current_state);
                
                if current_state == crate::session::session_types::SessionState::Connected {
                    info!("✅ INVITE transaction completed with ACK for session {}", session_id);
                    
                    // Remove from pending calls since transaction is complete
                    {
                        let mut pending = self.pending_calls.write().await;
                        pending.remove(&call_id);
                    }
                    
                    info!("📞 Call fully established for session {} (Call-ID: {})", session_id, call_id);
                }
            }
        }
        
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
        
        // **PHASE 2: AUTOMATIC MEDIA COORDINATION**
        // 1. Automatically clean up media when ending the call
        info!("🎵 Cleaning up media automatically for ended call...");
        
        // Stop media and clean up resources
        if let Err(e) = session.stop_media().await {
            warn!("Failed to stop media for session {}: {}", session_id, e);
            // Continue with termination even if media stop fails
        } else {
            info!("✅ Media automatically cleaned up for session {}", session_id);
        }
        
        // Clear media session references
        session.set_media_session_id(None).await;
        
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        info!("Call ended for session {} (state: Terminated, media: cleaned up)", session_id);
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
    
    /// Hold/pause a call
    pub async fn hold_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Holding call for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        // Check current state
        let current_state = session.state().await;
        info!("Session {} current state before hold: {}", session_id, current_state);
        
        // Validate that we can hold from the current state
        if current_state != crate::session::session_types::SessionState::Connected {
            return Err(anyhow::anyhow!(
                "Cannot hold call in state {}. Call must be Connected to hold.",
                current_state
            ));
        }
        
        // **PHASE 2: AUTOMATIC MEDIA COORDINATION**
        // 2. Automatically pause media when holding the call
        info!("🎵 Pausing media automatically for held call...");
        
        // Pause media for the session
        if let Err(e) = session.pause_media().await {
            warn!("Failed to pause media for session {}: {}", session_id, e);
            // Continue with hold even if media pause fails
        } else {
            info!("✅ Media automatically paused for session {}", session_id);
        }
        
        // Set session to on-hold state (we'll use Paused as a hold state)
        // Note: In a full implementation, we might have a separate Hold state
        info!("Call held for session {} (media: paused)", session_id);
        Ok(())
    }
    
    /// Resume a held call
    pub async fn resume_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Resuming call for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        // Check current state
        let current_state = session.state().await;
        info!("Session {} current state before resume: {}", session_id, current_state);
        
        // **PHASE 2: AUTOMATIC MEDIA COORDINATION**
        // 3. Automatically resume media when resuming the call
        info!("🎵 Resuming media automatically for resumed call...");
        
        // Resume media for the session
        if let Err(e) = session.resume_media().await {
            warn!("Failed to resume media for session {}: {}", session_id, e);
            // Continue with resume even if media resume fails
        } else {
            info!("✅ Media automatically resumed for session {}", session_id);
        }
        
        info!("Call resumed for session {} (media: active)", session_id);
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
                "Cannot accept call in state {}. Call must be Ringing to accept.",
                current_state
            ));
        }
        
        // **PHASE 2: AUTOMATIC MEDIA COORDINATION**
        // Set up media automatically when accepting the call
        info!("🎵 Setting up media automatically for accepted call...");
        
        // Set media to negotiating state
        if let Err(e) = session.set_media_negotiating().await {
            warn!("Failed to set media negotiating for session {}: {}", session_id, e);
        }
        
        // Start media for the session (this will coordinate with MediaManager)
        if let Err(e) = session.start_media().await {
            warn!("Failed to start media for session {}: {}", session_id, e);
            // Continue with call acceptance even if media setup fails
        } else {
            info!("✅ Media automatically set up for session {}", session_id);
        }
        
        // **TRANSACTION-CORE INTEGRATION**
        // Send 200 OK response via transaction-core server transaction
        if let Err(e) = self.send_accept_response(session_id).await {
            warn!("Failed to send accept response via transaction-core: {}", e);
            // Continue with state change even if response fails
        } else {
            info!("📞 Sent 200 OK response via transaction-core for session {}", session_id);
        }
        
        // Set session to connected state
        session.set_state(crate::session::session_types::SessionState::Connected).await
            .context("Failed to set session state to connected")?;
        
        info!("Call accepted for session {} (state: Connected, media: active)", session_id);
        Ok(())
    }
    
    /// Send accept response via transaction-core
    async fn send_accept_response(&self, session_id: &SessionId) -> Result<()> {
        // Find the transaction for this session
        let transaction_id = self.find_transaction_for_session(session_id).await?;
        
        // Create a 200 OK response with SDP
        let mut response = Response::new(StatusCode::Ok);
        
        // Add basic SDP for media negotiation
        let sdp = self.create_basic_sdp()?;
        response = response.with_body(sdp);
        
        // Send response via transaction-core (it handles all SIP protocol details)
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| anyhow::anyhow!("Failed to send response via transaction-core: {}", e))?;
        
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
        
        // **TRANSACTION-CORE INTEGRATION**
        // Send error response via transaction-core
        if let Err(e) = self.send_reject_response(session_id, status_code).await {
            warn!("Failed to send reject response via transaction-core: {}", e);
            // Continue with state change even if response fails
        } else {
            info!("📞 Sent {} response via transaction-core for session {}", status_code, session_id);
        }
        
        // Set session to terminated state
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        // Remove from pending calls
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _)| sid != session_id);
        }
        
        info!("Call rejected for session {}", session_id);
        Ok(())
    }
    
    /// Send reject response via transaction-core
    async fn send_reject_response(&self, session_id: &SessionId, status_code: StatusCode) -> Result<()> {
        // Find the transaction for this session
        let transaction_id = self.find_transaction_for_session(session_id).await?;
        
        // Create error response
        let response = Response::new(status_code);
        
        // Send response via transaction-core (it handles all SIP protocol details)
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| anyhow::anyhow!("Failed to send reject response via transaction-core: {}", e))?;
        
        Ok(())
    }
    
    /// Find transaction ID for a session
    async fn find_transaction_for_session(&self, session_id: &SessionId) -> Result<TransactionKey> {
        let pending = self.pending_calls.read().await;
        for (_, (sid, transaction_id)) in pending.iter() {
            if sid == session_id {
                return Ok(transaction_id.clone());
            }
        }
        Err(anyhow::anyhow!("Transaction not found for session {}", session_id))
    }
    
    /// Create basic SDP for media negotiation
    fn create_basic_sdp(&self) -> Result<bytes::Bytes> {
        let sdp = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n",
            chrono::Utc::now().timestamp(),
            chrono::Utc::now().timestamp(),
            self.config.bind_address.ip(),
            self.config.bind_address.ip(),
            10000 // Default RTP port
        );
        
        Ok(bytes::Bytes::from(sdp))
    }
} 