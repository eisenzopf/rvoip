// Call control operations for the client-core library
// 
// This module contains all call control operations including hold/resume,
// DTMF transmission, call transfer, and capabilities management.

use chrono::Utc;

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::CallId,
};

use crate::client::types::*;

/// Call control operations implementation for ClientManager
impl super::manager::ClientManager {
    // ===== PRIORITY 3.2: CALL CONTROL OPERATIONS =====
    
    /// Put a call on hold
    pub async fn hold_call(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to hold
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
            
        // Use session-core hold functionality
        SessionControl::hold_session(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to hold call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("on_hold".to_string(), "true".to_string());
            call_info.metadata.insert("hold_initiated_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for hold state change
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::HoldStateChanged { on_hold: true },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Put call {} on hold", call_id);
        Ok(())
    }
    
    /// Resume a call from hold
    pub async fn resume_call(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Use session-core resume functionality
        SessionControl::resume_session(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to resume call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("on_hold".to_string(), "false".to_string());
            call_info.metadata.insert("resumed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for hold state change
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::HoldStateChanged { on_hold: false },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Resumed call {} from hold", call_id);
        Ok(())
    }
    
    /// Check if a call is currently on hold
    pub async fn is_call_on_hold(&self, call_id: &CallId) -> ClientResult<bool> {
        if let Some(call_info) = self.call_info.get(call_id) {
            // Check metadata for hold status
            let on_hold = call_info.metadata.get("on_hold")
                .map(|s| s == "true")
                .unwrap_or(false);
            Ok(on_hold)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }
    
    /// Send DTMF digits during a call
    pub async fn send_dtmf(&self, call_id: &CallId, digits: &str) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to send DTMF
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
        
        // Validate DTMF digits
        if digits.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "dtmf_digits".to_string(),
                reason: "DTMF digits cannot be empty".to_string() 
            });
        }
        
        // Check for valid DTMF characters (0-9, A-D, *, #)
        for ch in digits.chars() {
            if !matches!(ch, '0'..='9' | 'A'..='D' | 'a'..='d' | '*' | '#') {
                return Err(ClientError::InvalidConfiguration { 
                    field: "dtmf_digits".to_string(),
                    reason: format!("Invalid DTMF character: {}", ch) 
                });
            }
        }
            
        // Use session-core DTMF functionality
        SessionControl::send_dtmf(&self.coordinator, &session_id, digits)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to send DTMF: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            let dtmf_history = call_info.metadata.entry("dtmf_history".to_string())
                .or_insert_with(String::new);
            if !dtmf_history.is_empty() {
                dtmf_history.push(',');
            }
            dtmf_history.push_str(&format!("{}@{}", digits, Utc::now().to_rfc3339()));
            
            call_info.metadata.insert("last_dtmf_sent".to_string(), digits.to_string());
            call_info.metadata.insert("last_dtmf_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Emit MediaEvent for DTMF
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::DtmfSent { digits: digits.to_string() },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Sent DTMF '{}' to call {}", digits, call_id);
        Ok(())
    }
    
    /// Transfer a call to another destination (blind transfer)
    pub async fn transfer_call(&self, call_id: &CallId, target: &str) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Validate call state
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Connected => {
                    // OK to transfer
                }
                crate::call::CallState::Terminated | 
                crate::call::CallState::Failed | 
                crate::call::CallState::Cancelled => {
                    return Err(ClientError::InvalidCallState { 
                        call_id: *call_id, 
                        current_state: call_info.state.clone() 
                    });
                }
                _ => {
                    return Err(ClientError::InvalidCallStateGeneric { 
                        expected: "Connected".to_string(),
                        actual: format!("{:?}", call_info.state)
                    });
                }
            }
        }
        
        // Validate target URI
        if target.is_empty() {
            return Err(ClientError::InvalidConfiguration { 
                field: "transfer_target".to_string(),
                reason: "Transfer target cannot be empty".to_string() 
            });
        }
        
        if !target.starts_with("sip:") && !target.starts_with("tel:") {
            return Err(ClientError::InvalidConfiguration { 
                field: "transfer_target".to_string(),
                reason: "Transfer target must be a valid SIP or TEL URI".to_string() 
            });
        }
            
