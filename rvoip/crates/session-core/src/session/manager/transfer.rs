use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, error, info};

use rvoip_sip_core::{Request, Response, Method, StatusCode};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::builder::headers::ReferToExt;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName};
use rvoip_sip_core::json::ext::SipMessageJson;

use crate::dialog::DialogId;
use crate::events::SessionEvent;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use super::core::SessionManager;
use super::super::session::Session;
use super::super::SessionId;
use super::super::session_types::{TransferId, TransferType, TransferState};

impl SessionManager {
    /// Initiate a call transfer for a session
    pub async fn initiate_transfer(
        &self, 
        session_id: &SessionId, 
        target_uri: String, 
        transfer_type: TransferType,
        referred_by: Option<String>
    ) -> Result<TransferId, Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Delegate to the session
        session.initiate_transfer(target_uri, transfer_type, referred_by).await
    }
    
    /// Build and send a REFER request for call transfer
    pub async fn send_refer_request(
        &self,
        session_id: &SessionId,
        target_uri: String,
        transfer_type: TransferType,
        referred_by: Option<String>
    ) -> Result<TransferId, Error> {
        // Get the session and its dialog
        let session = self.get_session(session_id)?;
        let dialog = session.dialog().await.ok_or_else(|| {
            Error::SessionNotFoundWithId(
                session_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No active dialog for session".to_string()),
                    ..Default::default()
                }
            )
        })?;
        
        // Initiate transfer in session (sets up state)
        let transfer_id = session.initiate_transfer(target_uri.clone(), transfer_type, referred_by.clone()).await?;
        
        // Get next sequence number for dialog
        let next_seq = dialog.local_seq + 1;
        
        // Helper to get display name or empty string
        let local_display = dialog.local_uri.to_display_name().unwrap_or_default();
        let remote_display = dialog.remote_uri.to_display_name().unwrap_or_default();
        
        // Build REFER request based on dialog
        let refer_request = match transfer_type {
            TransferType::Blind => {
                SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
                    .map_err(|e| Error::InvalidRequest(
                        format!("Failed to create REFER request: {}", e),
                        ErrorContext {
                            category: ErrorCategory::Protocol,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(session_id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("REFER request building failed".to_string()),
                            ..Default::default()
                        }
                    ))?
                    .from(
                        &local_display, 
                        &dialog.local_uri.to_string(), 
                        dialog.local_tag.as_deref()
                    )
                    .to(
                        &remote_display, 
                        &dialog.remote_uri.to_string(), 
                        dialog.remote_tag.as_deref()
                    )
                    .call_id(&dialog.call_id)
                    .cseq(next_seq)
                    .contact(&dialog.local_uri.to_string(), None)
                    .refer_to_blind_transfer(&target_uri)
                    .build()
            },
            TransferType::Attended => {
                // For attended transfers, we need consultation session info
                let consultation_session_id = session.consultation_session_id().await;
                
                if let Some(consult_id) = consultation_session_id {
                    let consult_session = self.get_session(&consult_id)?;
                    let consult_dialog = consult_session.dialog().await.ok_or_else(|| {
                        Error::SessionNotFoundWithId(
                            consult_id.to_string(),
                            ErrorContext {
                                category: ErrorCategory::Session,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(consult_id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("No dialog for consultation session".to_string()),
                                ..Default::default()
                            }
                        )
                    })?;
                    
                    SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
                        .map_err(|e| Error::InvalidRequest(
                            format!("Failed to create attended REFER request: {}", e),
                            ErrorContext {
                                category: ErrorCategory::Protocol,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(session_id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Attended REFER request building failed".to_string()),
                                ..Default::default()
                            }
                        ))?
                        .from(
                            &local_display, 
                            &dialog.local_uri.to_string(), 
                            dialog.local_tag.as_deref()
                        )
                        .to(
                            &remote_display, 
                            &dialog.remote_uri.to_string(), 
                            dialog.remote_tag.as_deref()
                        )
                        .call_id(&dialog.call_id)
                        .cseq(next_seq)
                        .contact(&dialog.local_uri.to_string(), None)
                        .refer_to_attended_transfer(
                            &target_uri,
                            &consult_dialog.call_id,
                            consult_dialog.remote_tag.as_deref().unwrap_or(""),
                            consult_dialog.local_tag.as_deref().unwrap_or("")
                        )
                        .build()
                } else {
                    return Err(Error::InvalidSessionStateTransition {
                        from: "attended_transfer".to_string(),
                        to: "without_consultation".to_string(),
                        context: ErrorContext {
                            category: ErrorCategory::Session,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(session_id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Attended transfer requires consultation session".to_string()),
                            ..Default::default()
                        }
                    });
                }
            },
            TransferType::Consultative => {
                // Similar to attended but with different semantics
                SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
                    .map_err(|e| Error::InvalidRequest(
                        format!("Failed to create consultative REFER request: {}", e),
                        ErrorContext {
                            category: ErrorCategory::Protocol,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(session_id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Consultative REFER request building failed".to_string()),
                            ..Default::default()
                        }
                    ))?
                    .from(
                        &local_display, 
                        &dialog.local_uri.to_string(), 
                        dialog.local_tag.as_deref()
                    )
                    .to(
                        &remote_display, 
                        &dialog.remote_uri.to_string(), 
                        dialog.remote_tag.as_deref()
                    )
                    .call_id(&dialog.call_id)
                    .cseq(next_seq)
                    .contact(&dialog.local_uri.to_string(), None)
                    .refer_to_uri(&target_uri)
                    .build()
            }
        };
        
        // TODO: Add Referred-By header support when sip-core supports it
        
        // Send the REFER request through the transaction manager
        // TODO: Integrate with transaction manager to actually send the request
        // For now, we simulate sending and track the transfer
        
        info!("Built REFER request for transfer {}: {} -> {}", transfer_id, session_id, target_uri);
        debug!("REFER request: {:?}", refer_request);
        
        Ok(transfer_id)
    }
    
    /// Handle an incoming REFER request
    pub async fn handle_refer_request(
        &self,
        refer_request: &Request,
        dialog_id: &DialogId
    ) -> Result<TransferId, Error> {
        // Find the session for this dialog
        let session = self.find_session_by_dialog(dialog_id)?;
        
        // Extract transfer information from REFER request
        let refer_to = refer_request.get_header_value(&HeaderName::ReferTo)
            .ok_or_else(|| Error::InvalidRequest(
                "Missing Refer-To header".to_string(),
                ErrorContext {
                    category: ErrorCategory::Protocol,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("REFER request missing required Refer-To header".to_string()),
                    ..Default::default()
                }
            ))?;
        
        let referred_by = refer_request.get_header_value(&HeaderName::ReferredBy)
            .map(|v| v.to_string());
        
        // Determine transfer type from Refer-To header
        let refer_to_str = refer_to.to_string();
        let transfer_type = if refer_to_str.contains("Replaces=") {
            TransferType::Attended
        } else {
            TransferType::Blind
        };
        
        // Initiate the transfer
        let transfer_id = session.initiate_transfer(
            refer_to_str.clone(),
            transfer_type,
            referred_by
        ).await?;
        
        // Accept the transfer immediately (this should send 202 Accepted)
        session.accept_transfer(&transfer_id).await?;
        
        // TODO: Send 202 Accepted response through transaction manager
        
        info!("Handled REFER request for session {}, transfer ID: {}", session.id, transfer_id);
        debug!("Refer-To: {}", refer_to_str);
        
        Ok(transfer_id)
    }
    
    /// Send a 202 Accepted response to a REFER request
    pub async fn send_refer_accepted(
        &self,
        refer_request: &Request,
        transfer_id: &TransferId
    ) -> Result<Response, Error> {
        let response = SimpleResponseBuilder::new(StatusCode::Accepted, None)
            .build();
        
        info!("Sending 202 Accepted for REFER transfer {}", transfer_id);
        
        Ok(response)
    }
    
    /// Create a consultation call for attended transfer
    pub async fn create_consultation_call(
        &self,
        original_session_id: &SessionId,
        target_uri: String
    ) -> Result<Arc<Session>, Error> {
        // Get the original session
        let original_session = self.get_session(original_session_id)?;
        
        // Create a new outgoing session for consultation
        let consultation_session = self.create_outgoing_session().await?;
        
        // Link the consultation session to the original session
        original_session.set_consultation_session(Some(consultation_session.id.clone())).await;
        
        // TODO: Actually initiate the outbound INVITE to target_uri
        
        // Publish consultation call created event
        self.event_bus.publish(SessionEvent::ConsultationCallCreated {
            original_session_id: original_session_id.clone(),
            consultation_session_id: consultation_session.id.clone(),
            transfer_id: "consultation".to_string(), // Would be a real transfer ID in full implementation
        });
        
        info!("Created consultation call {} for original session {} -> {}", 
              consultation_session.id, original_session_id, target_uri);
        
        Ok(consultation_session)
    }
    
    /// Complete an attended transfer by connecting two sessions
    pub async fn complete_attended_transfer(
        &self,
        transfer_id: &TransferId,
        transferor_session_id: &SessionId,
        transferee_session_id: &SessionId
    ) -> Result<(), Error> {
        // Get both sessions
        let transferor_session = self.get_session(transferor_session_id)?;
        let transferee_session = self.get_session(transferee_session_id)?;
        
        // Setup RTP relay between the sessions
        let relay_id = self.setup_rtp_relay(transferor_session_id, transferee_session_id).await?;
        
        // Complete the transfer on the transferor session
        transferor_session.complete_transfer(transfer_id, "200 OK".to_string()).await?;
        
        // Publish completion event
        self.event_bus.publish(SessionEvent::ConsultationCallCompleted {
            original_session_id: transferor_session_id.clone(),
            consultation_session_id: transferee_session_id.clone(),
            transfer_id: transfer_id.to_string(),
            success: true,
        });
        
        info!("Completed attended transfer {}, relay ID: {:?}", transfer_id, relay_id);
        
        Ok(())
    }
    
    /// Handle transfer progress notifications (NOTIFY)
    pub async fn handle_transfer_notify(
        &self,
        notify_request: &Request,
        dialog_id: &DialogId
    ) -> Result<(), Error> {
        // Find the session for this dialog
        let session = self.find_session_by_dialog(dialog_id)?;
        
        // Extract transfer status from NOTIFY body
        let status = if !notify_request.body().is_empty() {
            // Parse the subscription state - simplified for this implementation
            let body_str = String::from_utf8_lossy(notify_request.body());
            if body_str.contains("200") {
                "200 OK".to_string()
            } else if body_str.contains("100") {
                "100 Trying".to_string()
            } else if body_str.contains("18") {
                format!("18x {}", body_str.lines().next().unwrap_or("Ringing"))
            } else {
                body_str.to_string()
            }
        } else {
            "Unknown status".to_string()
        };
        
        // Find the current transfer for this session
        if let Some(transfer_context) = session.current_transfer().await {
            session.update_transfer_progress(&transfer_context.id, status.clone()).await?;
            
            // If this is a final success response, complete the transfer
            if status.contains("200") {
                session.complete_transfer(&transfer_context.id, status.clone()).await?;
                info!("Transfer {} completed successfully: {}", transfer_context.id, status);
            } else if status.contains("4") || status.contains("5") || status.contains("6") {
                // Error response, fail the transfer
                session.fail_transfer(&transfer_context.id, status.clone()).await?;
                error!("Transfer {} failed: {}", transfer_context.id, status);
            } else {
                // Provisional response, just update progress
                debug!("Transfer {} progress: {}", transfer_context.id, status);
            }
        }
        
        debug!("Handled transfer NOTIFY for dialog {}: {}", dialog_id, status);
        
        Ok(())
    }
    
    /// Send a NOTIFY message for transfer progress
    pub async fn send_transfer_notify(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId,
        status_code: u16,
        reason_phrase: &str
    ) -> Result<Request, Error> {
        // Get the session and its dialog
        let session = self.get_session(session_id)?;
        let dialog = session.dialog().await.ok_or_else(|| {
            Error::SessionNotFoundWithId(
                session_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No active dialog for session".to_string()),
                    ..Default::default()
                }
            )
        })?;
        
        // Create NOTIFY body with SIP message fragment
        let notify_body = format!("SIP/2.0 {} {}\r\n", status_code, reason_phrase);
        
        // Get next sequence number for dialog
        let next_seq = dialog.local_seq + 1;
        
        // Helper to get display names
        let local_display = dialog.local_uri.to_display_name().unwrap_or_default();
        let remote_display = dialog.remote_uri.to_display_name().unwrap_or_default();
        
        // Build NOTIFY request
        let notify_request = SimpleRequestBuilder::new(Method::Notify, &dialog.remote_target.to_string())
            .map_err(|e| Error::InvalidRequest(
                format!("Failed to create NOTIFY request: {}", e),
                ErrorContext {
                    category: ErrorCategory::Protocol,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("NOTIFY request building failed".to_string()),
                    ..Default::default()
                }
            ))?
            .from(
                &local_display, 
                &dialog.local_uri.to_string(), 
                dialog.local_tag.as_deref()
            )
            .to(
                &remote_display, 
                &dialog.remote_uri.to_string(), 
                dialog.remote_tag.as_deref()
            )
            .call_id(&dialog.call_id)
            .cseq(next_seq)
            .contact(&dialog.local_uri.to_string(), None)
            .content_type("message/sipfrag")
            .body(notify_body.as_bytes().to_vec())
            .build();
        
        info!("Sending NOTIFY for transfer {} progress: {} {}", transfer_id, status_code, reason_phrase);
        
        Ok(notify_request)
    }
    
    /// Get all sessions with active transfers
    pub async fn get_sessions_with_transfers(&self) -> Vec<Arc<Session>> {
        let mut sessions_with_transfers = Vec::new();
        
        for entry in self.sessions.iter() {
            let session = entry.value().clone();
            if session.has_transfer_in_progress().await {
                sessions_with_transfers.push(session);
            }
        }
        
        sessions_with_transfers
    }
    
    /// Cancel an ongoing transfer
    pub async fn cancel_transfer(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId,
        reason: String
    ) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Fail the transfer
        session.fail_transfer(transfer_id, reason.clone()).await?;
        
        // TODO: Send appropriate SIP responses/requests to cancel the transfer
        
        info!("Cancelled transfer {} for session {}: {}", transfer_id, session_id, reason);
        
        Ok(())
    }
    
    /// Handle blind transfer completion
    pub async fn handle_blind_transfer_completion(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        // Get the session
        let session = self.get_session(session_id)?;
        
        // Complete the transfer and terminate the session
        session.complete_transfer(transfer_id, "200 OK".to_string()).await?;
        
        info!("Completed blind transfer {} for session {}", transfer_id, session_id);
        
        Ok(())
    }
} 