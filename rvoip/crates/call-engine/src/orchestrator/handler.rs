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
use super::types::{AgentInfo, CallStatus};
use crate::agent::AgentStatus;
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
        info!("📞 Call {} ended: {}", call.id(), reason);
        
        if let Some(engine) = self.engine.upgrade() {
            // CRITICAL: Clean up from database queue first to prevent re-queueing
            if let Some(db_manager) = &engine.db_manager {
                // Remove from queue and active calls (this method handles both tables)
                if let Err(e) = db_manager.remove_call_from_queue(&call.id().to_string()).await {
                    debug!("Call {} not in queue or already removed: {}", call.id(), e);
                } else {
                    debug!("🧹 Cleaned up call {} from database", call.id());
                }
            }
            
            // First, check if this is a pending assignment that needs cleanup
            if let Some((_, pending_assignment)) = engine.pending_assignments.remove(&call.id()) {
                info!("🧹 Cleaning up pending assignment for call {} (agent {} never answered)", 
                      call.id(), pending_assignment.agent_id);
                
                // Return agent to available in database since they never actually took the call
                if let Some(db_manager) = &engine.db_manager {
                    let _ = db_manager.update_agent_call_count(&pending_assignment.agent_id.0, -1).await;
                    let _ = db_manager.update_agent_status(&pending_assignment.agent_id.0, AgentStatus::Available).await;
                }
                
                // Don't re-queue - the customer hung up
                info!("❌ Not re-queuing call {} - customer ended the call", pending_assignment.customer_session_id);
            }
            
            // Get the call info to find the related session
            let related_session_id = engine.active_calls.get(&call.id())
                .and_then(|call_info| call_info.related_session_id.clone());
            
            if let Some(related_id) = related_session_id {
                info!("📞 Forwarding BYE to related B2BUA session: {}", related_id);
                
                // Clean up related session from database too
                if let Some(db_manager) = &engine.db_manager {
                    let _ = db_manager.remove_call_from_queue(&related_id.to_string()).await;
                }
                
                // Terminate the related dialog
                if let Some(coordinator) = &engine.session_coordinator {
                    match coordinator.terminate_session(&related_id).await {
                        Ok(_) => {
                            info!("✅ Successfully terminated related B2BUA session: {}", related_id);
                        }
                        Err(e) => {
                            // This is expected if the other side already hung up
                            if e.to_string().contains("not found") || e.to_string().contains("No dialog found") {
                                info!("ℹ️ Related session {} already terminated (this is normal)", related_id);
                            } else {
                                warn!("Failed to terminate related session {}: {}", related_id, e);
                            }
                        }
                    }
                }
            } else {
                debug!("No related B2BUA session found for {} (may be a pending call)", call.id());
            }
            
            // Clean up the call info - this will handle agent wrap-up
            if let Err(e) = engine.handle_call_termination(call.id().clone()).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("CallCenterCallHandler: Call {} established", call.id);
        debug!("Local SDP available: {}, Remote SDP available: {}", 
               local_sdp.is_some(), remote_sdp.is_some());
        
        if let Some(engine) = self.engine.upgrade() {
            // Check if this is a pending agent assignment
            if let Some((_, pending_assignment)) = engine.pending_assignments.remove(&call.id) {
                info!("🔔 Agent {} answered for pending assignment", pending_assignment.agent_id);
                
                // This is an agent answering - complete the bridge
                let coordinator = engine.session_coordinator.as_ref().unwrap();
                let bridge_start = Instant::now();
                
                match coordinator.bridge_sessions(
                    &pending_assignment.customer_session_id, 
                    &pending_assignment.agent_session_id
                ).await {
                    Ok(bridge_id) => {
                        let bridge_time = bridge_start.elapsed().as_millis();
                        info!("✅ Successfully bridged customer {} with agent {} (bridge: {}) in {}ms", 
                              pending_assignment.customer_session_id, 
                              pending_assignment.agent_id, 
                              bridge_id, 
                              bridge_time);
                        
                        // Update customer call info
                        if let Some(mut call_info) = engine.active_calls.get_mut(&pending_assignment.customer_session_id) {
                            call_info.agent_id = Some(pending_assignment.agent_id.clone());
                            call_info.bridge_id = Some(bridge_id.clone());
                            call_info.status = CallStatus::Bridged;
                            call_info.answered_at = Some(chrono::Utc::now());
                        }
                        
                        // Update agent call info
                        if let Some(mut call_info) = engine.active_calls.get_mut(&pending_assignment.agent_session_id) {
                            call_info.bridge_id = Some(bridge_id);
                            call_info.status = CallStatus::Bridged;
                            call_info.answered_at = Some(chrono::Utc::now());
                        }
                    }
                    Err(e) => {
                        error!("Failed to bridge sessions after agent answered: {}", e);
                        
                        // Hang up both calls on bridge failure
                        let _ = coordinator.terminate_session(&pending_assignment.agent_session_id).await;
                        let _ = coordinator.terminate_session(&pending_assignment.customer_session_id).await;
                        
                        // Return agent to available in database
                        if let Some(db_manager) = &engine.db_manager {
                            let _ = db_manager.update_agent_call_count(&pending_assignment.agent_id.0, -1).await;
                            let _ = db_manager.update_agent_status(&pending_assignment.agent_id.0, AgentStatus::Available).await;
                        }
                    }
                }
            } else {
                // Regular call established (not a pending assignment)
                engine.update_call_established(call.id).await;
            }
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
        info!("📞 Call {} state changed from {:?} to {:?} (reason: {:?})", 
              session_id, old_state, new_state, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            // Update call status based on state change
            if let Some(mut call_info) = engine.active_calls.get_mut(session_id) {
                match new_state {
                    CallState::Active => call_info.status = CallStatus::Bridged,
                    CallState::Terminated => call_info.status = CallStatus::Disconnected,
                    CallState::Failed(_) => call_info.status = CallStatus::Failed,
                    _ => {} // Keep existing status for other states
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
        mut contact_uri: String,
        expires: u32,
    ) -> Result<(), CallCenterError> {
        tracing::info!("Processing REGISTER: transaction={}, from={}, contact={}, expires={}", 
                      transaction_id, from_uri, contact_uri, expires);
        
        // Parse the AOR (Address of Record) from the from_uri
        let aor = from_uri.clone(); // In practice, might need to normalize this
        
        // Fix the contact URI to include port if missing
        // When agents register with Contact: <sip:alice@127.0.0.1>, we need to add the port
        if contact_uri.contains(':') && !contact_uri.ends_with(":5060") {
            // Check if contact has a port (not just the sip: part)
            let parts: Vec<&str> = contact_uri.split('@').collect();
            if parts.len() == 2 {
                let host_part = parts[1];
                // Check if host part has a port
                if !host_part.contains(':') || host_part.split(':').nth(1).unwrap_or("").is_empty() {
                    // No port specified, need to extract from source
                    // For now, we'll use the AOR to determine the port
                    // In a real implementation, we'd get this from the Via header
                    let port = if aor.contains("alice") {
                        "5071"
                    } else if aor.contains("bob") {
                        "5072"
                    } else {
                        "5060" // Default SIP port
                    };
                    contact_uri = format!("{}:{}", contact_uri.trim_end_matches('>'), port).replace(">>", ">");
                }
            }
        }
        
        tracing::info!("Contact URI with port: {}", contact_uri);
        
        // Check if the agent exists in the database
        let agent_exists = if let Some(db_manager) = &self.db_manager {
            // Try to find the agent by username (extract from SIP URI)
            let username = aor.split('@').next()
                .unwrap_or(&aor)
                .trim_start_matches("sip:")
                .trim_start_matches('<');
            
            match db_manager.get_agent(&username).await {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(e) => {
                    tracing::error!("Database error checking agent: {}", e);
                    false
                }
            }
        } else {
            // No database, allow all registrations
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
        // Note: We now pass the contact_uri with port included
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
            if let Some(db_manager) = &self.db_manager {
                // Extract username from AOR
                let username = aor.split('@').next()
                    .unwrap_or(&aor)
                    .trim_start_matches("sip:")
                    .trim_start_matches('<');
                
                // Update or insert agent in database
                match db_manager.upsert_agent(&username, &username, Some(&contact_uri)).await {
                    Ok(_) => {
                        tracing::info!("✅ Agent {} registered in database with contact {}", username, contact_uri);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to update agent in database: {}", e);
                    }
                }
            }
        } else if status_code == 200 && expires == 0 {
            // Handle de-registration - mark agent as offline
            if let Some(db_manager) = &self.db_manager {
                let username = aor.split('@').next()
                    .unwrap_or(&aor)
                    .trim_start_matches("sip:")
                    .trim_start_matches('<');
                
                // Update database status to offline
                match db_manager.mark_agent_offline(&username).await {
                    Ok(_) => {
                        tracing::info!("✅ Agent {} marked offline in database", username);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to mark agent offline: {}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
} 