        // Use session-core transfer functionality
        SessionControl::transfer_session(&self.coordinator, &session_id, target)
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to transfer call: {}", e) 
            })?;
            
        // Update call metadata
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.metadata.insert("transfer_target".to_string(), target.to_string());
            call_info.metadata.insert("transfer_initiated_at".to_string(), Utc::now().to_rfc3339());
            call_info.metadata.insert("transfer_type".to_string(), "blind".to_string());
        }
        
        // Emit MediaEvent for transfer initiation
        if let Some(handler) = self.call_handler.client_event_handler.read().await.as_ref() {
            let media_event = crate::events::MediaEventInfo {
                call_id: *call_id,
                event_type: crate::events::MediaEventType::TransferInitiated { 
                    target: target.to_string(), 
                    transfer_type: "blind".to_string() 
                },
                timestamp: Utc::now(),
                metadata: {
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("session_id".to_string(), session_id.0.clone());
                    metadata
                },
            };
            handler.on_media_event(media_event).await;
        }
        
        tracing::info!("Initiated blind transfer of call {} to {}", call_id, target);
        Ok(())
    }
    
    /// Perform an attended transfer (consultative transfer)
    pub async fn attended_transfer(&self, call_id1: &CallId, call_id2: &CallId) -> ClientResult<()> {
        // Get session IDs for both calls (for validation, though not directly used below)
        let _session_id1 = self.session_mapping.get(call_id1)
            .ok_or(ClientError::CallNotFound { call_id: *call_id1 })?
            .clone();
        let _session_id2 = self.session_mapping.get(call_id2)
            .ok_or(ClientError::CallNotFound { call_id: *call_id2 })?
            .clone();
            
        // Validate both calls are in connected state
        for call_id in [call_id1, call_id2] {
            if let Some(call_info) = self.call_info.get(call_id) {
                match call_info.state {
                    crate::call::CallState::Connected => {
                        // OK to transfer
                    }
                    crate::call::CallState::Terminated | 
                    crate::call::CallState::Failed | 
                    crate::call::CallState::Cancelled => {
                        return Err(ClientError::InvalidCallState { 
                            call_id: *call_id, 
                            current_state: call_info.state.clone() 
                        });
                    }
                    _ => {
                        return Err(ClientError::InvalidCallStateGeneric { 
                            expected: "Connected".to_string(),
                            actual: format!("{:?}", call_info.state)
                        });
                    }
                }
            }
        }
        
        // For attended transfer, we typically would:
        // 1. Put the first call on hold
        // 2. Establish a consultation call with the transfer target
        // 3. Complete the transfer connecting the original caller to the transfer target
        // 
        // Since session-core doesn't have a specific attended transfer API,
        // we'll simulate it with available operations
        
        // Put first call on hold
        self.hold_call(call_id1).await?;
        
        // Get remote URI from second call to use as transfer target
        let target_uri = if let Some(call_info2) = self.call_info.get(call_id2) {
            call_info2.remote_uri.clone()
        } else {
            return Err(ClientError::CallNotFound { call_id: *call_id2 });
        };
        
        // Transfer the first call to the target of the second call
        self.transfer_call(call_id1, &target_uri).await?;
        
        // Hang up the consultation call since transfer is completing
        self.hangup_call(call_id2).await?;
        
        // Update metadata for attended transfer
        if let Some(mut call_info) = self.call_info.get_mut(call_id1) {
            call_info.metadata.insert("transfer_type".to_string(), "attended".to_string());
            call_info.metadata.insert("consultation_call_id".to_string(), call_id2.to_string());
            call_info.metadata.insert("attended_transfer_completed_at".to_string(), Utc::now().to_rfc3339());
        }
        
        tracing::info!("Completed attended transfer: call {} transferred to target of call {}", call_id1, call_id2);
        Ok(())
    }
    
    /// Get call control capabilities for a specific call
    pub async fn get_call_capabilities(&self, call_id: &CallId) -> ClientResult<CallCapabilities> {
        let call_info = self.get_call(call_id).await?;
        
        let capabilities = match call_info.state {
            crate::call::CallState::Connected => CallCapabilities {
                can_hold: true,
                can_resume: self.is_call_on_hold(call_id).await.unwrap_or(false),
                can_transfer: true,
                can_send_dtmf: true,
                can_mute: true,
                can_hangup: true,
            },
            crate::call::CallState::Ringing | crate::call::CallState::IncomingPending => CallCapabilities {
                can_hold: false,
                can_resume: false,
                can_transfer: false,
                can_send_dtmf: false,
                can_mute: false,
                can_hangup: true, // Can reject
            },
            crate::call::CallState::Initiating | crate::call::CallState::Proceeding => CallCapabilities {
                can_hold: false,
                can_resume: false,
                can_transfer: false,
                can_send_dtmf: false,
                can_mute: false,
                can_hangup: true, // Can cancel
            },
            _ => CallCapabilities::default(), // Terminated states have no capabilities
        };
        
        Ok(capabilities)
    }
} 