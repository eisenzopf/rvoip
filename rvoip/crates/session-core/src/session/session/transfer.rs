use std::time::SystemTime;
use tracing::{debug, error};

use crate::events::SessionEvent;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use super::core::Session;
use super::super::session_id::SessionId;
use super::super::session_types::{
    SessionState, TransferId, TransferState, TransferType, TransferContext
};

impl Session {
    /// Initiate a call transfer (send REFER)
    pub async fn initiate_transfer(&self, target_uri: String, transfer_type: TransferType, referred_by: Option<String>) -> Result<TransferId, Error> {
        // Check if session is in a valid state for transfer
        let state = self.state().await;
        if !matches!(state, SessionState::Connected | SessionState::OnHold) {
            return Err(Error::InvalidSessionStateTransition {
                from: state.to_string(),
                to: "transferring".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("Session must be connected or on hold to initiate transfer".to_string()),
                    ..Default::default()
                }
            });
        }
        
        // Check if there's already a transfer in progress
        {
            let current_transfer = self.transfer_context.lock().await;
            if current_transfer.is_some() {
                return Err(Error::InvalidSessionStateTransition {
                    from: "transfer_in_progress".to_string(),
                    to: "new_transfer".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer already in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        }
        
        // Create transfer context
        let transfer_context = TransferContext {
            id: TransferId::new(),
            transfer_type,
            state: TransferState::Initiated,
            target_uri: target_uri.clone(),
            transferor_session_id: Some(self.id.clone()),
            transferee_session_id: None,
            consultation_session_id: None,
            refer_to: target_uri.clone(),
            referred_by,
            reason: None,
            initiated_at: SystemTime::now(),
            completed_at: None,
        };
        
        let transfer_id = transfer_context.id.clone();
        
        // Store transfer context
        {
            let mut current_transfer = self.transfer_context.lock().await;
            *current_transfer = Some(transfer_context.clone());
        }
        
        // Update session state to transferring
        self.set_state(SessionState::Transferring).await?;
        
        // Publish transfer initiated event
        self.event_bus.publish(SessionEvent::TransferInitiated {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            transfer_type: transfer_type.to_string(),
            target_uri: target_uri,
        });
        
        debug!("Initiated {} transfer for session {} to {}", transfer_type, self.id, transfer_context.refer_to);
        
        Ok(transfer_id)
    }
    
    /// Accept an incoming transfer request (respond to REFER)
    pub async fn accept_transfer(&self, transfer_id: &TransferId) -> Result<(), Error> {
        let mut transfer_guard = self.transfer_context.lock().await;
        
        if let Some(ref mut transfer_context) = transfer_guard.as_mut() {
            if transfer_context.id == *transfer_id {
                match transfer_context.state {
                    TransferState::Initiated => {
                        transfer_context.state = TransferState::Accepted;
                        
                        // Publish transfer accepted event
                        self.event_bus.publish(SessionEvent::TransferAccepted {
                            session_id: self.id.clone(),
                            transfer_id: transfer_id.to_string(),
                        });
                        
                        debug!("Accepted transfer {} for session {}", transfer_id, self.id);
                        Ok(())
                    },
                    _ => {
                        Err(Error::InvalidSessionStateTransition {
                            from: transfer_context.state.to_string(),
                            to: "accepted".to_string(),
                            context: ErrorContext {
                                category: ErrorCategory::Session,
                                severity: ErrorSeverity::Error,
                                recovery: RecoveryAction::None,
                                retryable: false,
                                session_id: Some(self.id.to_string()),
                                timestamp: SystemTime::now(),
                                details: Some("Transfer not in initiated state".to_string()),
                                ..Default::default()
                            }
                        })
                    }
                }
            } else {
                Err(Error::InvalidSessionStateTransition {
                    from: "unknown_transfer".to_string(),
                    to: "accepted".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer ID mismatch".to_string()),
                        ..Default::default()
                    }
                })
            }
        } else {
            Err(Error::InvalidSessionStateTransition {
                from: "no_transfer".to_string(),
                to: "accepted".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No transfer in progress".to_string()),
                    ..Default::default()
                }
            })
        }
    }
    
    /// Get current transfer context
    pub async fn current_transfer(&self) -> Option<TransferContext> {
        self.transfer_context.lock().await.clone()
    }
    
    /// Get transfer history
    pub async fn transfer_history(&self) -> Vec<TransferContext> {
        self.transfer_history.lock().await.clone()
    }
    
    /// Check if transfer is in progress
    pub async fn has_transfer_in_progress(&self) -> bool {
        self.transfer_context.lock().await.is_some()
    }
    
    /// Set consultation session for attended transfer
    pub async fn set_consultation_session(&self, consultation_session_id: Option<SessionId>) {
        let mut guard = self.consultation_session_id.lock().await;
        *guard = consultation_session_id;
    }
    
