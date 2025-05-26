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

use rvoip_sip_core::{Request, Response, StatusCode, Method, Message};
use rvoip_transaction_core::{TransactionManager, TransactionEvent, TransactionKey};
use crate::api::server::config::ServerConfig;
use crate::session::{SessionManager, Session};
use crate::transport::{SessionTransportEvent, TransportIntegration};
use crate::{SessionId, Error};

/// High-level server manager for coordinating SIP sessions
/// 
/// **CRITICAL**: This manager is a COORDINATOR, not a SIP protocol handler.
/// It reacts to transaction events and coordinates session/media state.
/// All SIP responses are sent by transaction-core automatically.
pub struct ServerManager {
    /// Core session manager
    session_manager: Arc<SessionManager>,
    
    /// Transaction manager for SIP protocol handling
    transaction_manager: Arc<TransactionManager>,
    
    /// Server configuration
    config: ServerConfig,
    
    /// Pending incoming calls (Call-ID -> (SessionId, TransactionKey, Request))
    pending_calls: Arc<RwLock<HashMap<String, (SessionId, TransactionKey, Request)>>>,
    
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
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: We REACT to transaction events, we don't send responses.
    /// transaction-core handles all SIP protocol details automatically.
    pub async fn handle_transaction_event(&self, event: TransactionEvent) -> Result<()> {
        match event {
            TransactionEvent::InviteRequest { transaction_id, request, source } => {
                self.handle_invite_received(transaction_id, request, source).await?;
            },
            TransactionEvent::NonInviteRequest { transaction_id, request, source } => {
                self.handle_non_invite_received(transaction_id, request, source).await?;
            },
            TransactionEvent::NewRequest { transaction_id, request, source } => {
                match request.method() {
                    Method::Invite => {
                        info!("📞 Received INVITE request - coordinating session creation");
                        self.handle_invite_received(transaction_id, request, source).await?;
                    },
                    Method::Bye => {
                        self.handle_bye_received(transaction_id, request).await?;
                    },
                    Method::Ack => {
                        self.handle_ack_received(request).await?;
                    },
                    _ => {
                        debug!("Received {} request - handled by transaction-core", request.method());
                    }
                }
            },
            TransactionEvent::Response { transaction_id, response, .. } => {
                self.handle_response_sent(transaction_id, response).await?;
            },
            TransactionEvent::TransactionTerminated { transaction_id, .. } => {
                self.handle_transaction_completed(transaction_id).await?;
            },
            _ => {
                debug!("Received transaction event: {:?}", event);
            }
        }
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
    
    /// Handle INVITE received (coordinate session creation)
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: We create sessions and coordinate state.
    /// transaction-core automatically sends 180 Ringing and manages SIP protocol.
    async fn handle_invite_received(&self, transaction_id: TransactionKey, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Coordinating session creation for INVITE transaction {}", transaction_id);
        
        // Extract Call-ID
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("INVITE missing Call-ID header"))?
            .value();
        
        // Create incoming session (session-core responsibility)
        let session = self.session_manager.create_incoming_session().await
            .context("Failed to create incoming session")?;
        
        let session_id = session.id.clone();
        
        // Set session to Ringing state (session-core responsibility)
        session.set_state(crate::session::session_types::SessionState::Ringing).await
            .context("Failed to set session to ringing state")?;
        
        // Store the session mapping with transaction ID
        {
            let mut pending = self.pending_calls.write().await;
            pending.insert(call_id.clone(), (session_id.clone(), transaction_id.clone(), request.clone()));
        }
        
        {
            let mut active = self.active_sessions.write().await;
            active.insert(session_id.clone(), session);
        }
        
        info!("✅ Created session {} for INVITE transaction {} with Call-ID {} (transaction-core handles 180 Ringing)", 
              session_id, transaction_id, call_id);
        Ok(())
    }
    
