use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, error, info, warn};

use rvoip_sip_core::{Request, Response, Method, StatusCode};
use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
use rvoip_sip_core::builder::headers::{ReferToExt, ReferredByExt};
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName, TypedHeader};
use rvoip_sip_core::types::uri::Uri;
use rvoip_sip_core::json::ext::{SipMessageJson, SipJsonExt};
use rvoip_transaction_core::{TransactionManager, TransactionKey};

use crate::dialog::DialogId;
use crate::dialog::dialog_utils::uri_resolver;
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
        // Get the session and its default dialog
        let session = self.get_session(session_id)?;
        let dialog_id = self.default_dialogs.get(session_id)
            .ok_or_else(|| Error::DialogNotFound(
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Session has no default dialog for REFER".to_string()),
                    ..Default::default()
                }
            ))?;
        
        let dialog = self.dialog_manager.get_dialog(&dialog_id)?;
        
        // Generate transfer ID
        let transfer_id = TransferId::new();
        
        // Get next sequence number for the dialog
        let next_seq = dialog.local_seq + 1;
        
        // Build display names
        let local_display = dialog.local_uri.to_display_name().unwrap_or_default();
        let remote_display = dialog.remote_uri.to_display_name().unwrap_or_default();
        
        // Build the REFER request based on transfer type
        let refer_request = match transfer_type {
            TransferType::Blind => {
                let mut builder = SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
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
                    .refer_to_blind_transfer(&target_uri);
                
                // Add Referred-By header if provided
                if let Some(ref referred_by_uri) = referred_by {
                    builder = builder.referred_by_str(referred_by_uri)
                        .map_err(|e| Error::InvalidRequest(
                            format!("Failed to add Referred-By header: {}", e),
                            ErrorContext {
                                category: ErrorCategory::Protocol,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(session_id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Referred-By header parsing failed".to_string()),
                                ..Default::default()
                            }
                        ))?;
                }
                
                builder.build()
            },
            
            TransferType::Attended => {
                // For attended transfer, we need consultation dialog information
                // This would typically come from a consultation session
                // For now, we'll create a basic attended transfer structure
                
                // TODO: Get consultation dialog information from session context
                let consult_call_id = format!("consult-{}", transfer_id);
                let consult_to_tag = "consult-to-tag";
                let consult_from_tag = "consult-from-tag";
                    
                let mut builder = SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
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
                        &consult_call_id,
                        consult_to_tag,
                        consult_from_tag
                    );
                
                // Add Referred-By header if provided
                if let Some(ref referred_by_uri) = referred_by {
                    builder = builder.referred_by_str(referred_by_uri)
                        .map_err(|e| Error::InvalidRequest(
                            format!("Failed to add Referred-By header: {}", e),
                            ErrorContext {
                                category: ErrorCategory::Protocol,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(session_id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Referred-By header parsing failed".to_string()),
                                ..Default::default()
                            }
                        ))?;
                }
                
                builder.build()
            }
            
            TransferType::Consultative => {
                // Similar to attended but with different semantics
                let mut builder = SimpleRequestBuilder::new(Method::Refer, &dialog.remote_target.to_string())
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
                    .refer_to_uri(&target_uri);
                
                // Add Referred-By header if provided
                if let Some(ref referred_by_uri) = referred_by {
                    builder = builder.referred_by_str(referred_by_uri)
                        .map_err(|e| Error::InvalidRequest(
                            format!("Failed to add Referred-By header: {}", e),
                            ErrorContext {
                                category: ErrorCategory::Protocol,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(session_id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Referred-By header parsing failed".to_string()),
                                ..Default::default()
                            }
                        ))?;
                }
                
                builder.build()
            }
        };
        
        // Send the REFER request through the transaction manager
        info!("Sending REFER request for transfer {}: {} -> {}", transfer_id, session_id, target_uri);
        if let Ok(json) = refer_request.to_json_string_pretty() {
            debug!("REFER request: {}", json);
        }
        
        // Create client transaction for the REFER request and send it
        // Use the URI resolver to convert the URI to a SocketAddr
        let destination = match uri_resolver::resolve_uri_to_socketaddr(&dialog.remote_target).await {
            Some(addr) => addr,
            None => {
                error!("Failed to resolve destination address for REFER");
                return Err(Error::CannotResolveDestination(
                    format!("Failed to resolve destination for REFER: {}", dialog.remote_target),
                    ErrorContext {
                        category: ErrorCategory::Network,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        session_id: Some(session_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("REFER destination resolution failed".to_string()),
                        ..Default::default()
                    }
                ));
            }
        };
        
        match self.dialog_manager.send_request(&dialog_id, Method::Refer, Some(refer_request.to_string())).await {
            Ok(transaction_id) => {
                info!("Created REFER transaction: {}", transaction_id);
                info!("REFER request sent successfully for transfer {}", transfer_id);
            },
            Err(e) => {
                error!("Failed to send REFER request: {}", e);
                return Err(Error::TransactionFailed(
                    format!("Failed to send REFER request: {}", e),
                    Some(Box::new(e)),
                    ErrorContext {
                        category: ErrorCategory::Network,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        session_id: Some(session_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("REFER request sending failed".to_string()),
                        ..Default::default()
                    }
                ));
            }
        }
        
        // Update session with transfer context
        session.initiate_transfer(target_uri.clone(), transfer_type, referred_by.clone()).await?;
        
        // Publish transfer initiated event
        let event = SessionEvent::TransferInitiated {
            session_id: session_id.clone(),
            transfer_id: transfer_id.to_string(),
            transfer_type: format!("{:?}", transfer_type),
            target_uri: target_uri.clone(),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish TransferInitiated event: {}", e);
        }
        
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
        let session_id = session.id.clone();
        
        info!("Handling incoming REFER request for session {}", session_id);
        if let Ok(json) = refer_request.to_json_string_pretty() {
            debug!("REFER request: {}", json);
        }
        
        // Extract transfer information from REFER request
        let refer_to = refer_request.header(&HeaderName::ReferTo)
            .and_then(|h| match h {
                TypedHeader::ReferTo(rt) => Some(rt.uri().to_string()),
                _ => None,
            })
            .ok_or_else(|| Error::InvalidRequest(
                "Missing or invalid Refer-To header".to_string(),
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
        
        let referred_by = refer_request.header(&HeaderName::ReferredBy)
            .and_then(|h| match h {
                TypedHeader::ReferredBy(rb) => Some(rb.address().uri().to_string()),
                _ => None,
            });
        
        // Determine transfer type from Refer-To header
        let transfer_type = if refer_to.contains("Replaces=") {
            TransferType::Attended
        } else {
            TransferType::Blind
        };
        
        info!("Incoming transfer: type={:?}, target={}, referred_by={:?}", 
              transfer_type, refer_to, referred_by);
        
        // Initiate the transfer
        let transfer_id = session.initiate_transfer(
            refer_to.clone(),
            transfer_type,
            referred_by.clone()
        ).await?;
        
        // Accept the transfer immediately (this should send 202 Accepted)
        session.accept_transfer(&transfer_id).await?;
        
        // Send 202 Accepted response
        self.send_refer_accepted_response(refer_request, dialog_id).await?;
        
        // Publish transfer accepted event
        let event = SessionEvent::TransferAccepted {
            session_id: session_id.clone(),
            transfer_id: transfer_id.to_string(),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish TransferAccepted event: {}", e);
        }
        
        info!("Transfer {} accepted for session {}", transfer_id, session_id);
        
        Ok(transfer_id)
    }
    
    /// Send a 202 Accepted response to a REFER request
    pub async fn send_refer_accepted_response(
        &self,
        refer_request: &Request,
        dialog_id: &DialogId
    ) -> Result<(), Error> {
        let dialog = self.dialog_manager.get_dialog(dialog_id)?;
        
        // Build 202 Accepted response
        let response = SimpleResponseBuilder::new(StatusCode::Accepted, None)
            .contact(&dialog.local_uri.to_string(), None)
            .build();
        
        info!("Sending 202 Accepted response for REFER");
        if let Ok(json) = response.to_json_string_pretty() {
            debug!("202 Accepted response: {}", json);
        }
        
        // Send response through transaction manager
        // Create transaction key from the REFER request
        let transaction_key = match rvoip_transaction_core::TransactionKey::from_request(refer_request) {
            Some(key) => key,
            None => {
                error!("Failed to create transaction key from REFER request");
                return Err(Error::InvalidRequest(
                    "Failed to create transaction key from REFER request".to_string(),
                    ErrorContext {
                        category: ErrorCategory::Protocol,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        dialog_id: Some(dialog_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("REFER request missing required headers for transaction key".to_string()),
                        ..Default::default()
                    }
                ));
            }
        };
        
        match self.dialog_manager.send_response(&transaction_key, response).await {
            Ok(()) => {
                info!("202 Accepted response sent successfully");
            },
            Err(e) => {
                error!("Failed to send 202 Accepted response: {}", e);
                return Err(Error::TransactionFailed(
                    format!("Failed to send 202 Accepted response: {}", e),
                    Some(Box::new(e)),
                    ErrorContext {
                        category: ErrorCategory::Network,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        dialog_id: Some(dialog_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("202 Accepted response sending failed".to_string()),
                        ..Default::default()
                    }
                ));
            }
        }
        
        Ok(())
    }
    
    /// Process a response to a REFER request
    pub async fn process_refer_response(
        &self,
        response: &Response,
        session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        let session = self.get_session(session_id)?;
        
        info!("Processing REFER response for transfer {}: status={}", 
              transfer_id, response.status.as_u16());
        if let Ok(json) = response.to_json_string_pretty() {
            debug!("REFER response: {}", json);
        }
        
        match response.status.as_u16() {
            200..=299 => {
                // Success response - transfer accepted
                session.accept_transfer(transfer_id).await?;
                
                let event = SessionEvent::TransferAccepted {
                    session_id: session_id.clone(),
                    transfer_id: transfer_id.to_string(),
                };
                
                if let Err(e) = self.event_bus.publish(event).await {
                    warn!("Failed to publish TransferAccepted event: {}", e);
                }
                
                info!("Transfer {} accepted by remote party", transfer_id);
            },
            
            400..=699 => {
                // Error response - transfer failed
                let reason = format!("{} {}", response.status.as_u16(), response.status.as_reason());
                session.fail_transfer(transfer_id, reason.clone()).await?;
                
                let event = SessionEvent::TransferFailed {
                    session_id: session_id.clone(),
                    transfer_id: transfer_id.to_string(),
                    reason: reason.clone(),
                };
                
                if let Err(e) = self.event_bus.publish(event).await {
                    warn!("Failed to publish TransferFailed event: {}", e);
                }
                
                error!("Transfer {} failed: {}", transfer_id, reason);
            },
            
            _ => {
                // Provisional response - update progress
                let status = format!("{} {}", response.status.as_u16(), response.status.as_reason());
                
                let event = SessionEvent::TransferProgress {
                    session_id: session_id.clone(),
                    transfer_id: transfer_id.to_string(),
                    status: status.clone(),
                };
                
                if let Err(e) = self.event_bus.publish(event).await {
                    warn!("Failed to publish TransferProgress event: {}", e);
                }
                
                debug!("Transfer {} progress: {}", transfer_id, status);
            }
        }
        
        Ok(())
    }
    
    /// Handle a NOTIFY request with transfer progress
    pub async fn handle_transfer_notify(
        &self,
        notify_request: &Request,
        dialog_id: &DialogId
    ) -> Result<(), Error> {
        let session = self.find_session_by_dialog(dialog_id)?;
        let session_id = session.id.clone();
        
        info!("Handling transfer NOTIFY for session {}", session_id);
        if let Ok(json) = notify_request.to_json_string_pretty() {
            debug!("NOTIFY request: {}", json);
        }
        
        // Extract transfer progress from NOTIFY body
        let body = if !notify_request.body.is_empty() {
            String::from_utf8_lossy(&notify_request.body).to_string()
        } else {
            String::new()
        };
        
        // Parse sipfrag body to extract status
        let status = if body.starts_with("SIP/2.0") {
            // Extract status line from sipfrag
            body.lines().next().unwrap_or("Unknown").to_string()
        } else {
            body
        };
        
        // TODO: Extract transfer ID from Event header or other context
        let transfer_id = "unknown-transfer-id".to_string();
        
        // Determine if this is completion or progress
        if status.contains("200") || status.contains("OK") {
            // Transfer completed successfully
            let event = SessionEvent::TransferCompleted {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                final_status: status.clone(),
            };
            
            if let Err(e) = self.event_bus.publish(event).await {
                warn!("Failed to publish TransferCompleted event: {}", e);
            }
            
            info!("Transfer {} completed: {}", transfer_id, status);
        } else if status.contains("4") || status.contains("5") || status.contains("6") {
            // Transfer failed
            let event = SessionEvent::TransferFailed {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                reason: status.clone(),
            };
            
            if let Err(e) = self.event_bus.publish(event).await {
                warn!("Failed to publish TransferFailed event: {}", e);
            }
            
            error!("Transfer {} failed: {}", transfer_id, status);
        } else {
            // Transfer progress
            let event = SessionEvent::TransferProgress {
                session_id: session_id.clone(),
                transfer_id: transfer_id.clone(),
                status: status.clone(),
            };
            
            if let Err(e) = self.event_bus.publish(event).await {
                warn!("Failed to publish TransferProgress event: {}", e);
            }
            
            debug!("Transfer {} progress: {}", transfer_id, status);
        }
        
        Ok(())
    }
    
    /// Send a NOTIFY request with transfer progress
    pub async fn send_transfer_notify(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId,
        status: String
    ) -> Result<(), Error> {
        let dialog_id = self.default_dialogs.get(session_id)
            .ok_or_else(|| Error::DialogNotFound(
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Session has no default dialog for NOTIFY".to_string()),
                    ..Default::default()
                }
            ))?;
        
        let dialog = self.dialog_manager.get_dialog(&dialog_id)?;
        
        // Build NOTIFY request with sipfrag body
        let sipfrag_body = format!("SIP/2.0 {}", status);
        let next_seq = dialog.local_seq + 1;
        
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
                &dialog.local_uri.to_display_name().unwrap_or_default(), 
                &dialog.local_uri.to_string(), 
                dialog.local_tag.as_deref()
            )
            .to(
                &dialog.remote_uri.to_display_name().unwrap_or_default(), 
                &dialog.remote_uri.to_string(), 
                dialog.remote_tag.as_deref()
            )
            .call_id(&dialog.call_id)
            .cseq(next_seq)
            .contact(&dialog.local_uri.to_string(), None)
            .content_type("message/sipfrag")
            .body(sipfrag_body.into_bytes())
            .build();
        
        info!("Sending transfer NOTIFY for transfer {}: {}", transfer_id, status);
        if let Ok(json) = notify_request.to_json_string_pretty() {
            debug!("NOTIFY request: {}", json);
        }
        
        // Send NOTIFY through transaction manager
        // Use the URI resolver to convert the URI to a SocketAddr
        let destination = match uri_resolver::resolve_uri_to_socketaddr(&dialog.remote_target).await {
            Some(addr) => addr,
            None => {
                error!("Failed to resolve destination address for NOTIFY");
                return Err(Error::CannotResolveDestination(
                    format!("Failed to resolve destination for NOTIFY: {}", dialog.remote_target),
                    ErrorContext {
                        category: ErrorCategory::Network,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        session_id: Some(session_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("NOTIFY destination resolution failed".to_string()),
                        ..Default::default()
                    }
                ));
            }
        };
        
        match self.dialog_manager.send_request(&dialog_id, Method::Notify, Some(notify_request.to_string())).await {
            Ok(transaction_id) => {
                info!("Created NOTIFY transaction: {}", transaction_id);
                info!("NOTIFY request sent successfully for transfer {}", transfer_id);
            },
            Err(e) => {
                error!("Failed to send NOTIFY request: {}", e);
                return Err(Error::TransactionFailed(
                    format!("Failed to send NOTIFY request: {}", e),
                    Some(Box::new(e)),
                    ErrorContext {
                        category: ErrorCategory::Network,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::Retry,
                        retryable: true,
                        session_id: Some(session_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("NOTIFY request sending failed".to_string()),
                        ..Default::default()
                    }
                ));
            }
        }
        
        Ok(())
    }
    
    /// Get all sessions that have active transfers
    pub async fn get_sessions_with_transfers(&self) -> Vec<(SessionId, Vec<TransferId>)> {
        let mut sessions_with_transfers = Vec::new();
        
        for session_entry in self.sessions.iter() {
            let session_id = session_entry.key().clone();
            let session = session_entry.value();
            
            // Get transfer context from session
            if let Some(transfer_context) = session.current_transfer().await {
                sessions_with_transfers.push((session_id, vec![transfer_context.id]));
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
        let session = self.get_session(session_id)?;
        
        // Fail the transfer with cancellation reason
        session.fail_transfer(transfer_id, reason.clone()).await?;
        
        // Publish transfer failed event
        let event = SessionEvent::TransferFailed {
            session_id: session_id.clone(),
            transfer_id: transfer_id.to_string(),
            reason: format!("Cancelled: {}", reason),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish TransferFailed event: {}", e);
        }
        
        info!("Transfer {} cancelled for session {}: {}", transfer_id, session_id, reason);
        
        Ok(())
    }
    
    /// Create a consultation call for attended transfer
    pub async fn create_consultation_call(
        &self,
        original_session_id: &SessionId,
        target_uri: String
    ) -> Result<SessionId, Error> {
        // TODO: Implement consultation call creation
        // This would involve:
        // 1. Creating a new session for the consultation call
        // 2. Initiating an INVITE to the target
        // 3. Linking the consultation session to the original session
        // 4. Managing the consultation call lifecycle
        
        let consultation_session_id = SessionId::new();
        
        info!("Creating consultation call from {} to {}", original_session_id, target_uri);
        
        // For now, we'll just generate a new session ID and publish an event
        let transfer_id = TransferId::new();
        
        let event = SessionEvent::ConsultationCallCreated {
            original_session_id: original_session_id.clone(),
            consultation_session_id: consultation_session_id.clone(),
            transfer_id: transfer_id.to_string(),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish ConsultationCallCreated event: {}", e);
        }
        
        Ok(consultation_session_id)
    }
    
    /// Complete an attended transfer by connecting two sessions
    pub async fn complete_attended_transfer(
        &self,
        transferor_session_id: &SessionId,
        transferee_session_id: &SessionId,
        consultation_session_id: &SessionId
    ) -> Result<(), Error> {
        info!("Completing attended transfer: transferor={}, transferee={}, consultation={}", 
              transferor_session_id, transferee_session_id, consultation_session_id);
        
        // Step 1: Coordinate media during transfer
        let transfer_id = TransferId::new();
        
        // Get all sessions involved in the transfer
        let transferor_session = self.get_session(transferor_session_id)?;
        let transferee_session = self.get_session(transferee_session_id)?;
        let consultation_session = self.get_session(consultation_session_id)?;
        
        // Step 2: Setup media bridging for the transfer
        match self.setup_transfer_media_coordination(
            transferor_session_id,
            transferee_session_id,
            consultation_session_id,
            &transfer_id
        ).await {
            Ok(()) => {
                info!("Media coordination setup successful for transfer {}", transfer_id);
            },
            Err(e) => {
                error!("Failed to setup media coordination for transfer {}: {}", transfer_id, e);
                
                // Publish transfer failed event
                let event = SessionEvent::TransferFailed {
                    session_id: transferor_session_id.clone(),
                    transfer_id: transfer_id.to_string(),
                    reason: format!("Media coordination failed: {}", e),
                };
                
                if let Err(e) = self.event_bus.publish(event).await {
                    warn!("Failed to publish TransferFailed event: {}", e);
                }
                
                return Err(e);
            }
        }
        
        // Step 3: Send REFER with Replaces to connect transferee and consultation target
        // TODO: Implement actual REFER with Replaces header
        
        // Step 4: Coordinate media transfer from consultation to transferee
        match self.execute_media_transfer(
            consultation_session_id,
            transferee_session_id,
            &transfer_id
        ).await {
            Ok(()) => {
                info!("Media transfer executed successfully for transfer {}", transfer_id);
            },
            Err(e) => {
                error!("Failed to execute media transfer for transfer {}: {}", transfer_id, e);
                
                // Cleanup media coordination
                let _ = self.cleanup_transfer_media_coordination(&transfer_id).await;
                
                return Err(e);
            }
        }
        
        // Step 5: Terminate the transferor session and cleanup
        match self.terminate_transferor_session(transferor_session_id, &transfer_id).await {
            Ok(()) => {
                info!("Transferor session terminated successfully for transfer {}", transfer_id);
            },
            Err(e) => {
                warn!("Failed to terminate transferor session for transfer {}: {}", transfer_id, e);
                // Continue with transfer completion despite this error
            }
        }
        
        // Step 6: Publish successful transfer completion event
        let event = SessionEvent::ConsultationCallCompleted {
            original_session_id: transferor_session_id.clone(),
            consultation_session_id: consultation_session_id.clone(),
            transfer_id: transfer_id.to_string(),
            success: true,
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish ConsultationCallCompleted event: {}", e);
        }
        
        info!("Attended transfer {} completed successfully", transfer_id);
        Ok(())
    }
    
    /// Setup media coordination for call transfer
    pub async fn setup_transfer_media_coordination(
        &self,
        transferor_session_id: &SessionId,
        transferee_session_id: &SessionId,
        consultation_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Setting up media coordination for transfer {}", transfer_id);
        
        // Step 1: Put transferor session media on hold
        self.hold_session_media(transferor_session_id, transfer_id).await?;
        
        // Step 2: Setup media bridge between consultation and transferee sessions
        let relay_id = self.setup_rtp_relay(consultation_session_id, transferee_session_id).await?;
        
        // Step 3: Store relay information for cleanup
        // TODO: Store relay_id in transfer context for later cleanup
        
        // Step 4: Monitor media quality during transfer
        self.start_transfer_media_monitoring(
            transferor_session_id,
            transferee_session_id,
            consultation_session_id,
            transfer_id
        ).await?;
        
        info!("Media coordination setup completed for transfer {}", transfer_id);
        Ok(())
    }
    
    /// Execute media transfer between sessions
    pub async fn execute_media_transfer(
        &self,
        source_session_id: &SessionId,
        target_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Executing media transfer from {} to {} for transfer {}", 
              source_session_id, target_session_id, transfer_id);
        
        // Step 1: Get media session information from source
        let source_media_info = self.get_session_media_info(source_session_id).await?;
        
        // Step 2: Prepare target session for media transfer
        self.prepare_session_for_media_transfer(target_session_id, &source_media_info, transfer_id).await?;
        
        // Step 3: Coordinate RTP stream transfer
        self.transfer_rtp_streams(source_session_id, target_session_id, transfer_id).await?;
        
        // Step 4: Update media state for both sessions
        self.update_transfer_media_states(source_session_id, target_session_id, transfer_id).await?;
        
        // Step 5: Publish media transfer progress event
        let event = SessionEvent::TransferProgress {
            session_id: target_session_id.clone(),
            transfer_id: transfer_id.to_string(),
            status: "Media transfer completed".to_string(),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish TransferProgress event: {}", e);
        }
        
        info!("Media transfer executed successfully for transfer {}", transfer_id);
        Ok(())
    }
    
    /// Hold session media during transfer
    pub async fn hold_session_media(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Putting session {} media on hold for transfer {}", session_id, transfer_id);
        
        let session = self.get_session(session_id)?;
        
        // Update session media state to paused
        session.set_media_state(crate::session::session::SessionMediaState::Paused).await?;
        
        // Get media session and pause it
        if let Some(media_session_id) = self.media_manager.get_media_session(session_id).await {
            // TODO: Implement media pause functionality in MediaManager
            // For now, we'll just log the action
            info!("Media session {} paused for transfer", media_session_id);
        }
        
        // Publish media hold event
        let event = SessionEvent::Custom {
            session_id: session_id.clone(),
            event_type: "media_hold".to_string(),
            data: serde_json::json!({
                "transfer_id": transfer_id.to_string(),
                "reason": "call_transfer"
            }),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish media hold event: {}", e);
        }
        
        Ok(())
    }
    
    /// Resume session media after transfer
    pub async fn resume_session_media(
        &self,
        session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Resuming session {} media after transfer {}", session_id, transfer_id);
        
        let session = self.get_session(session_id)?;
        
        // Update session media state to active
        session.set_media_state(crate::session::session::SessionMediaState::Active).await?;
        
        // Get media session and resume it
        if let Some(media_session_id) = self.media_manager.get_media_session(session_id).await {
            // TODO: Implement media resume functionality in MediaManager
            // For now, we'll just log the action
            info!("Media session {} resumed after transfer", media_session_id);
        }
        
        // Publish media resume event
        let event = SessionEvent::Custom {
            session_id: session_id.clone(),
            event_type: "media_resume".to_string(),
            data: serde_json::json!({
                "transfer_id": transfer_id.to_string(),
                "reason": "transfer_completed"
            }),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish media resume event: {}", e);
        }
        
        Ok(())
    }
    
    /// Start monitoring media quality during transfer
    pub async fn start_transfer_media_monitoring(
        &self,
        transferor_session_id: &SessionId,
        transferee_session_id: &SessionId,
        consultation_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Starting media quality monitoring for transfer {}", transfer_id);
        
        // Monitor media quality for all sessions involved in the transfer
        let sessions = vec![transferor_session_id, transferee_session_id, consultation_session_id];
        
        for session_id in sessions {
            if let Ok(session) = self.get_session(session_id) {
                // Get current media metrics
                if let Some(metrics) = session.media_metrics().await {
                    // Publish media quality event
                    let event = SessionEvent::Custom {
                        session_id: session_id.clone(),
                        event_type: "transfer_media_quality".to_string(),
                        data: serde_json::json!({
                            "transfer_id": transfer_id.to_string(),
                            "metrics": {
                                "jitter": metrics.jitter_ms,
                                "packet_loss": metrics.packet_loss,
                                "rtt": metrics.rtt_ms
                            }
                        }),
                    };
                    
                    if let Err(e) = self.event_bus.publish(event).await {
                        warn!("Failed to publish media quality event: {}", e);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Get media information for a session
    pub async fn get_session_media_info(
        &self,
        session_id: &SessionId
    ) -> Result<SessionMediaInfo, Error> {
        let session = self.get_session(session_id)?;
        
        // Get media session ID
        let media_session_id = self.media_manager.get_media_session(session_id).await
            .ok_or_else(|| Error::MediaResourceError(
                "No media session found for session".to_string(),
                ErrorContext {
                    category: ErrorCategory::Media,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(session_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Media session lookup failed".to_string()),
                    ..Default::default()
                }
            ))?;
        
        // Get RTP stream information
        let rtp_stream_info = session.rtp_stream_info().await;
        
        // Get media metrics
        let media_metrics = session.media_metrics().await;
        
        Ok(SessionMediaInfo {
            session_id: session_id.clone(),
            media_session_id,
            rtp_stream_info,
            media_metrics,
            media_state: session.media_state().await,
        })
    }
    
    /// Prepare session for media transfer
    pub async fn prepare_session_for_media_transfer(
        &self,
        session_id: &SessionId,
        source_media_info: &SessionMediaInfo,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Preparing session {} for media transfer {}", session_id, transfer_id);
        
        let session = self.get_session(session_id)?;
        
        // Update session media state to negotiating
        session.set_media_state(crate::session::session::SessionMediaState::Negotiating).await?;
        
        // TODO: Implement media preparation logic
        // This would involve:
        // 1. Updating RTP stream parameters
        // 2. Coordinating codec negotiation if needed
        // 3. Setting up media relay parameters
        
        info!("Session {} prepared for media transfer", session_id);
        Ok(())
    }
    
    /// Transfer RTP streams between sessions
    pub async fn transfer_rtp_streams(
        &self,
        source_session_id: &SessionId,
        target_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Transferring RTP streams from {} to {} for transfer {}", 
              source_session_id, target_session_id, transfer_id);
        
        // Setup RTP relay between source and target
        let relay_id = self.setup_rtp_relay(source_session_id, target_session_id).await?;
        
        // TODO: Store relay_id for cleanup
        
        info!("RTP streams transferred successfully for transfer {}", transfer_id);
        Ok(())
    }
    
    /// Update media states for transfer sessions
    pub async fn update_transfer_media_states(
        &self,
        source_session_id: &SessionId,
        target_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Updating media states for transfer {}", transfer_id);
        
        // Update source session media state
        let source_session = self.get_session(source_session_id)?;
        source_session.set_media_state(crate::session::session::SessionMediaState::Paused).await?;
        
        // Update target session media state
        let target_session = self.get_session(target_session_id)?;
        target_session.set_media_state(crate::session::session::SessionMediaState::Active).await?;
        
        info!("Media states updated for transfer {}", transfer_id);
        Ok(())
    }
    
    /// Terminate transferor session
    pub async fn terminate_transferor_session(
        &self,
        transferor_session_id: &SessionId,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Terminating transferor session {} for transfer {}", transferor_session_id, transfer_id);
        
        // Stop media for the transferor session
        self.stop_session_media(transferor_session_id).await?;
        
        // Update session state to terminated
        let session = self.get_session(transferor_session_id)?;
        session.set_state(crate::session::session_types::SessionState::Terminated).await?;
        
        // Publish session terminated event
        let event = SessionEvent::Terminated {
            session_id: transferor_session_id.clone(),
            reason: format!("Transfer {} completed", transfer_id),
        };
        
        if let Err(e) = self.event_bus.publish(event).await {
            warn!("Failed to publish session terminated event: {}", e);
        }
        
        info!("Transferor session {} terminated for transfer {}", transferor_session_id, transfer_id);
        Ok(())
    }
    
    /// Cleanup media coordination after transfer
    pub async fn cleanup_transfer_media_coordination(
        &self,
        transfer_id: &TransferId
    ) -> Result<(), Error> {
        info!("Cleaning up media coordination for transfer {}", transfer_id);
        
        // TODO: Implement cleanup logic
        // This would involve:
        // 1. Removing RTP relays
        // 2. Cleaning up media sessions
        // 3. Releasing media resources
        
        info!("Media coordination cleanup completed for transfer {}", transfer_id);
        Ok(())
    }
}

/// Media information for a session
#[derive(Debug, Clone)]
pub struct SessionMediaInfo {
    pub session_id: SessionId,
    pub media_session_id: crate::media::MediaSessionId,
    pub rtp_stream_info: Option<crate::media::RtpStreamInfo>,
    pub media_metrics: Option<crate::media::QualityMetrics>,
    pub media_state: crate::session::session::SessionMediaState,
} 