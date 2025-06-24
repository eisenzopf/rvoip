//! CallHandler implementation for the call center
//!
//! This module provides the CallHandler trait implementation that integrates
//! with session-core to receive and process incoming calls.

use std::sync::Weak;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};
use rvoip_session_core::{
    CallHandler, IncomingCall, CallDecision, CallSession, SessionId, CallState,
    MediaQualityAlertLevel, MediaFlowDirection, WarningCategory
};
use std::time::Instant;

use super::core::CallCenterEngine;
use crate::error::CallCenterError;

/// CallHandler implementation for the call center
#[derive(Clone, Debug)]
pub struct CallCenterCallHandler {
    pub engine: Weak<CallCenterEngine>,
}

#[async_trait]
impl CallHandler for CallCenterCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        debug!("CallCenterCallHandler: Received incoming call {}", call.id);
        
        // Try to get a strong reference to the engine
        if let Some(engine) = self.engine.upgrade() {
            // Process the incoming call through the call center's routing logic
            match engine.process_incoming_call(call).await {
                Ok(decision) => decision,
                Err(e) => {
                    error!("Failed to process incoming call: {}", e);
                    CallDecision::Reject("Call center processing error".to_string())
                }
            }
        } else {
            warn!("Call center engine has been dropped");
            CallDecision::Reject("Call center not available".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("CallCenterCallHandler: Call {} ended: {}", call.id, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            if let Err(e) = engine.handle_call_termination(call.id).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("CallCenterCallHandler: Call {} established", call.id);
        debug!("Local SDP available: {}, Remote SDP available: {}", 
               local_sdp.is_some(), remote_sdp.is_some());
        
        // Update call state to active/bridged
        if let Some(engine) = self.engine.upgrade() {
            engine.update_call_established(call.id).await;
        }
    }
    
    // === New event handler methods ===
    
    async fn on_call_state_changed(
        &self, 
        session_id: &SessionId, 
        old_state: &CallState, 
        new_state: &CallState, 
        reason: Option<&str>
    ) {
        info!("CallCenterCallHandler: Call {} state changed from {:?} to {:?} (reason: {:?})", 
              session_id, old_state, new_state, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            // Update internal tracking
            if let Err(e) = engine.update_call_state(session_id, new_state).await {
                error!("Failed to update call state: {}", e);
            }
            
            // Route calls when ringing
            if matches!(new_state, CallState::Ringing) {
                if let Err(e) = engine.route_incoming_call(session_id).await {
                    error!("Failed to route ringing call: {}", e);
                }
            }
            
            // Clean up on termination
            if matches!(new_state, CallState::Terminated) {
                if let Err(e) = engine.cleanup_call(session_id).await {
                    error!("Failed to cleanup terminated call: {}", e);
                }
            }
        }
    }
    
    async fn on_media_quality(
        &self, 
        session_id: &SessionId, 
        mos_score: f32, 
        packet_loss: f32, 
        alert_level: MediaQualityAlertLevel
    ) {
        debug!("CallCenterCallHandler: Call {} quality - MOS: {}, Loss: {}%, Alert: {:?}", 
               session_id, mos_score, packet_loss, alert_level);
        
        if let Some(engine) = self.engine.upgrade() {
            // Store quality metrics
            if let Err(e) = engine.record_quality_metrics(session_id, mos_score, packet_loss).await {
                error!("Failed to record quality metrics: {}", e);
            }
            
            // Alert supervisors on poor quality
            if matches!(alert_level, MediaQualityAlertLevel::Poor | MediaQualityAlertLevel::Critical) {
                if let Err(e) = engine.alert_poor_quality(session_id, mos_score, alert_level).await {
                    error!("Failed to alert poor quality: {}", e);
                }
            }
        }
    }
    
    async fn on_dtmf(&self, session_id: &SessionId, digit: char, duration_ms: u32) {
        info!("CallCenterCallHandler: Call {} received DTMF '{}' ({}ms)", 
              session_id, digit, duration_ms);
        
        if let Some(engine) = self.engine.upgrade() {
            // Process DTMF for IVR or agent features
            if let Err(e) = engine.process_dtmf_input(session_id, digit).await {
                error!("Failed to process DTMF: {}", e);
            }
        }
    }
    
    async fn on_media_flow(
        &self, 
        session_id: &SessionId, 
        direction: MediaFlowDirection, 
        active: bool, 
        codec: &str
    ) {
        debug!("CallCenterCallHandler: Call {} media flow {:?} {} (codec: {})", 
               session_id, direction, if active { "started" } else { "stopped" }, codec);
        
        if let Some(engine) = self.engine.upgrade() {
            // Track media flow status
            if let Err(e) = engine.update_media_flow(session_id, direction, active, codec).await {
                error!("Failed to update media flow status: {}", e);
            }
        }
    }
    
    async fn on_warning(
        &self, 
        session_id: Option<&SessionId>, 
        category: WarningCategory, 
        message: &str
    ) {
        match session_id {
            Some(id) => warn!("CallCenterCallHandler: Warning for call {} ({:?}): {}", 
                            id, category, message),
            None => warn!("CallCenterCallHandler: General warning ({:?}): {}", 
                         category, message),
        }
        
        if let Some(engine) = self.engine.upgrade() {
            // Log warnings for monitoring
            if let Err(e) = engine.log_warning(session_id, category, message).await {
                error!("Failed to log warning: {}", e);
            }
        }
    }
}

impl CallCenterEngine {
    /// Handle SIP REGISTER request forwarded from session-core
    /// This is called when dialog-core receives a REGISTER and forwards it to us
    pub async fn handle_register_request(
        &self,
        transaction_id: &str,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> Result<(), CallCenterError> {
        tracing::info!("Processing REGISTER: transaction={}, from={}, contact={}, expires={}", 
                      transaction_id, from_uri, contact_uri, expires);
        
        // Parse the AOR (Address of Record) from the from_uri
        let aor = from_uri.clone(); // In practice, might need to normalize this
        
        // Check if the agent exists in the database
        // TODO: Fix limbo parameter binding syntax
        let agent_exists = {
            /*
            let conn = self.database.connection().await;
            match conn.query(
                "SELECT id FROM agents WHERE sip_uri = :aor",
                (("aor", aor.as_str()),)
            ).await {
                Ok(mut rows) => rows.next().await.is_ok(),
                Err(e) => {
                    tracing::error!("Database error checking agent: {}", e);
                    false
                }
            }
            */
            // Temporarily return true to allow compilation
            true
        };
        
        if !agent_exists {
            tracing::warn!("Registration attempt for unknown agent: {}", aor);
            
            // Send 404 Not Found response
            let session_coord = self.session_coordinator.as_ref()
                .ok_or_else(|| CallCenterError::internal(
                    "Session coordinator not available"
                ))?;
            
            session_coord.send_sip_response(
                transaction_id,
                404,
                Some("Agent not found"),
                None,
            ).await
            .map_err(|e| CallCenterError::internal(
                &format!("Failed to send REGISTER response: {}", e)
            ))?;
            
            return Err(CallCenterError::NotFound(
                format!("Agent {} not registered in system", aor)
            ));
        }
        
        // Process the registration with our SIP registrar
        // Note: We now pass the contact_uri as a string instead of a parsed Contact
        let mut registrar = self.sip_registrar.lock().await;
        let response = registrar.process_register_simple(
            &aor,
            &contact_uri,
            Some(expires),
            None, // User-Agent would come from SIP headers
            "unknown".to_string(), // Remote address would come from transport layer
        )?;
        
        tracing::info!("REGISTER processed: {:?} for {}", response.status, aor);
        
        // Send proper SIP response through session-core
        let session_coord = self.session_coordinator.as_ref()
            .ok_or_else(|| CallCenterError::internal(
                "Session coordinator not available"
            ))?;
        
        let (status_code, reason) = match response.status {
            crate::agent::RegistrationStatus::Created => {
                tracing::info!("Sending 200 OK for successful registration");
                (200, Some("Registration successful"))
            }
            crate::agent::RegistrationStatus::Refreshed => {
                tracing::info!("Sending 200 OK for registration refresh");
                (200, Some("Registration refreshed"))
            }
            crate::agent::RegistrationStatus::Removed => {
                tracing::info!("Sending 200 OK for de-registration");
                (200, Some("De-registration successful"))
            }
        };
        
        // Build headers with Contact information
        let expires_str = expires.to_string();
        let contact_header = format!("<{}>;expires={}", contact_uri, expires);
        let headers = vec![
            ("Expires", expires_str.as_str()),
            ("Contact", contact_header.as_str()),
        ];
        
        session_coord.send_sip_response(
            transaction_id,
            status_code,
            reason,
            Some(headers),
        ).await
        .map_err(|e| CallCenterError::internal(
            &format!("Failed to send REGISTER response: {}", e)
        ))?;
        
        tracing::info!("REGISTER response sent: {} {}", status_code, reason.unwrap_or(""));
        
        // Update agent status in database if registration was successful
        if status_code == 200 && expires > 0 {
            // TODO: Fix limbo parameter binding syntax
            /*
            let conn = self.database.connection().await;
            if let Err(e) = conn.execute(
                "UPDATE agents SET status = 'available', last_seen_at = datetime('now') WHERE sip_uri = :aor",
                (("aor", aor.as_str()),)
            ).await {
                tracing::error!("Failed to update agent status: {}", e);
            } else {
                tracing::info!("Updated agent {} status to available", aor);
            }
            */
            tracing::info!("TODO: Update agent {} status to available in database", aor);
        }
        
        Ok(())
    }
} 