    /// Handle non-INVITE received (coordinate appropriate response)
    async fn handle_non_invite_received(&self, transaction_id: TransactionKey, request: Request, source: std::net::SocketAddr) -> Result<()> {
        info!("Coordinating response for non-INVITE transaction {} (method: {})", transaction_id, request.method());
        
        // For non-INVITE methods, we just log - transaction-core handles responses automatically
        match request.method() {
            Method::Options => {
                debug!("OPTIONS request - transaction-core will send capabilities response");
            },
            Method::Info => {
                debug!("INFO request - transaction-core will send 200 OK");
            },
            Method::Message => {
                debug!("MESSAGE request - transaction-core will send 200 OK");
            },
            _ => {
                debug!("Non-INVITE method {} - transaction-core handles automatically", request.method());
            }
        }
        
        Ok(())
    }
    
    /// Handle BYE received (coordinate session termination)
    async fn handle_bye_received(&self, transaction_id: TransactionKey, request: Request) -> Result<()> {
        info!("Coordinating session termination for BYE transaction {}", transaction_id);
        
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("BYE missing Call-ID header"))?
            .value();
        
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).cloned().map(|(session_id, _, _)| session_id)
        };
        
        // Coordinate session termination (session-core responsibility)
        if let Some(session_id) = session_id {
            self.end_call(&session_id).await?;
            info!("✅ Coordinated call termination for session {} (Call-ID: {}) - transaction-core sends 200 OK", session_id, call_id);
        } else {
            warn!("Received BYE for unknown Call-ID: {} - transaction-core will still send 200 OK", call_id);
        }
        
        Ok(())
    }
    
    /// Handle ACK received (coordinate session confirmation)
    async fn handle_ack_received(&self, request: Request) -> Result<()> {
        info!("Coordinating session confirmation for ACK");
        
        let call_id = request.call_id()
            .ok_or_else(|| anyhow::anyhow!("ACK missing Call-ID header"))?
            .value();
        
        let session_id = {
            let pending = self.pending_calls.read().await;
            pending.get(&call_id).cloned().map(|(session_id, _, _)| session_id)
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
    
    /// Handle response sent by transaction-core (coordinate session state)
    async fn handle_response_sent(&self, transaction_id: TransactionKey, response: Response) -> Result<()> {
        debug!("Response sent by transaction-core: {} for transaction {}", response.status_code(), transaction_id);
        
        // Coordinate session state based on response sent by transaction-core
        let status_code = response.status_code();
        match status_code {
            180 => {
                debug!("180 Ringing sent by transaction-core - session remains in Ringing state");
            },
            200 => {
                debug!("200 OK sent by transaction-core - session should be in Connected state");
            },
            status if status >= 400 && status < 600 => {
                debug!("Error response {} sent by transaction-core - session should be terminated", status);
            },
            _ => {
                debug!("Response {} sent by transaction-core", status_code);
            }
        }
        
        Ok(())
    }
    
    /// Handle transaction completed (coordinate cleanup)
    async fn handle_transaction_completed(&self, transaction_id: TransactionKey) -> Result<()> {
        debug!("Transaction {} completed - coordinating cleanup", transaction_id);
        
        // Find session associated with this transaction and clean up if needed
        let session_to_cleanup = {
            let pending = self.pending_calls.read().await;
            pending.iter()
                .find(|(_, (_, tid, _))| *tid == transaction_id)
                .map(|(call_id, (session_id, _, _))| (call_id.clone(), session_id.clone()))
        };
        
        if let Some((call_id, session_id)) = session_to_cleanup {
            debug!("Cleaning up completed transaction for session {} (Call-ID: {})", session_id, call_id);
            
            // Remove from pending calls
            {
                let mut pending = self.pending_calls.write().await;
                pending.remove(&call_id);
            }
        }
        
        Ok(())
    }
    
    /// End an active call (coordinate session and media termination)
    pub async fn end_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Coordinating call termination for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
        };
        
        let session = match session {
            Some(session) => session,
            None => {
                warn!("Session {} not found in active sessions (may have been already removed)", session_id);
                return Ok(());
            }
        };
        
        let current_state = session.state().await;
        info!("Session {} current state before ending: {}", session_id, current_state);
        
        // **AUTOMATIC MEDIA COORDINATION** (session-core responsibility)
        info!("🎵 Coordinating media cleanup for ended call...");
        
        if let Err(e) = session.stop_media().await {
            warn!("Failed to stop media for session {}: {}", session_id, e);
        } else {
            info!("✅ Media automatically cleaned up for session {}", session_id);
        }
        
        session.set_media_session_id(None).await;
        
        // Set session to terminated state (session-core responsibility)
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        info!("✅ Call termination coordinated for session {} (state: Terminated, media: cleaned up)", session_id);
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
    
    /// Hold/pause a call (coordinate media pause)
    pub async fn hold_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Coordinating call hold for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        let current_state = session.state().await;
        info!("Session {} current state before hold: {}", session_id, current_state);
        
        if current_state != crate::session::session_types::SessionState::Connected {
            return Err(anyhow::anyhow!(
                "Cannot hold call in state {}. Call must be Connected to hold.",
                current_state
            ));
        }
        
        // **AUTOMATIC MEDIA COORDINATION** (session-core responsibility)
        info!("🎵 Coordinating media pause for held call...");
        
        if let Err(e) = session.pause_media().await {
            warn!("Failed to pause media for session {}: {}", session_id, e);
        } else {
            info!("✅ Media automatically paused for session {}", session_id);
        }
        
        info!("✅ Call hold coordinated for session {} (media: paused)", session_id);
        Ok(())
    }
    
    /// Resume a held call (coordinate media resume)
    pub async fn resume_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Coordinating call resume for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        let current_state = session.state().await;
        info!("Session {} current state before resume: {}", session_id, current_state);
        
        // **AUTOMATIC MEDIA COORDINATION** (session-core responsibility)
        info!("🎵 Coordinating media resume for resumed call...");
        
        if let Err(e) = session.resume_media().await {
            warn!("Failed to resume media for session {}: {}", session_id, e);
        } else {
            info!("✅ Media automatically resumed for session {}", session_id);
        }
        
        info!("✅ Call resume coordinated for session {} (media: active)", session_id);
        Ok(())
    }
    
    /// Accept an incoming call (coordinate session acceptance and media setup)
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: We coordinate session state and media.
    /// transaction-core automatically sends 200 OK response when we signal acceptance.
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        info!("Coordinating call acceptance for session {}", session_id);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        let current_state = session.state().await;
        info!("Session {} current state: {}", session_id, current_state);
        
        if current_state != crate::session::session_types::SessionState::Ringing {
            return Err(anyhow::anyhow!(
                "Cannot accept call in state {}. Call must be Ringing to accept.",
                current_state
            ));
        }
        
        // **AUTOMATIC MEDIA COORDINATION** (session-core responsibility)
        info!("🎵 Coordinating media setup for accepted call...");
        
        if let Err(e) = session.set_media_negotiating().await {
            warn!("Failed to set media negotiating for session {}: {}", session_id, e);
        }
        
        if let Err(e) = session.start_media().await {
            warn!("Failed to start media for session {}: {}", session_id, e);
        } else {
            info!("✅ Media automatically set up for session {}", session_id);
        }
        
        // Set session to connected state (session-core responsibility)
        session.set_state(crate::session::session_types::SessionState::Connected).await
            .context("Failed to set session state to connected")?;
        
        // **TRANSACTION-CORE INTEGRATION**: Signal acceptance to transaction-core
        // transaction-core will automatically send 200 OK response with proper SDP
        if let Err(e) = self.signal_call_acceptance(session_id).await {
            warn!("Failed to signal call acceptance to transaction-core: {}", e);
        } else {
            info!("📞 Signaled call acceptance to transaction-core for session {}", session_id);
        }
        
        info!("✅ Call acceptance coordinated for session {} (state: Connected, media: active)", session_id);
        Ok(())
    }
    
    /// Reject an incoming call (coordinate session rejection)
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: We coordinate session state.
    /// transaction-core automatically sends error response when we signal rejection.
    pub async fn reject_call(&self, session_id: &SessionId, status_code: StatusCode) -> Result<()> {
        info!("Coordinating call rejection for session {} with status {}", session_id, status_code);
        
        let session = {
            let active = self.active_sessions.read().await;
            active.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("Session {} not found", session_id))?
        };
        
        // Set session to terminated state (session-core responsibility)
        session.set_state(crate::session::session_types::SessionState::Terminated).await
            .context("Failed to set session state to terminated")?;
        
        // **TRANSACTION-CORE INTEGRATION**: Signal rejection to transaction-core
        // transaction-core will automatically send error response
        if let Err(e) = self.signal_call_rejection(session_id, status_code).await {
            warn!("Failed to signal call rejection to transaction-core: {}", e);
        } else {
            info!("📞 Signaled call rejection to transaction-core for session {}", session_id);
        }
        
        // Remove from active sessions
        {
            let mut active = self.active_sessions.write().await;
            active.remove(session_id);
        }
        
        // Remove from pending calls
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _, _)| sid != session_id);
        }
        
        info!("✅ Call rejection coordinated for session {}", session_id);
        Ok(())
    }
    
    /// Signal call acceptance to transaction-core (coordination interface)
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: session-core coordinates, transaction-core handles SIP protocol.
    /// We signal acceptance and let transaction-core create the proper SDP response using sip-core.
    async fn signal_call_acceptance(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Replace with proper event-based coordination
        // For now, find transaction and send response (temporary approach)
        let (transaction_id, request) = self.find_transaction_and_request_for_session(session_id).await?;
        
        // **ARCHITECTURAL FIX**: Use sip-core's SdpBuilder instead of manual SDP creation
        let sdp_session = rvoip_sip_core::sdp::SdpBuilder::new("Session")
            .origin("-", &chrono::Utc::now().timestamp().to_string(), "1", "IN", "IP4", &self.config.bind_address.ip().to_string())
            .connection("IN", "IP4", &self.config.bind_address.ip().to_string())
            .time("0", "0")
            .media_audio(10000, "RTP/AVP")
                .formats(&["0"])
                .rtpmap("0", "PCMU/8000")
                .direction(rvoip_sip_core::sdp::attributes::MediaDirection::SendRecv)
                .done()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build SDP: {}", e))?;
        
        // Convert SDP session to bytes
        let sdp_bytes = bytes::Bytes::from(sdp_session.to_string());
        
        let response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
            &request,
            StatusCode::Ok,
            Some("OK"),
        ).build().with_body(sdp_bytes);
        
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| anyhow::anyhow!("Failed to signal acceptance to transaction-core: {}", e))?;
        
        Ok(())
    }
    
    /// Signal call rejection to transaction-core (coordination interface)
    async fn signal_call_rejection(&self, session_id: &SessionId, status_code: StatusCode) -> Result<()> {
        // TODO: Replace with proper event-based coordination
        let transaction_id = self.find_transaction_for_session(session_id).await?;
        
        let response = Response::new(status_code);
        
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| anyhow::anyhow!("Failed to signal rejection to transaction-core: {}", e))?;
        
        Ok(())
    }
    
    /// Find transaction ID and original request for a session
    async fn find_transaction_and_request_for_session(&self, session_id: &SessionId) -> Result<(TransactionKey, Request)> {
        let pending = self.pending_calls.read().await;
        for (_, (sid, transaction_id, request)) in pending.iter() {
            if sid == session_id {
                return Ok((transaction_id.clone(), request.clone()));
            }
        }
        Err(anyhow::anyhow!("Transaction not found for session {}", session_id))
    }
    
    /// Find transaction ID for a session
    async fn find_transaction_for_session(&self, session_id: &SessionId) -> Result<TransactionKey> {
        let pending = self.pending_calls.read().await;
        for (_, (sid, transaction_id, _)) in pending.iter() {
            if sid == session_id {
                return Ok(transaction_id.clone());
            }
        }
        Err(anyhow::anyhow!("Transaction not found for session {}", session_id))
    }
} 