    /// Get consultation session ID
    pub async fn consultation_session_id(&self) -> Option<SessionId> {
        self.consultation_session_id.lock().await.clone()
    }
    
    /// Update transfer progress (NOTIFY handling)
    pub async fn update_transfer_progress(&self, transfer_id: &TransferId, status: String) -> Result<(), Error> {
        let transfer_guard = self.transfer_context.lock().await;
        
        if let Some(ref transfer_context) = transfer_guard.as_ref() {
            if transfer_context.id == *transfer_id {
                // Publish transfer progress event
                self.event_bus.publish(SessionEvent::TransferProgress {
                    session_id: self.id.clone(),
                    transfer_id: transfer_id.to_string(),
                    status: status.clone(),
                });
                
                debug!("Transfer {} progress for session {}: {}", transfer_id, self.id, status);
                Ok(())
            } else {
                Err(Error::InvalidSessionStateTransition {
                    from: "unknown_transfer".to_string(),
                    to: "progress".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("Transfer ID mismatch".to_string()),
                        ..Default::default()
                    }
                })
            }
        } else {
            Err(Error::InvalidSessionStateTransition {
                from: "no_transfer".to_string(),
                to: "progress".to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some("No transfer in progress".to_string()),
                    ..Default::default()
                }
            })
        }
    }
    
    /// Complete a transfer successfully
    pub async fn complete_transfer(&self, transfer_id: &TransferId, final_status: String) -> Result<(), Error> {
        let mut transfer_context = {
            let mut transfer_guard = self.transfer_context.lock().await;
            
            if let Some(mut transfer_context) = transfer_guard.take() {
                if transfer_context.id == *transfer_id {
                    transfer_context.state = TransferState::Confirmed;
                    transfer_context.completed_at = Some(SystemTime::now());
                    transfer_context
                } else {
                    return Err(Error::InvalidSessionStateTransition {
                        from: "unknown_transfer".to_string(),
                        to: "completed".to_string(),
                        context: ErrorContext {
                            category: ErrorCategory::Session,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(self.id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Transfer ID mismatch".to_string()),
                            ..Default::default()
                        }
                    });
                }
            } else {
                return Err(Error::InvalidSessionStateTransition {
                    from: "no_transfer".to_string(),
                    to: "completed".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("No transfer in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        };
        
        // Add to transfer history
        {
            let mut history = self.transfer_history.lock().await;
            history.push(transfer_context);
        }
        
        // Update session state back to connected or terminate if this was the transferor
        self.set_state(SessionState::Terminated).await?;
        
        // Publish transfer completed event
        self.event_bus.publish(SessionEvent::TransferCompleted {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            final_status: final_status.clone(),
        });
        
        debug!("Completed transfer {} for session {} with status: {}", transfer_id, self.id, final_status);
        
        Ok(())
    }
    
    /// Fail a transfer
    pub async fn fail_transfer(&self, transfer_id: &TransferId, reason: String) -> Result<(), Error> {
        let mut transfer_context = {
            let mut transfer_guard = self.transfer_context.lock().await;
            
            if let Some(mut transfer_context) = transfer_guard.take() {
                if transfer_context.id == *transfer_id {
                    transfer_context.state = TransferState::Failed(reason.clone());
                    transfer_context.completed_at = Some(SystemTime::now());
                    transfer_context
                } else {
                    return Err(Error::InvalidSessionStateTransition {
                        from: "unknown_transfer".to_string(),
                        to: "failed".to_string(),
                        context: ErrorContext {
                            category: ErrorCategory::Session,
                            severity: ErrorSeverity::Error,
                            recovery: RecoveryAction::None,
                            retryable: false,
                            session_id: Some(self.id.to_string()),
                            timestamp: SystemTime::now(),
                            details: Some("Transfer ID mismatch".to_string()),
                            ..Default::default()
                        }
                    });
                }
            } else {
                return Err(Error::InvalidSessionStateTransition {
                    from: "no_transfer".to_string(),
                    to: "failed".to_string(),
                    context: ErrorContext {
                        category: ErrorCategory::Session,
                        severity: ErrorSeverity::Error,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        session_id: Some(self.id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some("No transfer in progress".to_string()),
                        ..Default::default()
                    }
                });
            }
        };
        
        // Add to transfer history
        {
            let mut history = self.transfer_history.lock().await;
            history.push(transfer_context);
        }
        
        // Update session state back to connected
        self.set_state(SessionState::Connected).await?;
        
        // Publish transfer failed event
        self.event_bus.publish(SessionEvent::TransferFailed {
            session_id: self.id.clone(),
            transfer_id: transfer_id.to_string(),
            reason: reason.clone(),
        });
        
        error!("Transfer {} failed for session {}: {}", transfer_id, self.id, reason);
        
        Ok(())
    }
} 