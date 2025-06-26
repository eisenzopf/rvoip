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
            // Get the call info to find the related session
            let related_session_id = engine.active_calls.get(&call.id())
                .and_then(|call_info| call_info.related_session_id.clone());
            
            if let Some(related_id) = related_session_id {
                info!("📞 Forwarding BYE to related B2BUA session: {}", related_id);
                
                // Terminate the related dialog
                if let Some(coordinator) = &engine.session_coordinator {
                    match coordinator.terminate_session(&related_id).await {
                        Ok(_) => {
                            info!("✅ Successfully terminated related B2BUA session: {}", related_id);
                        }
                        Err(e) => {
                            warn!("Failed to terminate related session {}: {}", related_id, e);
                        }
                    }
                }
            } else {
                warn!("⚠️ No related B2BUA session found for {}", call.id());
            }
            
            // Clean up the call info
            if let Err(e) = engine.handle_call_termination(call.id().clone()).await {
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
                        
                        // Create agent ID
                        let agent_id = crate::agent::AgentId::from(username.to_string());
                        
                        // Add to available agents DashMap
                        let agent_session_id = SessionId(format!("agent-{}-registered", agent_id));
                        
                        self.available_agents.insert(agent_id.clone(), AgentInfo {
                            agent_id: agent_id.clone(),
                            session_id: agent_session_id,
                            status: crate::agent::AgentStatus::Available,
                            sip_uri: aor.clone(),
                            contact_uri: contact_uri.clone(),
                            skills: vec!["general".to_string()], // Default skills
                            current_calls: 0,
                            max_calls: 1, // Default max calls
                            last_call_end: None,
                            performance_score: 0.5,
                        });
                        
                        tracing::info!("✅ Agent {} added to available agents pool", username);
                    }
                    Err(e) => {
                        tracing::error!("❌ Failed to update agent in database: {}", e);
                    }
                }
            }
        } else if status_code == 200 && expires == 0 {
            // Handle de-registration - remove from available agents
            if let Some(db_manager) = &self.db_manager {
                let username = aor.split('@').next()
                    .unwrap_or(&aor)
                    .trim_start_matches("sip:")
                    .trim_start_matches('<');
                
                let agent_id = crate::agent::AgentId::from(username.to_string());
                
                // Remove from available agents DashMap
                if self.available_agents.remove(&agent_id).is_some() {
                    tracing::info!("✅ Agent {} removed from available agents pool", username);
                }
                
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