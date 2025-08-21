//! Session management operations for SessionCoordinator

use crate::api::types::{CallSession, SessionId, SessionStats, CallState};
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
use crate::session::Session;
use super::SessionCoordinator;

impl SessionCoordinator {
    /// Create an outgoing call
    pub async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
        sip_call_id: Option<String>,
    ) -> Result<CallSession> {
        let session_id = SessionId::new();
        
        // Generate Call-ID if not provided (UAC responsibility per RFC 3261)
        let sip_call_id = sip_call_id.or_else(|| Some(format!("call-{}", uuid::Uuid::new_v4())));
        
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
            sip_call_id: sip_call_id.clone(),
        };

        // Create internal session from call session
        let mut session = Session::from_call_session(call.clone());
        if let Some(ref sdp_str) = sdp {
            session.local_sdp = Some(sdp_str.clone());
        }

        // Register session
        self.registry.register_session(session).await?;

        // Send events
        if let Some(ref local_sdp) = sdp {
            self.publish_event(SessionEvent::SdpEvent {
                session_id: session_id.clone(),
                event_type: "local_sdp_offer".to_string(),
                sdp: local_sdp.clone(),
            }).await.map_err(|_| SessionError::internal("Failed to send SDP event"))?;
        }

        self.publish_event(SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            call_state: call.state.clone(),
        }).await.map_err(|_| SessionError::internal("Failed to send session created event"))?;
        
        // CRITICAL: Track From URI BEFORE creating dialog
        // This ensures the mapping exists when the 200 OK arrives
        // Track the 'from' parameter, not config.local_address
        // The 100 calls test uses different From URIs for each call
        self.dialog_coordinator.track_from_uri(session_id.clone(), from);
        
        // Create dialog with the Call-ID
        let dialog_handle = self.dialog_manager
            .create_outgoing_call(session_id.clone(), from, to, sdp, sip_call_id.clone())
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to create call: {}", e)))?;
        
        // CRITICAL FIX: Also set session-to-dialog mapping in the coordinator
        // This ensures bidirectional mapping for proper event routing
        self.dialog_coordinator.map_session_to_dialog(
            session_id.clone(), 
            dialog_handle.dialog_id.clone(),
            sip_call_id.clone()  // Pass the Call-ID for tracking
        );
        
        tracing::info!("📍 SESSION OPS: Mapped session {} to dialog {} for outgoing call", 
                     session_id, dialog_handle.dialog_id);
            
        Ok(call)
    }

    /// Terminate a session
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists
        if self.registry.get_public_session(session_id).await?.is_none() {
            return Err(SessionError::session_not_found(&session_id.0));
        }
        
        // Terminate via dialog
        self.dialog_manager
            .terminate_session(session_id)
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to terminate session: {}", e)))?;
            
        Ok(())
    }

    /// Send DTMF tones on an active session
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        // Verify session exists and is active
        if let Some(session) = self.find_session(session_id).await? {
            match session.state {
                CallState::Active => {
                    // Send DTMF through the dialog manager
                    self.dialog_manager
                        .send_dtmf(session_id, digits)
                        .await
                        .map_err(|e| SessionError::internal(&format!("Failed to send DTMF: {}", e)))?;
                    
                    tracing::info!("Sent DTMF '{}' for session {}", digits, session_id);
                    Ok(())
                }
                _ => {
                    Err(SessionError::invalid_state(&format!("Session {} is not active, current state: {:?}", session_id, session.state)))
                }
            }
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
    }

    /// Generate SDP offer for a session
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        self.media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })
    }

    /// Create a pre-allocated outgoing session (for agent registration)
    pub async fn create_outgoing_session(&self) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Pre-register session in registry without creating dialog yet
        let call_session = CallSession {
            id: session_id.clone(),
            from: String::new(), // Will be set when actually used
            to: String::new(),
            state: CallState::Initiating,
            started_at: None,
            sip_call_id: None,
        };
        
        // Create internal session
        let session = Session::from_call_session(call_session);
        self.registry.register_session(session).await?;
        
        Ok(session_id)
    }

    /// Find a session by ID
    pub async fn find_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        self.registry.get_public_session(session_id).await
    }

    /// List active sessions
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        self.registry.list_active_sessions().await
    }

    /// Get session statistics
    pub async fn get_stats(&self) -> Result<SessionStats> {
        self.registry.get_stats().await
    }
    
    /// Send a SIP response through dialog-core (for REGISTER, etc.)
    /// 
    /// This allows the application to send proper SIP responses when
    /// auto-response is disabled.
    pub async fn send_sip_response(
        &self,
        transaction_id: &str,
        status_code: u16,
        reason_phrase: Option<&str>,
        _headers: Option<Vec<(&str, &str)>>,
    ) -> Result<()> {
        // Parse the transaction ID
        let tx_key: rvoip_dialog_core::transaction::TransactionKey = transaction_id.parse()
            .map_err(|e| SessionError::internal(&format!("Invalid transaction ID: {}", e)))?;
        
        // Map status code to SIP status
        let _status = match status_code {
            200 => rvoip_sip_core::StatusCode::Ok,
            401 => rvoip_sip_core::StatusCode::Unauthorized,
            403 => rvoip_sip_core::StatusCode::Forbidden,
            404 => rvoip_sip_core::StatusCode::NotFound,
            500 => rvoip_sip_core::StatusCode::ServerInternalError,
            _ => rvoip_sip_core::StatusCode::Ok, // Default to OK
        };
        
        // Send response through dialog coordinator's send_response method
        self.dialog_coordinator
            .send_response(&tx_key, status_code, reason_phrase.unwrap_or(""))
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to send SIP response: {}", e)))?;
        
        Ok(())
    }
} 