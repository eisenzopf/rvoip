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
    /// **ARCHITECTURAL FIX**: Forward events to DialogManager first, then coordinate sessions.
    /// DialogManager handles SIP protocol details and dialog state management.
    pub async fn handle_transaction_event(&self, event: TransactionEvent) -> Result<()> {
        debug!("ServerManager received transaction event: {:?}", event);
        
        // **CRITICAL FIX**: Forward transaction events to DialogManager first
        // DialogManager handles SIP protocol details and dialog state
        debug!("Forwarding transaction event to DialogManager");
        self.session_manager.dialog_manager().process_transaction_event(event.clone()).await;
        debug!("DialogManager processing completed, continuing with session coordination");
        
        // Then handle session-level coordination based on the event
        match event {
            TransactionEvent::NewRequest { transaction_id, request, source } => {
                match request.method() {
                    Method::Invite => {
                        // Use RFC-compliant coordination approach
                        if let Err(e) = self.handle_invite_request(transaction_id, request, source).await {
                            error!("❌ Failed to handle INVITE request: {}", e);
                        }
                    },
                    Method::Bye => {
                        info!("📞 Received BYE request - coordinating session termination");
                        // Handle BYE request
                        // TODO: Implement BYE handling
                    },
                    Method::Ack => {
                        self.handle_ack_received(request).await?;
                    },
                    _ => {
                        debug!("📞 Received {} request - forwarding to DialogManager", request.method());
                        // Forward other methods to DialogManager
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
                debug!("Received transaction event: {:?} - forwarded to DialogManager", event);
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
    
    /// Handle incoming INVITE requests - RFC-compliant session coordination only
    /// 
    /// **ARCHITECTURAL PRINCIPLE**: session-core is a COORDINATOR, not a SIP protocol handler.
    /// transaction-core should handle all SIP protocol details per RFC 3261.
    /// We only coordinate session creation and state management.
    async fn handle_invite_request(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    ) -> Result<()> {
        info!("📞 Received INVITE request - coordinating session creation");
        
        // **RFC 3261 COMPLIANCE**: session-core is Transaction User (TU)
        // - transaction-core should auto-send 100 Trying within 200ms (Timer 100)
        // - We only make application-level decisions and coordinate state
        // - We do NOT send SIP responses directly (that's transaction-core's job)
        
        info!("Coordinating session creation for INVITE transaction {}", transaction_id);
        
        // Extract Call-ID for session tracking
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
        
        // **ARCHITECTURAL FIX**: Do NOT send SIP responses manually
        // transaction-core will automatically send 100 Trying via Timer 100
        // session-core only coordinates application state, not SIP protocol
        
        info!("✅ Created session {} for INVITE transaction {} with Call-ID {}", 
              session_id, transaction_id, call_id);
        info!("🎯 transaction-core will automatically send 100 Trying via Timer 100");
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
        
        // Look for session in both pending calls and active sessions
        let session_id = {
            // First check pending calls
            let pending = self.pending_calls.read().await;
            if let Some((session_id, _, _)) = pending.get(&call_id) {
                Some(session_id.clone())
            } else {
                drop(pending);
                // If not in pending, check active sessions
                // For now, just take the first active session as a fallback
                // TODO: Implement proper Call-ID to SessionId mapping
                let active = self.active_sessions.read().await;
                active.keys().next().cloned()
            }
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
    /// transaction-core sends 200 OK response when we call send_response.
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
            return Err(anyhow::anyhow!("Session {} is not in Ringing state (current: {})", session_id, current_state));
        }
        
        info!("🎵 Setting up media-core integration for accepted call...");
        
        // Get transaction info from pending_calls
        let (transaction_id, request) = {
            let pending = self.pending_calls.read().await;
            let mut found_entry = None;
            
            // Find the pending call for this session
            for (call_id, (sid, tx_id, req)) in pending.iter() {
                if sid == session_id {
                    found_entry = Some((tx_id.clone(), req.clone()));
                    break;
                }
            }
            
            found_entry.ok_or_else(|| anyhow::anyhow!("No pending call found for session {}", session_id))?
        };
        
        // Extract SDP offer from INVITE request
        if !request.body().is_empty() {
            let sdp_str = String::from_utf8_lossy(request.body());
            info!("📋 Processing SDP offer: {} bytes", request.body().len());
            debug!("SDP offer content:\\n{}", sdp_str);
            
            // Generate SDP answer using media-core integration
            let sdp_answer = self.build_sdp_answer(&sdp_str).await
                .context("Failed to generate SDP answer")?;
            
            info!("✅ Generated SDP answer: {} bytes", sdp_answer.len());
            debug!("SDP answer content:\\n{}", sdp_answer);
            
            // **RFC-COMPLIANT**: Use transaction-core's send_response API
            let mut ok_response = rvoip_sip_core::builder::SimpleResponseBuilder::response_from_request(
                &request,
                rvoip_sip_core::StatusCode::Ok,
                Some("OK")
            ).build();
            
            // Add SDP answer as body
            ok_response = ok_response.with_body(bytes::Bytes::from(sdp_answer));
            
            // Add Content-Type header
            let content_type = rvoip_sip_core::types::content_type::ContentType::from_type_subtype("application", "sdp");
            ok_response.headers.push(rvoip_sip_core::TypedHeader::ContentType(content_type));
            
            // Send 200 OK through transaction-core
            if let Err(e) = self.transaction_manager.send_response(&transaction_id, ok_response).await {
                error!("❌ Failed to send 200 OK response: {}", e);
                return Err(anyhow::anyhow!("Failed to send 200 OK response: {}", e));
            }
            
            info!("✅ Sent 200 OK with SDP answer for session {}", session_id);
        } else {
            return Err(anyhow::anyhow!("INVITE request missing SDP offer"));
        }
        
        // Update session state to Active
        session.set_state(crate::session::session_types::SessionState::Connected).await
            .map_err(|e| anyhow::anyhow!("Failed to transition session to active: {}", e))?;
        
        // Remove from pending calls
        {
            let mut pending = self.pending_calls.write().await;
            pending.retain(|_, (sid, _, _)| sid != session_id);
        }
        
        info!("✅ Call acceptance coordinated for session {}", session_id);
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
    
    /// Extract media configuration from SDP offer for media-core integration
    async fn extract_media_config_from_sdp(&self, sdp: &rvoip_sip_core::types::sdp::SdpSession) -> Result<crate::media::MediaConfig> {
        use crate::media::{MediaConfig, AudioCodecType, SessionMediaType, SessionMediaDirection};
        use std::net::SocketAddr;
        
        // Extract connection information
        let connection = sdp.connection_info.as_ref()
            .ok_or_else(|| anyhow::anyhow!("SDP missing connection information"))?;
        
        // Find audio media description
        let audio_media = sdp.media_descriptions.iter()
            .find(|m| m.media == "audio")
            .ok_or_else(|| anyhow::anyhow!("SDP missing audio media description"))?;
        
        // Extract remote RTP address and port
        let remote_ip = connection.connection_address.parse()
            .map_err(|e| anyhow::anyhow!("Invalid IP address in SDP: {}", e))?;
        let remote_port = audio_media.port;
        let remote_addr = SocketAddr::new(remote_ip, remote_port);
        
        // Determine preferred codec from SDP formats
        let preferred_codec = if audio_media.formats.contains(&"0".to_string()) {
            AudioCodecType::PCMU
        } else if audio_media.formats.contains(&"8".to_string()) {
            AudioCodecType::PCMA
        } else if audio_media.formats.contains(&"9".to_string()) {
            AudioCodecType::G722
        } else {
            // Default to PCMU if no recognized codec
            AudioCodecType::PCMU
        };
        
        // Allocate local RTP port (for now, use a simple allocation strategy)
        // TODO: Integrate with media-core for proper port allocation
        let local_port = 10000 + (rand::random::<u16>() % 10000); // Random port in range 10000-19999
        let local_addr = SocketAddr::new(self.config.bind_address.ip(), local_port);
        
        info!("📊 Extracted media config: remote={}:{}, local={}:{}, codec={:?}", 
              remote_ip, remote_port, local_addr.ip(), local_addr.port(), preferred_codec);
        
        Ok(MediaConfig {
            local_addr,
            remote_addr: Some(remote_addr),
            media_type: SessionMediaType::Audio,
            payload_type: preferred_codec.to_payload_type(),
            clock_rate: preferred_codec.clock_rate(),
            audio_codec: preferred_codec,
            direction: SessionMediaDirection::SendRecv,
        })
    }
    
    /// Generate SDP answer using media-core integration
    async fn build_sdp_answer(&self, offer_sdp: &str) -> Result<String> {
        info!("🎵 Generating SDP answer using media-core integration...");
        
        // Get media manager from session manager
        let media_manager = self.session_manager.media_manager();
        
        // Get supported codecs from media-core (returns Vec<u8>)
        let supported_codecs = media_manager.get_supported_codecs().await;
        info!("🎼 Media-core supported codecs: {:?}", supported_codecs);
        
        // Convert u8 payload types to String format for negotiation
        let supported_codec_strings: Vec<String> = supported_codecs.iter().map(|&pt| pt.to_string()).collect();
        
        // Parse the offer to extract remote capabilities
        // For now, use a simple approach - TODO: implement proper SDP parsing
        let remote_port = if offer_sdp.contains("m=audio") {
            // Extract port from "m=audio 6000 RTP/AVP 0" line
            offer_sdp.lines()
                .find(|line| line.starts_with("m=audio"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|port_str| port_str.parse::<u16>().ok())
                .unwrap_or(6000)
        } else {
            6000
        };
        
        // Allocate local RTP port (use a simple approach for now)
        let local_port = 10000 + (chrono::Utc::now().timestamp() % 10000) as u16;
        
        info!("🔌 Allocated local RTP port: {} (remote: {})", local_port, remote_port);
        
        // Negotiate codecs (find common codecs between offer and our capabilities)
        let negotiated_codecs = self.negotiate_codecs(offer_sdp, &supported_codec_strings).await?;
        info!("🤝 Negotiated codecs: {:?}", negotiated_codecs);
        
        // Generate SDP answer
        let session_id = chrono::Utc::now().timestamp();
        let session_version = 1;
        
        let sdp_answer = format!(
            "v=0\r\n\
             o=server {} {} IN IP4 127.0.0.1\r\n\
             s=Media Session\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP {}\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            session_id,
            session_version,
            local_port,
            negotiated_codecs.join(" ")
        );
        
        info!("✅ Generated SDP answer with media-core integration");
        debug!("SDP answer content:\\n{}", sdp_answer);
        
        Ok(sdp_answer)
    }
    
    /// Negotiate codecs between SDP offer and supported codecs
    async fn negotiate_codecs(&self, offer_sdp: &str, supported_codecs: &[String]) -> Result<Vec<String>> {
        // Extract codecs from SDP offer
        let mut offered_codecs = Vec::new();
        
        // Look for "m=audio" line to get format list
        if let Some(audio_line) = offer_sdp.lines().find(|line| line.starts_with("m=audio")) {
            let parts: Vec<&str> = audio_line.split_whitespace().collect();
            if parts.len() > 3 {
                // Skip "m=audio", port, "RTP/AVP", then collect format numbers
                for format in &parts[3..] {
                    offered_codecs.push(format.to_string());
                }
            }
        }
        
        info!("📋 Offered codecs: {:?}", offered_codecs);
        info!("🎼 Supported codecs: {:?}", supported_codecs);
        
        // Find intersection (common codecs)
        let mut negotiated = Vec::new();
        for offered in &offered_codecs {
            if supported_codecs.contains(offered) {
                negotiated.push(offered.clone());
            }
        }
        
        // If no common codecs, fall back to PCMU (0)
        if negotiated.is_empty() {
            negotiated.push("0".to_string()); // PCMU
        }
        
        info!("🤝 Negotiated codecs: {:?}", negotiated);
        Ok(negotiated)
    }
} 